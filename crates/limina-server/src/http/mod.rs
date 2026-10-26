//! The HTTP API: the REST routes of the Python runtime (`api.py`, `runtime_api.py`,
//! `engine_account_api.py`, `internal_api.py`), the live WebSocket, the OpenAPI document and the
//! web console (`web_ui.py`), with the same paths, parameters, status codes, headers and bodies.
//!
//! ```text
//! /livez /readyz /healthz /v1/me /v1/runtime/…          probes, identity, engines   routes::runtime
//! /v1/projects…                                          projects and their surfaces routes::{projects, knowledge, resources, accounts, upload}
//! /v1/projects/{slug}/live                               live WebSocket              live
//! /internal/v1/projects/{slug}/…                         agent research protocol     routes::internal
//! /openapi.json /docs /redoc                             API description             openapi
//! /auth/…                                                console sign-in             crate::auth
//! everything else                                        the console, or JSON 404    web
//! ```
//!
//! # Requests
//!
//! Handlers read their inputs in FastAPI's order ([`call`]): a JSON body's syntax, the
//! `Idempotency-Key` header, the caller ([`crate::auth`]), query parameters, then body fields.
//! Validation errors are FastAPI's 422 `invalid_request` with pydantic-style entries in
//! `details.errors` ([`validate`]); every [`limina_core::LiminaError`] is the stable
//! `{"error": {"code", "message", "details"}}` with its status (`Retry-After` on 429). Unknown
//! paths and methods answer FastAPI's `{"detail": "Not Found"}` / `{"detail": "Method Not
//! Allowed"}`.
//!
//! # Limits
//!
//! JSON bodies are capped at [`HttpSettings::json_body_limit_bytes`] and uploads at
//! `upload_limit_bytes` (see [`routes::upload`]); larger bodies are refused with 413
//! `payload_too_large`. Live sessions are bounded as described in [`live`].
//!
//! # Deliberate differences from the Python API
//!
//! - **Body limits.** Uvicorn accepted bodies of any size; here JSON bodies over 4 MiB and
//!   multipart bodies over the upload limit plus 1 MiB are a 413 `payload_too_large`.
//! - **Integers beyond 64 bits** in query or body fields are a 422 `int_parsing_size`; Python
//!   accepted them and failed in the database (500).
//! - **JSON syntax errors** keep FastAPI's entry (`json_invalid`, the character position), but
//!   `ctx.error` names the problem in approximations of Python's `json` wording.
//! - **`HEAD`** is answered for every `GET` route (FastAPI answered 405), and a 405's `Allow` lists
//!   every method of the path (Starlette listed the first matching route's).
//! - **No trailing-slash redirects**: `/v1/projects/` is a JSON 404 instead of a 307.
//! - **Static segments that shadow a parameter**: FastAPI fell through from `…/claude-code`,
//!   `…/knowledge/relations` and `…/sources/upload` to the `{engine}`, `{artifact_id}` and
//!   `{source_id}` routes for the methods the static route lacks; those three are routed the same
//!   way here. Deeper combinations (`GET …/knowledge/relations/comments`) answer 405.
//! - **Uploads** authenticate (and check the editor role) before the form is read, so nothing is
//!   stored for a caller who may not upload; FastAPI parsed the whole form first.
//! - **Live refusals** (4400/4401/4403) are close frames sent right after the handshake: uvicorn
//!   turned a close before accepting into an HTTP 403, so real clients never saw the codes.
//!   Messages that are not JSON objects end the session with an `invalid_request` error frame
//!   (Python's handler crashed, 1011); binary frames are read as text.
//! - **Live delivery** is pushed through the event notifier with a 1 s poll for other processes
//!   (Python polled every 250 ms); idle sessions are pinged every 20 s and closed after 20 s
//!   without an answer (uvicorn's defaults); shutdown closes sessions with 1001.
//! - **The console** is served only from a configured directory (`LIMINA_WEB_DIR`, `--web-dir`);
//!   there is no packaged build to fall back to. `/docs/oauth2-redirect` is not served.

pub mod call;
pub mod error;
pub mod live;
pub mod openapi;
pub mod requests;
pub mod routes;
pub mod schemas;
pub mod trace;
pub mod validate;
pub mod web;

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method};
use axum::routing::{any, get};
use tokio::sync::watch;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::app::{DEFAULT_UPLOAD_LIMIT_BYTES, Runtime};
use crate::auth::Auth;
pub use live::LiveSettings;

/// The default cap of a JSON request body.
pub const DEFAULT_JSON_BODY_LIMIT_BYTES: usize = 4 * 1024 * 1024;

