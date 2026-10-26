//! `limina serve`: a complete Limina instance ([`limina_server::server::serve`]).
//!
//! The options mirror the Python `serve` command and the environment its `create_app` read for
//! decisions made at startup; they override [`ServerConfig::from_env`]. Settings the server
//! components read on their own (sign-in details, CORS, MCP hosts, upload and rate limits, engine
//! settings) stay with the server crate. Logs follow `RUST_LOG` (`info` by default).

use std::path::{Path, PathBuf};

use clap::Args;
use limina_core::{LiminaError, Result};
use limina_server::http::web::resolve_web_dir;
use limina_server::server::{ServerConfig, serve};

/// Hosts that only accept connections from this machine.
const LOCAL_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "::1"];

#[derive(Debug, Clone, PartialEq, Args)]
pub struct ServeOptions {
    /// The runtime database (SQLite or PostgreSQL URL).
    #[arg(verbatim_doc_comment)]
    #[arg(
        long,
        env = "LIMINA_DATABASE_URL",
        hide_env_values = true,
        default_value = "sqlite:///.limina/server.db",
        value_name = "TEXT"
    )]
    pub database_url: String,

    /// Where managed projects get their working directories.
    #[arg(verbatim_doc_comment)]
    #[arg(
        long,
        env = "LIMINA_WORKSPACE_ROOT",
        hide_env_values = true,
        default_value = ".limina/workspaces",
        value_name = "PATH"
    )]
    pub workspace_root: PathBuf,

    #[arg(long, default_value = "127.0.0.1", value_name = "TEXT")]
    pub host: String,

    #[arg(long, default_value_t = 7433, value_name = "INTEGER")]
    pub port: u16,

    /// Shared access token for people and automation.
    #[arg(verbatim_doc_comment)]
    #[arg(long, env = "LIMINA_API_TOKEN", hide_env_values = true, value_name = "TEXT")]
    pub token: Option<String>,

    /// Shared instance-administrator token.
    #[arg(verbatim_doc_comment)]
    #[arg(long, env = "LIMINA_ADMIN_API_TOKEN", hide_env_values = true, value_name = "TEXT")]
    pub admin_token: Option<String>,

    /// The web console build to serve (a directory with `index.html`); API-only without one.
    #[arg(verbatim_doc_comment)]
    #[arg(long, env = "LIMINA_WEB_DIR", hide_env_values = true, value_name = "PATH")]
    pub web_dir: Option<PathBuf>,

    /// The key that encrypts project secrets [default: <workspace root>/../secret.key].
    #[arg(verbatim_doc_comment)]
    #[arg(long, env = "LIMINA_SECRET_KEY_PATH", hide_env_values = true, value_name = "PATH")]
    pub secret_key_path: Option<PathBuf>,

    /// OpenID Connect issuer for sign-in (with `LIMINA_OIDC_AUDIENCE`).
    #[arg(verbatim_doc_comment)]
    #[arg(long, env = "LIMINA_OIDC_ISSUER", hide_env_values = true, hide = true)]
    pub oidc_issuer: Option<String>,

    #[arg(long, env = "LIMINA_OIDC_AUDIENCE", hide_env_values = true, hide = true)]
    pub oidc_audience: Option<String>,

    /// WorkOS client for browser sign-in.
    #[arg(verbatim_doc_comment)]
    #[arg(long, env = "WORKOS_CLIENT_ID", hide_env_values = true, hide = true)]
    pub workos_client_id: Option<String>,

    /// Serve without any authentication (`1`, `true` or `yes`).
    #[arg(verbatim_doc_comment)]
    #[arg(
        long,
        env = "LIMINA_ALLOW_INSECURE_NO_AUTH", hide_env_values = true,
        hide = true,
        action = clap::ArgAction::Set,
        default_value = "",
        value_parser = parse_python_flag,
        num_args = 0..=1,
        default_missing_value = "true"
    )]
    pub allow_insecure_no_auth: bool,
}

/// The Python runtime's `value.lower() in {"1", "true", "yes"}`.
fn parse_python_flag(value: &str) -> std::result::Result<bool, String> {
    Ok(matches!(value.trim().to_lowercase().as_str(), "1" | "true" | "yes"))
}

impl ServeOptions {
    /// True when sign-in is configured by OIDC (issuer and audience).
    pub fn oidc_configured(&self) -> bool {
        self.oidc_issuer.as_deref().is_some_and(|v| !v.is_empty())
            && self.oidc_audience.as_deref().is_some_and(|v| !v.is_empty())
    }

    /// True when browser sign-in is configured by WorkOS.
    pub fn workos_configured(&self) -> bool {
        self.workos_client_id.as_deref().is_some_and(|v| !v.trim().is_empty())
    }

