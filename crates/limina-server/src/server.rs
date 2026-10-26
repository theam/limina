//! Running a complete Limina instance: configuration, the application router and the server
//! loop (Python's `create_app` plus `uvicorn.run`).
//!
//! ```no_run
//! # async fn run() -> limina_core::Result<()> {
//! use limina_server::server::{ServerConfig, serve};
//!
//! let config = ServerConfig::from_env()?;
//! serve(config, async {
//!     let _ = tokio::signal::ctrl_c().await;
//! })
//! .await
//! # }
//! ```
//!
//! # Environment
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `LIMINA_WEB_DIR` | — | the console build to serve (a directory with `index.html`) |
//! | `LIMINA_CORS_ORIGINS` | — | comma-separated browser origins allowed to call the API (`*` for any) |
//! | `LIMINA_MCP_ALLOWED_HOSTS` | `127.0.0.1:*,localhost:*,[::1]:*` | `Host` values the MCP endpoint accepts |
//! | `LIMINA_MCP_ALLOWED_ORIGINS` | `http://127.0.0.1:*,http://localhost:*,http://[::1]:*` | `Origin` values the MCP endpoint accepts |
//! | `LIMINA_UPLOAD_LIMIT_BYTES` | `26214400` | the largest uploaded source |
//!
//! plus the runtime's ([`crate::app`]) and sign-in's ([`crate::auth::config`]) variables. The
//! host and port come from the caller (`limina serve --host --port`), `127.0.0.1:7433` by
//! default.

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Router;
use limina_core::{LiminaError, Result, internal};
use limina_domain::environment::process_environment;

use crate::app::{Runtime, RuntimeConfig};
use crate::auth::{Auth, AuthConfig};
use crate::http::{self, DEFAULT_JSON_BODY_LIMIT_BYTES, HttpSettings, LiveSettings};

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 7433;
pub const DEFAULT_MCP_ALLOWED_HOSTS: &str = "127.0.0.1:*,localhost:*,[::1]:*";
pub const DEFAULT_MCP_ALLOWED_ORIGINS: &str =
    "http://127.0.0.1:*,http://localhost:*,http://[::1]:*";

/// Everything a running instance needs.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// `LIMINA_WEB_DIR`: the console build; without one (or without its `index.html`) the
    /// runtime serves the API only.
    pub web_dir: Option<PathBuf>,
    /// `LIMINA_CORS_ORIGINS`.
    pub cors_origins: Vec<String>,
    /// `LIMINA_MCP_ALLOWED_HOSTS`, for the MCP endpoint's DNS-rebinding protection.
    pub mcp_allowed_hosts: Vec<String>,
    /// `LIMINA_MCP_ALLOWED_ORIGINS`.
    pub mcp_allowed_origins: Vec<String>,
    /// `LIMINA_UPLOAD_LIMIT_BYTES`; [`serve`] also applies it to [`RuntimeConfig::upload_limit_bytes`].
    pub upload_limit_bytes: u64,
    /// The largest JSON request body (4 MiB).
    pub json_body_limit_bytes: usize,
    /// Timing and size limits of live sessions.
    pub live: LiveSettings,
    pub runtime: RuntimeConfig,
    pub auth: AuthConfig,
}

impl ServerConfig {
    /// Defaults around an explicit runtime and sign-in configuration.
    pub fn new(runtime: RuntimeConfig, auth: AuthConfig) -> Self {
        Self {
            host: DEFAULT_HOST.to_owned(),
            port: DEFAULT_PORT,
            web_dir: None,
            cors_origins: Vec::new(),
            mcp_allowed_hosts: list(DEFAULT_MCP_ALLOWED_HOSTS),
            mcp_allowed_origins: list(DEFAULT_MCP_ALLOWED_ORIGINS),
            upload_limit_bytes: runtime.upload_limit_bytes,
            json_body_limit_bytes: DEFAULT_JSON_BODY_LIMIT_BYTES,
            live: LiveSettings::default(),
            runtime,
            auth,
        }
    }

    /// The configuration from the process environment.
    pub fn from_env() -> Result<Self> {
        let env = process_environment();
        let auth = AuthConfig::from_lookup(|name| env.get(name).cloned())
            .map_err(|error| LiminaError::generic(error.message()))?;
        Self::from_map(&env, auth)
    }

    /// The configuration from an explicit environment map and sign-in configuration.
    pub fn from_map(env: &BTreeMap<String, String>, auth: AuthConfig) -> Result<Self> {
        let value = |name: &str| env.get(name).map(|value| value.trim()).filter(|v| !v.is_empty());
        let mut config = Self::new(RuntimeConfig::from_map(env)?, auth);
        config.web_dir = value("LIMINA_WEB_DIR").map(PathBuf::from);
        config.cors_origins = value("LIMINA_CORS_ORIGINS").map(list).unwrap_or_default();
        if let Some(hosts) = value("LIMINA_MCP_ALLOWED_HOSTS") {
            config.mcp_allowed_hosts = list(hosts);
        }
        if let Some(origins) = value("LIMINA_MCP_ALLOWED_ORIGINS") {
            config.mcp_allowed_origins = list(origins);
        }
        Ok(config)
    }

    /// The HTTP layer's part of the configuration (the web directory as given, unresolved).
    pub fn http_settings(&self) -> HttpSettings {
        HttpSettings {
            web_dir: self.web_dir.clone(),
            cors_origins: self.cors_origins.clone(),
            upload_limit_bytes: self.upload_limit_bytes,
            json_body_limit_bytes: self.json_body_limit_bytes,
            live: self.live,
        }
    }
}