/// What the HTTP layer is configured with.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpSettings {
    /// The console build, already resolved (see [`web::resolve_web_dir`]); `None` serves the API
    /// only.
    pub web_dir: Option<PathBuf>,
    /// Origins allowed to call the API from a browser (`LIMINA_CORS_ORIGINS`; `*` for any). No
    /// CORS headers without any.
    pub cors_origins: Vec<String>,
    /// The largest uploaded source (`LIMINA_UPLOAD_LIMIT_BYTES`).
    pub upload_limit_bytes: u64,
    /// The largest JSON request body.
    pub json_body_limit_bytes: usize,
    pub live: LiveSettings,
}

impl Default for HttpSettings {
    fn default() -> Self {
        Self {
            web_dir: None,
            cors_origins: Vec::new(),
            upload_limit_bytes: DEFAULT_UPLOAD_LIMIT_BYTES,
            json_body_limit_bytes: DEFAULT_JSON_BODY_LIMIT_BYTES,
            live: LiveSettings::default(),
        }
    }
}

/// What every handler shares. Cheap to clone.
#[derive(Clone)]
pub struct App(Arc<AppInner>);

pub struct AppInner {
    pub runtime: Runtime,
    pub auth: Auth,
    pub settings: HttpSettings,
    /// Turns `true` when the server stops; live sessions then close.
    pub shutdown: watch::Receiver<bool>,
}

impl Deref for App {
    type Target = AppInner;

    fn deref(&self) -> &AppInner {
        &self.0
    }
}

/// The complete HTTP application for `runtime`: [`routes()`] wrapped in [`layers()`]. `shutdown`
/// turning `true` closes live sessions.
pub fn router(
    runtime: Runtime,
    auth: Auth,
    settings: HttpSettings,
    shutdown: watch::Receiver<bool>,
) -> Router {
    let cors_origins = settings.cors_origins.clone();
    layers(routes(runtime, auth.clone(), settings, shutdown), &auth, &cors_origins)
}

/// Every route (API, OpenAPI, sign-in, console) without the shared layers, so other endpoints
/// (MCP) can be added before [`layers`] wraps them all.
pub fn routes(
    runtime: Runtime,
    auth: Auth,
    settings: HttpSettings,
    shutdown: watch::Receiver<bool>,
) -> Router {
    let upload_body_limit = usize::try_from(
        settings.upload_limit_bytes.saturating_add(routes::upload::FORM_OVERHEAD_BYTES),
    )
    .unwrap_or(usize::MAX);
    let console = web::Console::new(settings.web_dir.clone());
    let app = App(Arc::new(AppInner { runtime, auth: auth.clone(), settings, shutdown }));

    let (api, document) = utoipa_axum::router::OpenApiRouter::with_openapi(openapi::base())
        .merge(routes::public(upload_body_limit))
        .split_for_parts();
    let document = openapi::Document::new(&document);

    api.merge(routes::internal())
        .route("/v1/projects/{slug}/live", any(live::live))
        .route("/auth/mcp/callback", get(routes::connections::callback))
        .with_state(app)
        .route("/openapi.json", get(move || std::future::ready(document.clone())))
        .route("/docs", get(openapi::swagger_ui))
        .route("/redoc", get(openapi::redoc))
        .merge(auth.router())
        .merge(
            Router::new()
                .route("/assets/{*path}", get(web::asset))
                .fallback(web::fallback)
                .with_state(console),
        )
}

/// The layers every request passes, innermost first: FastAPI's 405 body, the authentication
/// context (and rotated session cookies), CORS when origins are configured, and the request log.
pub fn layers(routes: Router, auth: &Auth, cors_origins: &[String]) -> Router {
    let routes = routes
        .layer(axum::middleware::map_response(error::method_not_allowed_body))
        .layer(auth.layer());
    let routes = match cors_layer(cors_origins) {
        Some(cors) => routes.layer(cors),
        None => routes,
    };
    routes.layer(axum::middleware::from_fn(trace::request))
}

/// CORS like Starlette's `CORSMiddleware` as the Python runtime configured it: the listed
/// origins, no credentials, the API's methods and headers.
fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }
    let allow_origin = if origins.iter().any(|origin| origin == "*") {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(
            origins
                .iter()
                .filter_map(|origin| HeaderValue::from_str(origin).ok())
                .collect::<Vec<_>>(),
        )
    };
    let headers = [
        "authorization",
        "content-type",
        "idempotency-key",
        "accept",
        "accept-language",
        "content-language",
    ]
    .map(HeaderName::from_static);
    Some(
        CorsLayer::new()
            .allow_origin(allow_origin)
            .allow_credentials(false)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE])
            .allow_headers(headers)
            .max_age(std::time::Duration::from_secs(600)),
    )
}
