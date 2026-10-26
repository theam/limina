//! Engine readiness: installed providers, server-side credentials and project accounts.
//!
//! Part of `supervisor.py` (`configured_engines`, `engine_status`, `codex_auth_status`,
//! `project_engine_status`). Only credential *sources* are reported, never their values.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use limina_domain::Engine;
use limina_domain::engines::SUPPORTED_ENGINES;
use limina_domain::environment::claude_credential_source;
use limina_engines::{
    CodexAuthRouter, CodexAuthStatus, EngineAvailability, EngineSettings, binary_installed,
    claude_availability, codex_availability,
};

use super::engine_accounts::EngineAccount;

/// How long a Codex account probe is reused: each probe starts a provider process.
pub const CODEX_STATUS_TTL: Duration = Duration::from_secs(10);

/// Reads the environment the runtime's credentials come from. The default reads the process
/// environment at call time, like the Python runtime's `os.environ`; tests inject a map.
pub type EnvironmentSource = Arc<dyn Fn() -> BTreeMap<String, String> + Send + Sync>;

/// The process environment, read on every call.
pub fn process_environment_source() -> EnvironmentSource {
    Arc::new(limina_domain::environment::process_environment)
}

/// Replaces the runtime's own readiness report for an engine (Python replaced
/// `supervisor.engine_status`). Project accounts are still reported as usual. The scripted
/// development engine uses it to report itself ready without probing any provider.
pub type EngineStatusOverride = Arc<dyn Fn(Engine) -> EngineAvailability + Send + Sync>;

pub(crate) struct EngineProbe {
    settings: EngineSettings,
    /// An injected agent factory runs every engine (`configured_engines` in Python).
    custom_factory: bool,
    environment: EnvironmentSource,
    codex: Arc<CodexAuthRouter>,
    codex_status: Mutex<Option<(Instant, CodexAuthStatus)>>,
    status_override: Option<EngineStatusOverride>,
}

impl EngineProbe {
    pub(crate) fn new(
        settings: EngineSettings,
        custom_factory: bool,
        environment: EnvironmentSource,
        codex: Arc<CodexAuthRouter>,
        status_override: Option<EngineStatusOverride>,
    ) -> Self {
        Self {
            settings,
            custom_factory,
            environment,
            codex,
            codex_status: Mutex::new(None),
            status_override,
        }
    }

    pub(crate) fn environment(&self) -> BTreeMap<String, String> {
        (self.environment)()
    }

    fn cache(&self) -> MutexGuard<'_, Option<(Instant, CodexAuthStatus)>> {
        self.codex_status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the engine's provider can be started: its CLI is on `PATH` (or at the configured
    /// path), or an injected factory provides sessions.
    pub(crate) async fn installed(&self, engine: Engine) -> bool {
        if self.custom_factory {
            return true;
        }
        match engine {
            Engine::Codex => binary_installed(&self.settings.codex_bin).await,
            Engine::ClaudeCode => binary_installed(&self.settings.claude_bin).await,
        }
    }

    /// Whether the runtime itself can run turns on `engine`.
    pub(crate) async fn engine_status(&self, engine: Engine) -> EngineAvailability {
        if let Some(status) = &self.status_override {
            return status(engine);
        }
        let installed = self.installed(engine).await;
        match engine {
            Engine::Codex => {
                let status = if installed { self.codex_auth_status().await } else { None };
                codex_availability(installed, status.as_ref(), Some(&self.settings.codex_model))
            }
            Engine::ClaudeCode => {
                let source = claude_credential_source(&self.environment());
                claude_availability(
                    installed,
                    source.as_deref(),
                    self.settings.claude_model.as_deref(),
                )
            }
        }
    }

    pub(crate) async fn engine_availability(&self) -> Vec<EngineAvailability> {
        let mut engines = Vec::with_capacity(SUPPORTED_ENGINES.len());
        for engine in SUPPORTED_ENGINES {
            engines.push(self.engine_status(engine).await);
        }
        engines
    }

    /// Readiness for one project: its own account first, then the runtime's.
    pub(crate) async fn project_engine_status(
        &self,
        engine: Engine,
        account: Option<&EngineAccount>,
    ) -> EngineAvailability {
        let Some(account) = account else {
            return self.engine_status(engine).await;
        };
        let installed = self.installed(engine).await;
        let mut detail = "Runs on the project's own account".to_owned();
        if let Some(label) = account.label.as_deref().filter(|label| !label.is_empty()) {
            detail.push_str(&format!(" ({label})"));
        }
        if let Some(who) = account.connected_by.as_deref().filter(|who| !who.is_empty()) {
            detail.push_str(&format!(", connected by {who}"));
        }
        detail.push('.');
        EngineAvailability {
            engine: engine.id().to_owned(),
            label: engine.label().to_owned(),
            installed,
            credentials: installed,
            detail,
            model: None,
        }
    }

    /// The runtime's Codex account, cached for [`CODEX_STATUS_TTL`]. `None` while a login or
    /// logout owns the credential store, instead of waiting for it (and that is not cached).
    pub(crate) async fn codex_auth_status(&self) -> Option<CodexAuthStatus> {
        let started = Instant::now();
        if let Some((at, status)) = self.cache().as_ref()
            && started.duration_since(*at) < CODEX_STATUS_TTL
        {
            return Some(status.clone());
        }
        let status = self.codex.runtime().status_if_idle().await;
        *self.cache() = status.clone().map(|status| (started, status));
        status
    }

    pub(crate) fn forget_codex_auth_status(&self) {
        *self.cache() = None;
    }
}
