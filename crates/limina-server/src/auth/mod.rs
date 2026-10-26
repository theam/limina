//! Who is calling: tokens, OIDC, WorkOS sessions and failed-attempt rate limits.
//!
//! A port of the Python runtime's `auth.py`, `rate_limit.py`, `workos_auth.py` and `web_auth.py`
//! with the same HTTP contract. [`Auth`] is the configured stack the API uses for every request.
//!
//! # Modes
//!
//! | Mode | Configured with | Credentials | `Principal::auth_mode` |
//! |---|---|---|---|
//! | open | `LIMINA_ALLOW_INSECURE_NO_AUTH=1`, no tokens | none: everyone is the local admin | `local` |
//! | token | `LIMINA_API_TOKEN` / `LIMINA_ADMIN_API_TOKEN` | static bearer tokens | `local` |
//! | oidc | `LIMINA_OIDC_ISSUER` + `LIMINA_OIDC_AUDIENCE` | provider JWTs (JWKS) | `oidc` |
//! | workos | `WORKOS_CLIENT_ID` (+ key, cookie password) | session cookie or WorkOS JWT; static tokens as machine credentials | `workos` / `local` |
//!
//! See [`config`] for every variable.
//!
//! # Per request
//!
//! [`Auth::authenticate`] mirrors the Python `require_auth` dependency:
//!
//! 1. An `X-Limina-Actor` longer than 200 characters is a 422 `invalid_request`.
//! 2. The per-client limiter refuses with 429 (and `Retry-After`) after 10 failures per minute.
//!    Its key is the peer IP, or the forwarded client when the peer is a trusted proxy ([`proxy`]).
//! 3. A bearer token goes to the authenticator through the global limiter (1000 failures per
//!    minute across REST, WebSocket and MCP). Without one, and with WorkOS, the `limina_session`
//!    cookie is resolved instead: writes must carry the console's `Origin` ([`origin`]), and an
//!    expired access token is refreshed and the cookie rotated.
//! 4. A 401 counts as a failure for the client; success clears its failures; a 403 does neither.
//!
//! # Wiring it into the API
//!
//! ```no_run
//! use axum::{Json, Router, routing::get};
//! use limina_server::auth::{AdminCaller, Auth, Caller};
//!
//! async fn me(Caller(principal): Caller) -> Json<serde_json::Value> {
//!     Json(serde_json::json!({ "subject": principal.subject, "auth_mode": principal.auth_mode }))
//! }
//!
//! async fn admin_only(AdminCaller(_admin): AdminCaller) -> &'static str {
//!     "ok"
//! }
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let auth = Auth::from_env().await?;
//! let app: Router = Router::new()
//!     .route("/livez", get(|| async { "ok" })) // never authenticates
//!     .route("/v1/me", get(me))
//!     .route("/v1/runtime/admin", get(admin_only))
//!     .merge(auth.router()) // /auth/config, /auth/login, /auth/callback, /auth/logout
//!     .layer(auth.layer()); // runs per request; writes rotated session cookies
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:7433").await?;
//! // Connect info gives the per-client limiter the peer address.
//! let service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
//! axum::serve(listener, service).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Routes that must not run without a caller can also be guarded as a group with
//! `.route_layer(axum::middleware::from_extractor::<Caller>())` (see [`layer`]). Handlers that need
//! the raw outcome (cookies included) call [`Auth::authenticate`] with [`Auth::credentials`].
//!
//! WebSocket and MCP authenticate bearer tokens only: call [`Auth::transport`] (the globally
//! rate-limited authenticator), and for the WebSocket also [`Auth::client_failures`] keyed by
//! [`RequestCredentials::client_key`], as the Python runtime does. `/healthz` reports
//! [`Auth::mode`]; errors become responses with [`error_response`].

pub mod authenticator;
pub mod config;
pub mod cookies;
pub mod directory;
pub mod jwks;
pub mod layer;
pub mod oidc;
pub mod origin;
pub mod proxy;
pub mod rate_limit;
pub mod request;
pub mod response;
pub mod seal;
pub mod tokens;
pub mod web;
pub mod workos;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::HeaderValue;
use limina_core::{ErrorKind, LiminaError, Principal, Result};
use serde_json::json;

use crate::app::PeopleDirectory;

