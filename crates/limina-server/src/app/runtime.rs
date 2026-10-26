//! Wires the application layer from configuration (the non-HTTP half of Python's `create_app`).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use limina_core::{LiminaError, Result, internal};
use limina_domain::collaboration::CollaborationService;
use limina_domain::environment::process_environment;
use limina_domain::exporter::MarkdownExporter;
use limina_domain::retry::RetryPolicy;
use limina_domain::service::ChallengeService;
use limina_engines::codex::auth::CodexAuthConnector;
use limina_engines::{
    AgentFactory, ClaudeLogins, CodexAuthManager, CodexAuthOptions, CodexAuthRouter,
    DefaultAgentFactory, EngineSettings, ProjectCodexAccounts,
};
use limina_store::{Db, SecretCipher};

use super::engine_accounts::ProjectEngineAccounts;
use super::engine_status::{EngineStatusOverride, EnvironmentSource, process_environment_source};
use super::notifier::EventNotifier;
use super::operations::{DEFAULT_UPLOAD_LIMIT_BYTES, ProjectOperations};
use super::supervisor::{ProjectSupervisor, SupervisorOptions};

/// Where the runtime keeps its database when `LIMINA_DATABASE_URL` is not set.
pub const DEFAULT_DATABASE_URL: &str = "sqlite:///.limina/runtime.db";
/// Where project workspaces live when `LIMINA_WORKSPACE_ROOT` is not set.
pub const DEFAULT_WORKSPACE_ROOT: &str = ".limina/workspaces";
/// The internal agent API when `LIMINA_INTERNAL_URL` is not set.
pub const DEFAULT_INTERNAL_URL: &str = "http://127.0.0.1:7433";
/// A project's coordinator lease lasts this long unless renewed.
pub const DEFAULT_LEASE_TTL_SECONDS: i64 = 3600;
/// How often a `WAITING` project rechecks its inbox.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Everything [`Runtime::new`] needs. [`RuntimeConfig::from_env`] reads the environment;
/// [`RuntimeConfig::new`] starts from the defaults for explicit (test) configurations.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// `LIMINA_DATABASE_URL`: `sqlite:///…` or `postgresql://…`.
    pub database_url: String,
    /// `LIMINA_WORKSPACE_ROOT`. Its parent is the runtime's state root, which holds the shared
    /// Codex home (`codex/`), project Codex homes (`accounts/codex/<slug>`), per-project Claude
    /// configuration (`claude/<slug>`) and, by default, the secret key.
    pub workspace_root: PathBuf,
    /// `LIMINA_SECRET_KEY`: a Fernet key for project secrets and Claude tokens. Without one, the
    /// key file at `secret_key_path` is used (created on first start, mode 0600).
    pub secret_key: Option<String>,
    /// `LIMINA_SECRET_KEY_PATH` (default `<state root>/secret.key`).
    pub secret_key_path: PathBuf,
    /// `LIMINA_INTERNAL_URL`: how provider processes reach the internal agent API.
    pub internal_url: String,
    pub poll_interval: Duration,
    pub lease_ttl_seconds: i64,
    /// `LIMINA_RUNTIME_RETRY_DELAYS_SECONDS` (`30,120,600`).
    pub retry_policy: RetryPolicy,
    /// `LIMINA_UPLOAD_LIMIT_BYTES` (25 MiB).
    pub upload_limit_bytes: u64,
    /// Engine binaries, models, sandbox, Codex auth mode and server credentials (see
    /// [`EngineSettings`]).
    pub engines: EngineSettings,
    /// Creates each project's provider session. `None` uses the real Codex and Claude sessions;
    /// tests inject fakes (every engine then counts as installed).
    pub agent_factory: Option<AgentFactory>,
    /// Replaces the `codex app-server` used for Codex account reads and logins (tests).
    pub codex_connector: Option<Arc<dyn CodexAuthConnector>>,
    /// Where server-side Claude credentials and the inherited `PATH` are read from at call time
    /// (the process environment unless replaced).
    pub environment: EnvironmentSource,
    /// Replaces the runtime's own engine readiness report, as a scripted engine needs (see
    /// [`EngineStatusOverride`]). `None` probes the installed providers.
    pub engine_status: Option<EngineStatusOverride>,
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("database_url", &self.database_url)
            .field("workspace_root", &self.workspace_root)
            .field("secret_key", &self.secret_key.as_ref().map(|_| "[REDACTED]"))
            .field("secret_key_path", &self.secret_key_path)
            .field("internal_url", &self.internal_url)
            .field("poll_interval", &self.poll_interval)
            .field("lease_ttl_seconds", &self.lease_ttl_seconds)
            .field("retry_policy", &self.retry_policy)
            .field("upload_limit_bytes", &self.upload_limit_bytes)
            .field("custom_agent_factory", &self.agent_factory.is_some())
            .field("custom_engine_status", &self.engine_status.is_some())
            .finish_non_exhaustive()
    }
}

