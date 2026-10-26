//! Browser sign-in for the web console: cookie sessions and the `/auth/*` routes.
//!
//! `GET /auth/config` tells the console how to sign people in. With WorkOS configured, the console
//! never handles credentials: `/auth/login` and `/auth/callback` run the AuthKit flow and leave a
//! sealed HttpOnly session cookie, which [`BrowserSessions::principal`] turns into a principal for
//! API calls. `POST /auth/logout` clears it and returns the WorkOS logout URL.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::State;
use axum::http::header::{HeaderValue, LOCATION, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use limina_core::{ErrorKind, LiminaError, Principal, Result};
use serde_json::json;
use url::form_urlencoded;

use super::cookies::{
    FLOW_COOKIE, FLOW_TTL_SECONDS, SESSION_COOKIE, SESSION_TTL_SECONDS, delete_cookie,
    request_cookie, set_cookie,
};
use super::jwks::py_str;
use super::origin::{origin_of, require_console_origin};
use super::proxy::TrustedProxies;
use super::request::RequestCredentials;
use super::response::error_response;
use super::workos::WorkOsAuth;

/// What `/auth/config` reports, so the console knows how to sign people in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleMode {
    /// Localhost without tokens: signed in directly.
    Open,
    /// Paste `LIMINA_API_TOKEN` or `LIMINA_ADMIN_API_TOKEN`.
    Token,
    /// Paste an access token from the identity provider.
    Oidc,
    /// The organization's sign-in through WorkOS AuthKit.
    Workos,
}

impl ConsoleMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Token => "token",
            Self::Oidc => "oidc",
            Self::Workos => "workos",
        }
    }
}

/// Cookie-backed sessions for the console, plus the rule that protects them from cross-site use.
#[derive(Clone)]
pub struct BrowserSessions {
    workos: Arc<WorkOsAuth>,
}

impl BrowserSessions {
    pub fn new(workos: Arc<WorkOsAuth>) -> Self {
        Self { workos }
    }

    pub fn workos(&self) -> &Arc<WorkOsAuth> {
        &self.workos
    }

    /// The console's origin: `LIMINA_PUBLIC_URL`'s, or the request's own.
    pub fn public_origin(&self, request_origin: &str) -> String {
        self.workos
            .settings()
            .public_url
            .as_deref()
            .map_or_else(|| request_origin.to_owned(), origin_of)
    }

    /// Cookies are `Secure` when the console is served over HTTPS.
    pub fn secure(&self, request_origin: &str) -> bool {
        self.public_origin(request_origin).starts_with("https://")
    }

    /// Resolves the session cookie; `None` when the request has none. A state-changing request
    /// must come from the console's origin. When the access token was refreshed, the second value
    /// is the rotated cookie the response must set.
    pub async fn principal(
        &self,
        request: &RequestCredentials,
    ) -> Result<Option<(Principal, Option<HeaderValue>)>> {
        let Some(cookie) = request.session_cookie.as_deref().filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let public_origin = self.public_origin(&request.request_origin);
        require_console_origin(&request.method, request.origin.as_deref(), &public_origin)?;
        let resolved = self.workos.resolve(cookie).await?;
        let rotated = resolved
            .refreshed_cookie
            .map(|value| self.session_cookie(&value, &request.request_origin))
            .transpose()?;
        Ok(Some((resolved.principal, rotated)))
    }

    /// The `Set-Cookie` value that stores a sealed session.
    pub fn session_cookie(&self, value: &str, request_origin: &str) -> Result<HeaderValue> {
        set_cookie(SESSION_COOKIE, value, SESSION_TTL_SECONDS, "/", self.secure(request_origin))
    }
}

#[derive(Clone)]
struct WebAuthState {
    mode: ConsoleMode,
    sessions: Option<BrowserSessions>,
    proxies: TrustedProxies,
}

/// `/auth/config` always; `/auth/login`, `/auth/callback` and `/auth/logout` with WorkOS.
pub(crate) fn router<S: Clone + Send + Sync + 'static>(
    mode: ConsoleMode,
    sessions: Option<BrowserSessions>,
    proxies: TrustedProxies,
) -> Router<S> {
    let routes = Router::new().route("/auth/config", get(config));
    let routes = if sessions.is_some() {
        routes
            .route("/auth/login", get(login))
            .route("/auth/callback", get(callback))
            .route("/auth/logout", post(logout))
    } else {
        routes
    };
    routes.with_state(WebAuthState { mode, sessions, proxies })
}

async fn config(State(state): State<WebAuthState>) -> Json<serde_json::Value> {
    Json(json!({"mode": state.mode.as_str()}))
}