pub use authenticator::{Authenticator, ChainedAuthenticator, RateLimitedAuthenticator};
pub use config::{AuthConfig, AuthConfigError, AuthLimits, AuthOptions};
pub use directory::WorkOsDirectory;
pub use jwks::{Jwks, JwksPolicy, TokenError};
pub use layer::{AdminCaller, AuthLayer, Caller, RequestAuth};
pub use oidc::{OidcAuthenticator, OidcSettings};
pub use proxy::TrustedProxies;
pub use rate_limit::{FailureRateLimiter, RateLimitSettings};
pub use request::{ACTOR_HEADER, ACTOR_LIMIT, RequestCredentials};
pub use response::{AuthRejection, error_response};
pub use seal::SessionSealer;
pub use tokens::{LocalAuthenticator, machine_token_authenticator};
pub use web::{BrowserSessions, ConsoleMode};
pub use workos::{ResolvedSession, WorkOsAuth, WorkOsSettings, safe_return_path};

/// A successfully authenticated request.
#[derive(Debug, Clone)]
pub struct Authenticated {
    pub principal: Principal,
    /// A `Set-Cookie` value the response must carry (a rotated WorkOS session).
    pub set_cookie: Option<HeaderValue>,
}

/// The configured authentication stack. Cheap to clone.
#[derive(Clone)]
pub struct Auth(Arc<AuthInner>);

struct AuthInner {
    transport: RateLimitedAuthenticator,
    client_failures: FailureRateLimiter,
    sessions: Option<BrowserSessions>,
    /// Where owners look people up to share projects by email (WorkOS User Management).
    directory: Option<Arc<dyn PeopleDirectory>>,
    console_mode: ConsoleMode,
    trusted_proxies: TrustedProxies,
}

impl Auth {
    /// [`AuthConfig::from_env`], then [`Auth::from_config`].
    pub async fn from_env() -> std::result::Result<Self, AuthConfigError> {
        Self::from_config(AuthConfig::from_env()?).await
    }

    /// Builds the stack. OIDC discovery happens here, so an unreachable provider fails startup.
    pub async fn from_config(config: AuthConfig) -> std::result::Result<Self, AuthConfigError> {
        let http = http_client(config.http_timeout)?;
        let options =
            AuthOptions { limits: config.limits, trusted_proxies: config.trusted_proxies };
        if let Some(settings) = config.workos {
            if config.oidc.is_some() {
                return Err(AuthConfigError::new(
                    "Configure either WorkOS or LIMINA_OIDC_ISSUER, not both.",
                ));
            }
            let workos = Arc::new(WorkOsAuth::new(settings, http)?);
            return Self::with_workos(
                workos,
                config.api_token.as_deref(),
                config.admin_api_token.as_deref(),
                options,
            );
        }
        let base: Arc<dyn Authenticator> = match config.oidc {
            Some(settings) => Arc::new(OidcAuthenticator::connect(settings, &http).await?),
            None => {
                if config.api_token.is_none() && !config.allow_insecure_no_auth {
                    return Err(AuthConfigError::new(
                        "Configure LIMINA_API_TOKEN for local development or LIMINA_OIDC_ISSUER \
                         and LIMINA_OIDC_AUDIENCE for team deployment.",
                    ));
                }
                Arc::new(LocalAuthenticator::new(config.api_token, config.admin_api_token)?)
            }
        };
        Self::new(base, None, options)
    }

    /// People sign in with WorkOS; optional static tokens keep CLI, MCP and CI automation working.
    pub fn with_workos(
        workos: Arc<WorkOsAuth>,
        api_token: Option<&str>,
        admin_api_token: Option<&str>,
        options: AuthOptions,
    ) -> std::result::Result<Self, AuthConfigError> {
        let mut chain: Vec<Arc<dyn Authenticator>> = Vec::new();
        if let Some(machine) = machine_token_authenticator(api_token, admin_api_token)? {
            chain.push(Arc::new(machine));
        }
        chain.push(workos.clone());
        let base = Arc::new(ChainedAuthenticator::new("workos", chain)?);
        Self::new(base, Some(workos), options)
    }

    /// An explicit authenticator, with browser sessions when `workos` is given.
    pub fn new(
        base: Arc<dyn Authenticator>,
        workos: Option<Arc<WorkOsAuth>>,
        options: AuthOptions,
    ) -> std::result::Result<Self, AuthConfigError> {
        let AuthOptions { limits, trusted_proxies } = options;
        let console_mode = if workos.is_some() {
            ConsoleMode::Workos
        } else if base.mode() == "oidc" {
            ConsoleMode::Oidc
        } else if base.requires_credentials() {
            ConsoleMode::Token
        } else {
            ConsoleMode::Open
        };
        let directory =
            workos.as_ref().map(|workos| Arc::new(workos.directory()) as Arc<dyn PeopleDirectory>);
        Ok(Self(Arc::new(AuthInner {
            transport: RateLimitedAuthenticator::new(base, FailureRateLimiter::new(limits.global)?),
            client_failures: FailureRateLimiter::new(limits.client)?,
            sessions: workos.map(BrowserSessions::new),
            directory,
            console_mode,
            trusted_proxies,
        })))
    }