impl RuntimeConfig {
    /// Defaults for everything but the database and the workspaces; nothing is read from the
    /// environment here (the secret key file still honors `LIMINA_SECRET_KEY` at startup).
    pub fn new(database_url: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        Self {
            database_url: database_url.into(),
            secret_key: None,
            secret_key_path: state_root(&workspace_root).join("secret.key"),
            workspace_root,
            internal_url: DEFAULT_INTERNAL_URL.to_owned(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            lease_ttl_seconds: DEFAULT_LEASE_TTL_SECONDS,
            retry_policy: RetryPolicy::default(),
            upload_limit_bytes: DEFAULT_UPLOAD_LIMIT_BYTES,
            engines: default_engine_settings(),
            agent_factory: None,
            codex_connector: None,
            environment: process_environment_source(),
            engine_status: None,
        }
    }

    /// The configuration from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_map(&process_environment())
    }

    /// The configuration from an explicit environment map.
    pub fn from_map(env: &BTreeMap<String, String>) -> Result<Self> {
        let value = |name: &str| env.get(name).map(|value| value.trim()).filter(|v| !v.is_empty());
        let workspace_root =
            PathBuf::from(value("LIMINA_WORKSPACE_ROOT").unwrap_or(DEFAULT_WORKSPACE_ROOT));
        let mut config =
            Self::new(value("LIMINA_DATABASE_URL").unwrap_or(DEFAULT_DATABASE_URL), workspace_root);
        config.secret_key = value("LIMINA_SECRET_KEY").map(str::to_owned);
        if let Some(path) = value("LIMINA_SECRET_KEY_PATH") {
            config.secret_key_path = PathBuf::from(path);
        }
        if let Some(url) = value("LIMINA_INTERNAL_URL") {
            config.internal_url = url.to_owned();
        }
        config.retry_policy = RetryPolicy::parse(
            env.get("LIMINA_RUNTIME_RETRY_DELAYS_SECONDS").map_or("30,120,600", String::as_str),
        )
        .map_err(LiminaError::invariant)?;
        if let Some(limit) = value("LIMINA_UPLOAD_LIMIT_BYTES") {
            config.upload_limit_bytes = limit.parse().map_err(|_| {
                LiminaError::invariant("LIMINA_UPLOAD_LIMIT_BYTES must be a whole number of bytes.")
            })?;
        }
        config.engines = EngineSettings::from_map(env).map_err(LiminaError::invariant)?;
        Ok(config)
    }

    /// The directory that holds the workspace root (`.limina` by default).
    pub fn state_root(&self) -> PathBuf {
        state_root(&self.workspace_root)
    }
}

#[allow(clippy::expect_used, reason = "without prices there is nothing that can fail to parse")]
fn default_engine_settings() -> EngineSettings {
    EngineSettings::from_map(&BTreeMap::new()).expect("default engine settings are valid")
}

fn state_root(workspace_root: &Path) -> PathBuf {
    match workspace_root.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        Some(_) => PathBuf::from("."),
        None => workspace_root.to_path_buf(),
    }
}