/// A comma-separated list, items trimmed, empty items dropped.
fn list(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|item| !item.is_empty()).map(str::to_owned).collect()
}

/// The application router for an existing runtime and sign-in stack (tests serve it with
/// `tower::ServiceExt::oneshot` or a listener). Serve it with
/// `into_make_service_with_connect_info::<SocketAddr>()` so rate limits see client addresses.
pub async fn build_app(runtime: Runtime, auth: Auth, config: &ServerConfig) -> Router {
    // Without `serve`, nothing ever signals a shutdown: live sessions end with their clients.
    let (_never, shutdown) = http::live::shutdown_signal();
    app_with_shutdown(runtime, auth, config, shutdown)
}

fn app_with_shutdown(
    runtime: Runtime,
    auth: Auth,
    config: &ServerConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Router {
    let mut settings = config.http_settings();
    settings.web_dir = http::web::resolve_web_dir(settings.web_dir.as_deref());
    // MCP sits before the shared layers so it gets the request log, CORS and the sign-in
    // context like every other route. `nest_service` serves both `/mcp` and `/mcp/`.
    let mcp = crate::mcp::router(
        &runtime,
        auth.clone(),
        crate::mcp::McpOptions {
            allowed_hosts: config.mcp_allowed_hosts.clone(),
            allowed_origins: config.mcp_allowed_origins.clone(),
        },
    );
    let routes = http::routes(runtime, auth.clone(), settings, shutdown).nest_service("/mcp", mcp);
    http::layers(routes, &auth, &config.cors_origins)
}

/// Runs the instance until `shutdown` resolves: connects and initializes the database, starts
/// the runtime (projects left running resume), serves HTTP with client addresses, then stops
/// accepting, closes live sessions, waits for in-flight requests and shuts the runtime down.
pub async fn serve(
    mut config: ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    config.auth.check_bind(&config.host).map_err(|error| LiminaError::generic(error.message()))?;
    config.runtime.upload_limit_bytes = config.upload_limit_bytes;
    let runtime = Runtime::new(config.runtime.clone()).await?;
    serve_runtime(runtime, config, shutdown).await
}

/// [`serve`] for a runtime the caller already built (for example to seed it first). The runtime
/// is started here and shut down when the server stops.
pub async fn serve_runtime(
    runtime: Runtime,
    config: ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    config.auth.check_bind(&config.host).map_err(|error| LiminaError::generic(error.message()))?;
    let auth = Auth::from_config(config.auth.clone())
        .await
        .map_err(|error| LiminaError::generic(error.message()))?;
    let listener = tokio::net::TcpListener::bind((config.host.as_str(), config.port))
        .await
        .map_err(|error| {
            LiminaError::transport(format!(
                "Cannot listen on {}:{}: {error}",
                config.host, config.port
            ))
        })?;
    let address = listener.local_addr().map_err(internal)?;
    runtime.start().await?;
    let (stopping, stopped) = http::live::shutdown_signal();
    let app = app_with_shutdown(runtime.clone(), auth, &config, stopped);
    tracing::info!(%address, auth = config.auth_mode_label(), "Limina is serving");
    let served = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            shutdown.await;
            tracing::info!("Limina is shutting down");
            let _ = stopping.send(true);
        })
        .await;
    runtime.shutdown().await;
    runtime.db().close().await;
    served.map_err(|error| LiminaError::transport(format!("The server failed: {error}")))
}

impl ServerConfig {
    fn auth_mode_label(&self) -> &'static str {
        if self.auth.workos.is_some() {
            "workos"
        } else if self.auth.oidc.is_some() {
            "oidc"
        } else if self.auth.api_token.is_some() || self.auth.admin_api_token.is_some() {
            "token"
        } else {
            "open"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn defaults_follow_the_python_runtime() {
        let config = ServerConfig::from_map(&BTreeMap::new(), AuthConfig::default()).unwrap();
        assert_eq!((config.host.as_str(), config.port), ("127.0.0.1", 7433));
        assert_eq!(config.web_dir, None);
        assert!(config.cors_origins.is_empty());
        assert_eq!(config.mcp_allowed_hosts, ["127.0.0.1:*", "localhost:*", "[::1]:*"]);
        assert_eq!(
            config.mcp_allowed_origins,
            ["http://127.0.0.1:*", "http://localhost:*", "http://[::1]:*"]
        );
        assert_eq!(config.upload_limit_bytes, 25 * 1024 * 1024);
    }

    #[test]
    fn the_environment_sets_every_field() {
        let config = ServerConfig::from_map(
            &map(&[
                ("LIMINA_WEB_DIR", "/app/web"),
                ("LIMINA_CORS_ORIGINS", " https://a.example , ,https://b.example"),
                ("LIMINA_MCP_ALLOWED_HOSTS", "limina.example:443"),
                ("LIMINA_MCP_ALLOWED_ORIGINS", "https://limina.example"),
                ("LIMINA_UPLOAD_LIMIT_BYTES", "1024"),
            ]),
            AuthConfig::default(),
        )
        .unwrap();
        assert_eq!(config.web_dir, Some(PathBuf::from("/app/web")));
        assert_eq!(config.cors_origins, ["https://a.example", "https://b.example"]);
        assert_eq!(config.mcp_allowed_hosts, ["limina.example:443"]);
        assert_eq!(config.mcp_allowed_origins, ["https://limina.example"]);
        assert_eq!(config.upload_limit_bytes, 1024);
        assert_eq!(config.runtime.upload_limit_bytes, 1024);
    }
}