    /// The authenticator's mode (`local`, `oidc` or `workos`), reported by `/healthz`.
    pub fn mode(&self) -> &str {
        self.0.transport.mode()
    }

    /// What `/auth/config` reports.
    pub fn console_mode(&self) -> ConsoleMode {
        self.0.console_mode
    }

    /// The bearer-token authenticator behind the global failure ceiling, for WebSocket and MCP.
    pub fn transport(&self) -> &RateLimitedAuthenticator {
        &self.0.transport
    }

    /// The per-client failure limiter (keyed by peer address).
    pub fn client_failures(&self) -> &FailureRateLimiter {
        &self.0.client_failures
    }

    /// The proxies whose forwarding headers are believed.
    pub fn trusted_proxies(&self) -> &TrustedProxies {
        &self.0.trusted_proxies
    }

    /// The credentials of a request, resolving forwarding headers from trusted proxies.
    pub fn credentials(&self, parts: &axum::http::request::Parts) -> RequestCredentials {
        RequestCredentials::from_parts(parts, &self.0.trusted_proxies)
    }

    /// The console's cookie sessions, when WorkOS is configured.
    pub fn sessions(&self) -> Option<&BrowserSessions> {
        self.0.sessions.as_ref()
    }

    /// The people directory owners share projects through (WorkOS User Management), when the
    /// sign-in provider has one. OIDC and token modes share by email without a directory.
    pub fn directory(&self) -> Option<&dyn PeopleDirectory> {
        self.0.directory.as_deref()
    }

    /// Authenticates one REST request (see the module docs for the exact order of checks).
    pub async fn authenticate(&self, request: &RequestCredentials) -> Result<Authenticated> {
        if let Some(actor) = &request.actor_hint
            && actor.chars().count() > ACTOR_LIMIT
        {
            return Err(actor_too_long(actor));
        }
        let key = request.client_key();
        self.0.client_failures.check(&key)?;
        let result = self.session_or_bearer(request).await;
        match &result {
            Ok(_) => self.0.client_failures.success(&key),
            Err(error) if error.kind == ErrorKind::Authentication => {
                self.0.client_failures.failure(&key);
            }
            Err(_) => {}
        }
        result
    }

    /// Bearer credentials win (API, CLI, MCP); otherwise the console's session cookie.
    async fn session_or_bearer(&self, request: &RequestCredentials) -> Result<Authenticated> {
        let sessions = match &self.0.sessions {
            Some(sessions) if request.bearer_token.is_none() => sessions,
            _ => {
                let principal = self
                    .0
                    .transport
                    .authenticate(request.bearer_token.as_deref(), request.actor_hint.as_deref())
                    .await?;
                return Ok(Authenticated { principal, set_cookie: None });
            }
        };
        match sessions.principal(request).await? {
            Some((principal, set_cookie)) => Ok(Authenticated { principal, set_cookie }),
            None => Err(LiminaError::authentication()),
        }
    }

    /// The per-request layer for the API router (see [`layer`]).
    pub fn layer(&self) -> AuthLayer {
        AuthLayer::new(self.clone())
    }

    /// `/auth/config`, and with WorkOS `/auth/login`, `/auth/callback` and `/auth/logout`.
    pub fn router<S: Clone + Send + Sync + 'static>(&self) -> Router<S> {
        web::router(self.0.console_mode, self.0.sessions.clone(), self.0.trusted_proxies.clone())
    }
}

/// The HTTP client for identity-provider calls: a timeout on every call and no redirects.
pub fn http_client(timeout: Duration) -> std::result::Result<reqwest::Client, AuthConfigError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("limina/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| AuthConfigError::new(format!("Cannot build the HTTP client: {error}")))
}

/// The 422 the REST API returned for an over-long `X-Limina-Actor` header.
fn actor_too_long(actor: &str) -> LiminaError {
    LiminaError::new(ErrorKind::Invariant, "The request did not match the API contract.")
        .with_code("invalid_request")
        .with(
            "errors",
            json!([{
                "type": "string_too_long",
                "loc": ["header", ACTOR_HEADER],
                "msg": format!("String should have at most {ACTOR_LIMIT} characters"),
                "input": actor,
                "ctx": {"max_length": ACTOR_LIMIT},
            }]),
        )
}