/// The application layer: services, supervisor, operations and the event notifier, sharing one
/// database and secret cipher.
///
/// ```no_run
/// # async fn serve() -> limina_core::Result<()> {
/// use limina_server::app::{Runtime, RuntimeConfig};
///
/// let runtime = Runtime::new(RuntimeConfig::from_env()?).await?;
/// runtime.start().await?; // resumes projects left RUNNING or WAITING
/// // … serve HTTP and MCP with `runtime.operations()` …
/// runtime.shutdown().await;
/// # Ok(())
/// # }
/// ```
///
/// Cloning is cheap and shares every component (the HTTP and MCP transports each hold a clone).
#[derive(Clone)]
pub struct Runtime {
    db: Db,
    service: ChallengeService,
    collaboration: CollaborationService,
    exporter: MarkdownExporter,
    supervisor: ProjectSupervisor,
    operations: ProjectOperations,
    notifier: EventNotifier,
}

impl Runtime {
    /// Connects and initializes the database, loads the secret key, creates the Codex homes and
    /// builds every component. Nothing runs until [`Self::start`].
    pub async fn new(config: RuntimeConfig) -> Result<Self> {
        let db = Db::connect(&config.database_url).await?;
        db.initialize().await?;
        let workspace_root = std::path::absolute(&config.workspace_root).map_err(internal)?;
        let state_root = state_root(&workspace_root);
        let cipher =
            Arc::new(load_cipher(config.secret_key.clone(), config.secret_key_path.clone()).await?);
        let service = ChallengeService::new(db.clone(), Some(Arc::clone(&cipher)));
        let exporter = MarkdownExporter::new(service.clone());
        let collaboration = CollaborationService::new(db.clone());
        let notifier = EventNotifier::new();
        let settings = config.engines.clone();
        let runtime_codex = CodexAuthManager::new(
            settings.codex_home_or(&state_root),
            CodexAuthOptions {
                mode: settings.codex_auth_mode.clone(),
                api_key: settings.openai_api_key.clone(),
                access_token: settings.codex_access_token.clone(),
                codex_bin: settings.codex_bin.clone(),
                connector: config.codex_connector.clone(),
                login_command: None,
            },
        )
        .await?;
        let mut project_codex = ProjectCodexAccounts::new(
            state_root.join("accounts").join("codex"),
            settings.codex_bin.clone(),
        );
        if let Some(connector) = &config.codex_connector {
            project_codex = project_codex.with_connector(Arc::clone(connector));
        }
        let project_codex = Arc::new(project_codex);
        let router = Arc::new(CodexAuthRouter::new(runtime_codex, Arc::clone(&project_codex)));
        let accounts = Arc::new(ProjectEngineAccounts::new(
            db.clone(),
            Some(Arc::clone(&cipher)),
            project_codex,
            Arc::new(ClaudeLogins::new(settings.claude_bin.clone())),
            settings.claude_config_root_or(&state_root),
        ));
        let custom_agent_factory = config.agent_factory.is_some();
        let agent_factory = config.agent_factory.clone().unwrap_or_else(|| {
            DefaultAgentFactory::new(settings.clone(), state_root.clone(), Arc::clone(&router))
                .into_factory()
        });
        let supervisor = ProjectSupervisor::new(
            service.clone(),
            exporter.clone(),
            router,
            accounts,
            notifier.clone(),
            SupervisorOptions {
                workspace_root,
                internal_url: config.internal_url.clone(),
                agent_factory,
                custom_agent_factory,
                poll_interval: config.poll_interval,
                lease_ttl_seconds: config.lease_ttl_seconds,
                retry_policy: config.retry_policy.clone(),
                engines: settings,
                environment: Arc::clone(&config.environment),
                engine_status: config.engine_status.clone(),
            },
        );
        let operations = ProjectOperations::new(
            service.clone(),
            exporter.clone(),
            supervisor.clone(),
            notifier.clone(),
            config.upload_limit_bytes,
        );
        Ok(Self { db, service, collaboration, exporter, supervisor, operations, notifier })
    }

    /// Resumes every project left `RUNNING` or `WAITING` (Python's startup `recover()`).
    pub async fn start(&self) -> Result<()> {
        self.supervisor.recover().await
    }

