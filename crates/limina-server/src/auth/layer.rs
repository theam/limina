//! Request authentication for axum routes: a layer plus extractors.
//!
//! [`AuthLayer`] (from [`Auth::layer`]) wraps the whole router. For each request it installs a
//! [`RequestAuth`] context in the request extensions and, once the handler has answered, appends
//! any `Set-Cookie` the authentication produced (a rotated WorkOS session) to the response.
//! Authentication itself is lazy: it runs the first time a [`Caller`] or [`AdminCaller`] is
//! extracted, at most once per request, so public routes under the same layer never authenticate
//! and never count as failed attempts.
//!
//! To require a caller for a whole router instead of per handler, add
//! `.route_layer(axum::middleware::from_extractor::<Caller>())` under the [`AuthLayer`]; handlers
//! can still extract `Caller` (the outcome is cached).

use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use axum::extract::{FromRequestParts, Request};
use axum::http::header::{HeaderMap, HeaderValue, SET_COOKIE};
use axum::http::request::Parts;
use axum::response::Response;
use futures::future::BoxFuture;
use limina_core::{LiminaError, Principal, Result, internal};
use tower::{Layer, Service};

use super::Auth;
use super::response::AuthRejection;

/// Installs a [`RequestAuth`] per request and writes the cookies it collects.
#[derive(Clone)]
pub struct AuthLayer {
    auth: Auth,
}

impl AuthLayer {
    pub(crate) fn new(auth: Auth) -> Self {
        Self { auth }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService { auth: self.auth.clone(), inner }
    }
}

#[derive(Clone)]
pub struct AuthService<S> {
    auth: Auth,
    inner: S,
}

impl<S> Service<Request> for AuthService<S>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, std::result::Result<Response, S::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request) -> Self::Future {
        let context = RequestAuth::new(self.auth.clone());
        request.extensions_mut().insert(context.clone());
        // Call the service that was polled ready; leave a fresh clone in its place.
        let fresh = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, fresh);
        Box::pin(async move {
            let mut response = inner.call(request).await?;
            context.write_cookies(response.headers_mut());
            Ok(response)
        })
    }
}

/// The authentication state of one request.
#[derive(Clone)]
pub struct RequestAuth(Arc<RequestAuthInner>);

struct RequestAuthInner {
    auth: Auth,
    outcome: tokio::sync::OnceCell<Result<Principal>>,
    cookies: Mutex<Vec<HeaderValue>>,
}

impl RequestAuth {
    fn new(auth: Auth) -> Self {
        Self(Arc::new(RequestAuthInner {
            auth,
            outcome: tokio::sync::OnceCell::new(),
            cookies: Mutex::new(Vec::new()),
        }))
    }

    pub fn auth(&self) -> &Auth {
        &self.0.auth
    }

    /// Authenticates the request on first use ([`Auth::authenticate`]); later calls return the
    /// same outcome. A rotated session cookie is queued for the response.
    pub async fn principal(&self, parts: &Parts) -> Result<Principal> {
        self.0
            .outcome
            .get_or_init(|| async {
                let credentials = self.0.auth.credentials(parts);
                let authenticated = self.0.auth.authenticate(&credentials).await?;
                if let Some(cookie) = authenticated.set_cookie {
                    self.0.cookies.lock().unwrap_or_else(PoisonError::into_inner).push(cookie);
                }
                Ok(authenticated.principal)
            })
            .await
            .clone()
    }

    fn write_cookies(&self, headers: &mut HeaderMap) {
        let cookies =
            std::mem::take(&mut *self.0.cookies.lock().unwrap_or_else(PoisonError::into_inner));
        for cookie in cookies {
            headers.append(SET_COOKIE, cookie);
        }
    }
}

fn context(parts: &Parts) -> std::result::Result<RequestAuth, AuthRejection> {
    parts
        .extensions
        .get::<RequestAuth>()
        .cloned()
        .ok_or_else(|| AuthRejection(internal("the authentication layer is not installed")))
}

/// The authenticated caller: a bearer credential, or else the console's session cookie.
///
/// Rejects with 401 (`authentication_required`), 403 (`permission_denied`: identity-provider
/// policy, or a cross-site write with the session cookie), 429 (`rate_limited`, with
/// `Retry-After`) or 422 (`invalid_request`: an `X-Limina-Actor` over 200 characters).
#[derive(Debug, Clone)]
pub struct Caller(pub Principal);

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        Ok(Self(context(parts)?.principal(parts).await?))
    }
}

/// A [`Caller`] who is an instance administrator; others get 403.
#[derive(Debug, Clone)]
pub struct AdminCaller(pub Principal);

impl<S: Send + Sync> FromRequestParts<S> for AdminCaller {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let principal = context(parts)?.principal(parts).await?;
        if !principal.instance_admin {
            return Err(AuthRejection(LiminaError::authorization_with(
                "Instance administrator access is required.",
            )));
        }
        Ok(Self(principal))
    }
}