async fn login(State(state): State<WebAuthState>, parts: Parts) -> Response {
    let Some(sessions) = state.sessions else { return StatusCode::NOT_FOUND.into_response() };
    let request = RequestCredentials::from_parts(&parts, &state.proxies);
    let return_to = query_param(&parts.uri, "return_to").filter(|value| !value.is_empty());
    let canonical = sessions.public_origin(&request.request_origin);
    if canonical != request.request_origin {
        // Cookies are per host: start (and finish) the flow on the public URL.
        let query = return_to
            .map(|value| {
                let encoded = form_urlencoded::Serializer::new(String::new())
                    .append_pair("return_to", &value)
                    .finish();
                format!("?{encoded}")
            })
            .unwrap_or_default();
        return redirect(&format!("{canonical}/auth/login{query}"));
    }
    let started =
        sessions.workos.start_login(&canonical, return_to.as_deref()).and_then(|(url, flow)| {
            let cookie = set_cookie(
                FLOW_COOKIE,
                &flow,
                FLOW_TTL_SECONDS,
                "/auth",
                sessions.secure(&canonical),
            )?;
            Ok((url, cookie))
        });
    match started {
        Ok((url, cookie)) => with_cookies(redirect(&url), [cookie]),
        Err(error) => error_response(&error),
    }
}

async fn callback(State(state): State<WebAuthState>, parts: Parts) -> Response {
    let Some(sessions) = state.sessions else { return StatusCode::NOT_FOUND.into_response() };
    let code = query_param(&parts.uri, "code").filter(|value| !value.is_empty());
    let flow_state = query_param(&parts.uri, "state").filter(|value| !value.is_empty());
    let error = query_param(&parts.uri, "error").filter(|value| !value.is_empty());
    let mut cookies = Vec::new();
    let response = match (code, flow_state, error) {
        (Some(code), Some(flow_state), None) => {
            let request = RequestCredentials::from_parts(&parts, &state.proxies);
            let flow_cookie = request_cookie(&parts.headers, FLOW_COOKIE);
            let finished = sessions.workos.finish_login(&code, &flow_state, flow_cookie.as_deref());
            match finished.await.and_then(|(session, return_to)| {
                Ok((sessions.session_cookie(&session, &request.request_origin)?, return_to))
            }) {
                Ok((cookie, return_to)) => {
                    cookies.push(cookie);
                    redirect(&return_to)
                }
                Err(error) if error.kind == ErrorKind::Authorization => {
                    signin(&reason(&error).unwrap_or_else(|| "denied".to_owned()))
                }
                Err(error) => {
                    tracing::warn!(message = %error.message, "WorkOS sign-in failed");
                    signin(&reason(&error).unwrap_or_else(|| "failed".to_owned()))
                }
            }
        }
        (_, _, error) => {
            // Provider detail stays in the server log; the console shows a generic message.
            let detail = error.as_deref().unwrap_or("missing code");
            tracing::warn!(error = %detail, "WorkOS sign-in returned an error");
            signin(if error.as_deref() == Some("access_denied") { "cancelled" } else { "failed" })
        }
    };
    match delete_cookie(FLOW_COOKIE, "/auth") {
        Ok(expired) => cookies.push(expired),
        Err(error) => return error_response(&error),
    }
    with_cookies(response, cookies)
}

async fn logout(State(state): State<WebAuthState>, parts: Parts) -> Response {
    let Some(sessions) = state.sessions else { return StatusCode::NOT_FOUND.into_response() };
    let request = RequestCredentials::from_parts(&parts, &state.proxies);
    let public_origin = sessions.public_origin(&request.request_origin);
    if request.origin.as_deref() != Some(public_origin.as_str()) {
        let error = LiminaError::authorization_with("Sign out must come from the console.")
            .with("reason", "origin");
        return error_response(&error);
    }
    let return_to = format!("{public_origin}/signin?signed_out=1");
    let target = sessions.workos.logout_url(request.session_cookie.as_deref(), &return_to);
    let body = Json(json!({"redirect": target.as_deref().unwrap_or("/signin?signed_out=1")}));
    match delete_cookie(SESSION_COOKIE, "/") {
        Ok(expired) => with_cookies(body.into_response(), [expired]),
        Err(error) => error_response(&error),
    }
}

fn reason(error: &LiminaError) -> Option<String> {
    error
        .details
        .get("reason")
        .filter(|value| !value.is_null())
        .map(py_str)
        .filter(|r| !r.is_empty())
}

/// The last value of a query parameter (percent-decoded).
fn query_param(uri: &Uri, name: &str) -> Option<String> {
    form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .last()
}

fn signin(reason: &str) -> Response {
    let query =
        form_urlencoded::Serializer::new(String::new()).append_pair("error", reason).finish();
    redirect(&format!("/signin?{query}"))
}

/// `303 See Other`, with the location percent-encoded the way Starlette's `RedirectResponse` does.
fn redirect(location: &str) -> Response {
    const SAFE: &[u8] = b"_.-~:/%#?=@[]!$&'()*+,;";
    let mut quoted = String::with_capacity(location.len());
    for byte in location.bytes() {
        if byte.is_ascii_alphanumeric() || SAFE.contains(&byte) {
            quoted.push(char::from(byte));
        } else {
            let _ = write!(quoted, "%{byte:02X}");
        }
    }
    let location = HeaderValue::try_from(quoted).unwrap_or_else(|_| HeaderValue::from_static("/"));
    (StatusCode::SEE_OTHER, [(LOCATION, location)]).into_response()
}

fn with_cookies(
    mut response: Response,
    cookies: impl IntoIterator<Item = HeaderValue>,
) -> Response {
    for cookie in cookies {
        response.headers_mut().append(SET_COOKIE, cookie);
    }
    response
}