    /// Stops managed execution gracefully (see [`ProjectSupervisor::shutdown`]). The database
    /// stays open for requests still finishing; drop the runtime to close it.
    pub async fn shutdown(&self) {
        self.supervisor.shutdown().await;
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn service(&self) -> &ChallengeService {
        &self.service
    }

    pub fn collaboration(&self) -> &CollaborationService {
        &self.collaboration
    }

    pub fn exporter(&self) -> &MarkdownExporter {
        &self.exporter
    }

    pub fn supervisor(&self) -> &ProjectSupervisor {
        &self.supervisor
    }

    pub fn operations(&self) -> &ProjectOperations {
        &self.operations
    }

    pub fn notifier(&self) -> &EventNotifier {
        &self.notifier
    }

    /// The absolute workspace root.
    pub fn workspace_root(&self) -> &Path {
        self.supervisor.workspace_root()
    }

    /// The actor an internal agent API call acts as, from its `Authorization` header
    /// (`Bearer <capability>`) and optional `X-Limina-Agent-Lane`. Errors are
    /// `authentication_required` (401) with the Python runtime's messages.
    pub fn internal_actor(
        &self,
        authorization: Option<&str>,
        slug: &str,
        lane: Option<&str>,
    ) -> Result<String> {
        self.supervisor.internal_actor(authorization, slug, lane)
    }
}

async fn load_cipher(key: Option<String>, path: PathBuf) -> Result<SecretCipher> {
    tokio::task::spawn_blocking(move || match key {
        Some(key) => SecretCipher::new(&key),
        None => SecretCipher::load(&path),
    })
    .await
    .map_err(internal)?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn defaults_match_the_python_runtime() {
        let config = RuntimeConfig::from_map(&BTreeMap::new()).unwrap();
        assert_eq!(config.database_url, DEFAULT_DATABASE_URL);
        assert_eq!(config.workspace_root, PathBuf::from(".limina/workspaces"));
        assert_eq!(config.secret_key_path, PathBuf::from(".limina/secret.key"));
        assert_eq!(config.internal_url, "http://127.0.0.1:7433");
        assert_eq!(config.retry_policy.delays_seconds, vec![30.0, 120.0, 600.0]);
        assert_eq!(config.upload_limit_bytes, 25 * 1024 * 1024);
        assert_eq!(config.lease_ttl_seconds, 3600);
        assert_eq!(config.state_root(), PathBuf::from(".limina"));
        assert_eq!(state_root(Path::new("workspaces")), PathBuf::from("."));
    }

    #[test]
    fn the_environment_overrides_every_setting() {
        let config = RuntimeConfig::from_map(&map(&[
            ("LIMINA_DATABASE_URL", "sqlite:////srv/limina.db"),
            ("LIMINA_WORKSPACE_ROOT", "/srv/limina/workspaces"),
            ("LIMINA_SECRET_KEY", "key"),
            ("LIMINA_SECRET_KEY_PATH", "/run/secrets/limina.key"),
            ("LIMINA_INTERNAL_URL", "http://runtime:7433"),
            ("LIMINA_RUNTIME_RETRY_DELAYS_SECONDS", "1,2"),
            ("LIMINA_UPLOAD_LIMIT_BYTES", "1024"),
            ("LIMINA_CODEX_MODEL", "gpt-test"),
        ]))
        .unwrap();
        assert_eq!(config.database_url, "sqlite:////srv/limina.db");
        assert_eq!(config.state_root(), PathBuf::from("/srv/limina"));
        assert_eq!(config.secret_key.as_deref(), Some("key"));
        assert_eq!(config.secret_key_path, PathBuf::from("/run/secrets/limina.key"));
        assert_eq!(config.internal_url, "http://runtime:7433");
        assert_eq!(config.retry_policy.delays_seconds, vec![1.0, 2.0]);
        assert_eq!(config.upload_limit_bytes, 1024);
        assert_eq!(config.engines.codex_model, "gpt-test");
        assert!(!format!("{config:?}").contains("\"key\""));
        let invalid = map(&[("LIMINA_RUNTIME_RETRY_DELAYS_SECONDS", "soon")]);
        assert_eq!(
            RuntimeConfig::from_map(&invalid).unwrap_err().message,
            "Runtime retry delays must be comma-separated seconds."
        );
    }
}
