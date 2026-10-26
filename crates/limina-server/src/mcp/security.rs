//! The transport boundary in front of the MCP endpoint: bearer authentication, then the DNS
//! rebinding protection of the Python SDK's `TransportSecuritySettings`.
//!
//! Checks run in the Python server's order, and fail with its statuses:
//!
//! 1. **Authentication** through [`Auth::transport`] (bearer tokens only, behind the global
//!    failure ceiling). Failures are the API's JSON errors: 401 with `WWW-Authenticate: Bearer`,
//!    403 for identities the provider refuses, 429 with `Retry-After` once the ceiling is hit.
//! 2. **`Content-Type`** of a `POST` must start with `application/json` (400 `Invalid
//!    Content-Type header`).
//! 3. **`Host`** must be allowed (421 `Invalid Host header`).
//! 4. **`Origin`**, when present, must be allowed (403 `Invalid Origin header`).
//!
//! Allow-list entries match exactly, and an entry ending in `:*` matches its base followed by
//! `:` and anything (any port): `localhost:*` admits `localhost:7433` but not `localhost`.

use std::collections::BTreeMap;

use axum::extract::{Request, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE, HOST, HeaderValue, ORIGIN, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use limina_core::{ErrorKind, Principal};

use crate::auth::request::{actor_hint, bearer_token};
use crate::auth::{Auth, Authenticator, error_response};

/// `LIMINA_MCP_ALLOWED_HOSTS` when unset: loopback on any port.
pub const DEFAULT_ALLOWED_HOSTS: &str = "127.0.0.1:*,localhost:*,[::1]:*";
/// `LIMINA_MCP_ALLOWED_ORIGINS` when unset: loopback pages over HTTP on any port.
pub const DEFAULT_ALLOWED_ORIGINS: &str = "http://127.0.0.1:*,http://localhost:*,http://[::1]:*";

/// Which `Host` and `Origin` values the MCP endpoint accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOptions {
    /// `Host` header values (`example.com:443`, or `example.com:*` for any port).
    pub allowed_hosts: Vec<String>,
    /// `Origin` header values (`https://app.example.com`, or `http://localhost:*`).
    pub allowed_origins: Vec<String>,
}

impl Default for McpOptions {
    fn default() -> Self {
        Self {
            allowed_hosts: split_list(DEFAULT_ALLOWED_HOSTS),
            allowed_origins: split_list(DEFAULT_ALLOWED_ORIGINS),
        }
    }
}

impl McpOptions {
    /// `LIMINA_MCP_ALLOWED_HOSTS` and `LIMINA_MCP_ALLOWED_ORIGINS` from the process environment.
    pub fn from_env() -> Self {
        Self::from_map(&limina_domain::environment::process_environment())
    }

    /// The options from an explicit environment map: comma-separated lists, entries trimmed and
    /// empty entries dropped. An unset (or blank) variable keeps the loopback default.
    pub fn from_map(env: &BTreeMap<String, String>) -> Self {
        let list = |name: &str, default: &str| {
            let value = env.get(name).map(|value| value.trim()).filter(|value| !value.is_empty());
            split_list(value.unwrap_or(default))
        };
        Self {
            allowed_hosts: list("LIMINA_MCP_ALLOWED_HOSTS", DEFAULT_ALLOWED_HOSTS),
            allowed_origins: list("LIMINA_MCP_ALLOWED_ORIGINS", DEFAULT_ALLOWED_ORIGINS),
        }
    }

    /// `TransportSecurityMiddleware._validate_host`: a missing `Host` is never allowed.
    pub fn host_allowed(&self, host: Option<&str>) -> bool {
        host.filter(|host| !host.is_empty())
            .is_some_and(|host| matches_allow_list(host, &self.allowed_hosts))
    }

    /// `TransportSecurityMiddleware._validate_origin`: requests without an `Origin` (same-origin
    /// and non-browser clients) are allowed.
    pub fn origin_allowed(&self, origin: Option<&str>) -> bool {
        origin
            .filter(|origin| !origin.is_empty())
            .is_none_or(|origin| matches_allow_list(origin, &self.allowed_origins))
    }
}

fn split_list(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|item| !item.is_empty()).map(str::to_owned).collect()
}

fn matches_allow_list(value: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|entry| {
        entry == value
            || entry
                .strip_suffix(":*")
                .and_then(|base| value.strip_prefix(base))
                .is_some_and(|rest| rest.starts_with(':'))
    })
}

/// The caller the guard authenticated, for the tool handlers.
#[derive(Debug, Clone)]
pub(crate) struct McpCaller(pub Principal);

#[derive(Clone)]
pub(crate) struct Guard {
    pub auth: Auth,
    pub options: McpOptions,
}

/// The middleware in front of the Streamable HTTP service (see the module docs).
pub(crate) async fn guard(
    State(guard): State<Guard>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let bearer = bearer_token(headers);
    let actor =
        actor_hint(headers).map(|actor| actor.trim().to_owned()).filter(|actor| !actor.is_empty());
    let principal =
        match guard.auth.transport().authenticate(bearer.as_deref(), actor.as_deref()).await {
            Ok(principal) => principal,
            Err(error) => {
                let mut response = error_response(&error);
                if error.kind == ErrorKind::Authentication {
                    response
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                }
                return response;
            }
        };
    let method = request.method().clone();
    // HTTP/2 carries the host in the `:authority` pseudo-header, which becomes the URI's.
    let authority = request.uri().authority().map(|authority| authority.as_str().to_owned());
    if let Some(rejection) =
        transport_rejection(&method, headers, authority.as_deref(), &guard.options)
    {
        return rejection;
    }
    accept_json_only_clients(&method, request.headers_mut());
    request.extensions_mut().insert(McpCaller(principal));
    next.run(request).await
}

