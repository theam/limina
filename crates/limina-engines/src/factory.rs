//! The runtime's default session factory: real providers, configured from [`EngineSettings`].

use std::path::PathBuf;
use std::sync::Arc;

use limina_domain::Engine;

use crate::claude::session::{ClaudeAgentSession, ClaudeSessionConfig};
use crate::codex::accounts::{CodexAuthRouter, ProjectCodexAuth};
use crate::codex::session::{CodexAgentSession, CodexSessionConfig};
use crate::session::{AgentFactory, AgentSession};
use crate::settings::EngineSettings;

/// Creates each project's provider session.
pub struct DefaultAgentFactory {
    settings: EngineSettings,
    state_root: PathBuf,
    codex_auth: Arc<CodexAuthRouter>,
}

impl DefaultAgentFactory {
    /// `state_root` is the directory that holds the workspaces (the parent of the workspace
    /// root); Claude's sandbox denies reads of it, and default homes live under it.
    pub fn new(
        settings: EngineSettings,
        state_root: impl Into<PathBuf>,
        codex_auth: Arc<CodexAuthRouter>,
    ) -> Self {
        Self { settings, state_root: state_root.into(), codex_auth }
    }

    /// A Codex session whose credentials are resolved per turn: the project's own sign-in when
    /// it has one, otherwise the runtime's.
    pub fn codex_session(&self, slug: &str) -> CodexAgentSession {
        let config = CodexSessionConfig {
            codex_bin: self.settings.codex_bin.clone(),
            model: self.settings.codex_model.clone(),
            sandbox: self.settings.codex_sandbox.clone(),
            pricing: self.settings.codex_pricing.clone(),
        };
        CodexAgentSession::new(
            config,
            Arc::new(ProjectCodexAuth::new(Arc::clone(&self.codex_auth), slug)),
        )
    }

    /// A Claude session with a private configuration directory per project.
    pub fn claude_session(&self, slug: &str) -> ClaudeAgentSession {
        let config_root = self.settings.claude_config_root_or(&self.state_root);
        ClaudeAgentSession::new(ClaudeSessionConfig {
            claude_bin: self.settings.claude_bin.clone(),
            config_dir: config_root.join(slug),
            state_root: self.state_root.clone(),
            model: self.settings.claude_model.clone(),
            weaker_nested_sandbox: self.settings.claude_weaker_nested_sandbox,
        })
    }

    pub fn session(&self, slug: &str, engine: Engine) -> Arc<dyn AgentSession> {
        match engine {
            Engine::Codex => Arc::new(self.codex_session(slug)),
            Engine::ClaudeCode => Arc::new(self.claude_session(slug)),
        }
    }

    pub fn into_factory(self) -> AgentFactory {
        let factory = Arc::new(self);
        Arc::new(move |slug: &str, engine: Engine| Ok(factory.session(slug, engine)))
    }
}