    /// Rejects a non-local bind that nobody would have to authenticate against. Returns the
    /// usage error message.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let has_token = self.token.as_deref().is_some_and(|v| !v.is_empty())
            || self.admin_token.as_deref().is_some_and(|v| !v.is_empty());
        if !LOCAL_HOSTS.contains(&self.host.as_str())
            && !has_token
            && !self.oidc_configured()
            && !self.workos_configured()
            && !self.allow_insecure_no_auth
        {
            return Err("a shared token, WorkOS, or OIDC configuration is required for a \
                        non-local bind"
                .to_owned());
        }
        Ok(())
    }

    /// The URL managed agents use to reach this instance's internal API.
    pub fn internal_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Where project secrets are encrypted: `LIMINA_SECRET_KEY_PATH`, or `secret.key` next to
    /// the workspace root.
    pub fn resolved_secret_key_path(&self) -> PathBuf {
        self.secret_key_path.clone().unwrap_or_else(|| {
            self.workspace_root.parent().unwrap_or_else(|| Path::new("")).join("secret.key")
        })
    }

    /// The startup notice when the web console is served from `web_dir`.
    pub fn web_ui_notice(&self, web_dir: &Path) -> String {
        let mut host = if matches!(self.host.as_str(), "0.0.0.0" | "::") {
            "127.0.0.1".to_owned()
        } else {
            self.host.clone()
        };
        if host.contains(':') {
            host = format!("[{host}]");
        }
        format!("Limina web UI: http://{host}:{}/ (serving {})", self.port, web_dir.display())
    }
}

/// The server configuration: the environment, with these options on top.
pub fn server_config(options: &ServeOptions) -> Result<ServerConfig> {
    let mut config = ServerConfig::from_env()?;
    config.host.clone_from(&options.host);
    config.port = options.port;
    config.runtime.database_url.clone_from(&options.database_url);
    config.runtime.workspace_root.clone_from(&options.workspace_root);
    config.runtime.secret_key_path = options.resolved_secret_key_path();
    config.runtime.internal_url = options.internal_url();
    let non_empty = |value: &Option<String>| value.clone().filter(|value| !value.is_empty());
    config.auth.api_token = non_empty(&options.token);
    config.auth.admin_api_token = non_empty(&options.admin_token);
    config.auth.allow_insecure_no_auth = options.allow_insecure_no_auth;
    config.web_dir = options.web_dir.clone();
    Ok(config)
}

/// Runs the instance until SIGINT or SIGTERM, on a multi-threaded runtime of its own (the
/// command line itself runs on a single thread).
pub async fn run(options: ServeOptions) -> Result<()> {
    let mut config = server_config(&options)?;
    config.web_dir = resolve_web_dir(config.web_dir.as_deref());
    if let Some(web_dir) = &config.web_dir {
        crate::output::error_line(&options.web_ui_notice(web_dir));
    }
    let serving = tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("limina-server")
            .build()
            .map_err(limina_core::internal)?;
        runtime.block_on(serve(config, shutdown_signal()))
    });
    serving.await.map_err(|error| LiminaError::generic(format!("The server stopped: {error}")))?
}

/// Resolves on SIGINT (Ctrl-C) or, on Unix, SIGTERM.
pub(crate) async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct Harness {
        #[command(flatten)]
        options: ServeOptions,
    }

    fn options(args: &[&str]) -> ServeOptions {
        let mut argv = vec!["serve"];
        argv.extend_from_slice(args);
        Harness::try_parse_from(argv).unwrap().options
    }

    #[test]
    fn non_local_binds_need_authentication() {
        // Explicit flags win over whatever the test environment exports.
        let open = options(&["--host", "0.0.0.0", "--allow-insecure-no-auth", "false"]);
        let open = ServeOptions {
            token: None,
            admin_token: None,
            oidc_issuer: None,
            oidc_audience: None,
            workos_client_id: None,
            ..open
        };
        assert!(open.validate().is_err());
        assert!(ServeOptions { host: "127.0.0.1".into(), ..open.clone() }.validate().is_ok());
        assert!(ServeOptions { token: Some("t".into()), ..open.clone() }.validate().is_ok());
        assert!(ServeOptions { allow_insecure_no_auth: true, ..open.clone() }.validate().is_ok());
        let oidc = ServeOptions {
            oidc_issuer: Some("https://issuer.example".into()),
            oidc_audience: Some("limina".into()),
            ..open.clone()
        };
        assert!(oidc.validate().is_ok());
        assert!(ServeOptions { workos_client_id: Some(" ".into()), ..open }.validate().is_err());
    }

    #[test]
    fn derived_settings_follow_the_python_defaults() {
        let options = options(&["--port", "8080", "--workspace-root", "/data/workspaces"]);
        assert_eq!(options.internal_url(), "http://127.0.0.1:8080");
        let options = ServeOptions { secret_key_path: None, ..options };
        assert_eq!(options.resolved_secret_key_path(), PathBuf::from("/data/secret.key"));
        let notice = ServeOptions { host: "::".into(), ..options.clone() }
            .web_ui_notice(Path::new("/app/web"));
        assert_eq!(notice, "Limina web UI: http://127.0.0.1:8080/ (serving /app/web)");
        let notice = ServeOptions { host: "::1".into(), ..options }.web_ui_notice(Path::new("w"));
        assert_eq!(notice, "Limina web UI: http://[::1]:8080/ (serving w)");
        assert_eq!(parse_python_flag("YES"), Ok(true));
        assert_eq!(parse_python_flag("on"), Ok(false));
    }
}