/// `TransportSecurityMiddleware.validate_request`.
fn transport_rejection(
    method: &Method,
    headers: &HeaderMap,
    authority: Option<&str>,
    options: &McpOptions,
) -> Option<Response> {
    if method == Method::POST {
        let content_type = header_text(headers, CONTENT_TYPE.as_str());
        if !content_type.is_some_and(|value| value.to_lowercase().starts_with("application/json")) {
            return Some(plain(StatusCode::BAD_REQUEST, "Invalid Content-Type header"));
        }
    }
    let host = header_text(headers, HOST.as_str()).or_else(|| authority.map(str::to_owned));
    if !options.host_allowed(host.as_deref()) {
        tracing::warn!("MCP request refused: the Host header is not allowed");
        return Some(plain(StatusCode::MISDIRECTED_REQUEST, "Invalid Host header"));
    }
    if !options.origin_allowed(header_text(headers, ORIGIN.as_str()).as_deref()) {
        tracing::warn!("MCP request refused: the Origin header is not allowed");
        return Some(plain(StatusCode::FORBIDDEN, "Invalid Origin header"));
    }
    None
}

/// The Python server answered with JSON (`json_response=True`), so it only required clients to
/// accept `application/json`. The SDK's transport also wants `text/event-stream` listed; since
/// every response here is a single JSON message, such clients are told it is acceptable.
fn accept_json_only_clients(method: &Method, headers: &mut HeaderMap) {
    if method != Method::POST {
        return;
    }
    let Some(accept) = header_text(headers, ACCEPT.as_str()) else {
        return;
    };
    let types: Vec<&str> = accept.split(',').map(str::trim).collect();
    let accepts = |prefix: &str| types.iter().any(|media_type| media_type.starts_with(prefix));
    if accepts("application/json")
        && !accepts("text/event-stream")
        && let Ok(value) = HeaderValue::from_str(&format!("{accept}, text/event-stream"))
    {
        headers.insert(ACCEPT, value);
    }
}

/// A header as text (Latin-1 bytes are kept as their characters, like Starlette).
fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).map(|value| value.as_bytes().iter().map(|byte| char::from(*byte)).collect())
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    (status, message).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect()
    }

    #[test]
    fn defaults_are_loopback_on_any_port() {
        let options = McpOptions::from_map(&BTreeMap::new());
        assert_eq!(options, McpOptions::default());
        assert_eq!(options.allowed_hosts, ["127.0.0.1:*", "localhost:*", "[::1]:*"]);
        assert!(options.host_allowed(Some("localhost:7433")));
        assert!(options.host_allowed(Some("127.0.0.1:1")));
        assert!(options.host_allowed(Some("[::1]:7433")));
        // Wildcard entries require a port, as in the Python SDK.
        assert!(!options.host_allowed(Some("localhost")));
        assert!(!options.host_allowed(Some("localhost.attacker.invalid:80")));
        assert!(!options.host_allowed(Some("attacker.invalid")));
        assert!(!options.host_allowed(None));
        assert!(options.origin_allowed(None));
        assert!(options.origin_allowed(Some("http://localhost:5173")));
        assert!(!options.origin_allowed(Some("https://localhost:5173")));
        assert!(!options.origin_allowed(Some("http://evil.example")));
    }

    #[test]
    fn the_environment_replaces_the_lists() {
        let options = McpOptions::from_map(&map(&[
            ("LIMINA_MCP_ALLOWED_HOSTS", " limina.example.com , ,limina.example.com:* "),
            ("LIMINA_MCP_ALLOWED_ORIGINS", "https://console.example.com"),
        ]));
        assert_eq!(options.allowed_hosts, ["limina.example.com", "limina.example.com:*"]);
        assert!(options.host_allowed(Some("limina.example.com")));
        assert!(options.host_allowed(Some("limina.example.com:8443")));
        assert!(!options.host_allowed(Some("localhost:7433")));
        assert!(options.origin_allowed(Some("https://console.example.com")));
        assert!(!options.origin_allowed(Some("https://console.example.com:444")));
        let blank = McpOptions::from_map(&map(&[("LIMINA_MCP_ALLOWED_HOSTS", "  ")]));
        assert_eq!(blank, McpOptions::default());
        let nothing = McpOptions::from_map(&map(&[("LIMINA_MCP_ALLOWED_HOSTS", ",")]));
        assert!(nothing.allowed_hosts.is_empty());
        assert!(!nothing.host_allowed(Some("localhost:7433")));
    }

    #[test]
    fn json_only_clients_are_accepted() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        accept_json_only_clients(&Method::POST, &mut headers);
        assert_eq!(headers[ACCEPT], "application/json, text/event-stream");
        let mut both = HeaderMap::new();
        both.insert(ACCEPT, HeaderValue::from_static("application/json, text/event-stream"));
        accept_json_only_clients(&Method::POST, &mut both);
        assert_eq!(both[ACCEPT], "application/json, text/event-stream");
    }
}
