//! Per-project Codex accounts.
//!
//! A project owner can sign in with a device code. The sign-in lands in a Codex home that
//! belongs to that project alone (`<root>/<slug>`), where Codex keeps refreshing it. Projects
//! without one use the runtime's shared [`CodexAuthManager`]. The database side (who connected
//! the account and when) stays with the server; this module owns the homes and the managers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use limina_core::{Result, internal};

use super::auth::{CodexAuthConnector, CodexAuthManager, CodexAuthOptions};
use crate::process::blocking;

/// The project Codex homes under one root, with a lazily created manager per project.
pub struct ProjectCodexAccounts {
    root: PathBuf,
    codex_bin: String,
    connector: Option<Arc<dyn CodexAuthConnector>>,
    managers: tokio::sync::Mutex<HashMap<String, CodexAuthManager>>,
}

impl ProjectCodexAccounts {
    pub fn new(root: impl Into<PathBuf>, codex_bin: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            codex_bin: codex_bin.into(),
            connector: None,
            managers: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Uses a custom connector for every project manager (tests inject fakes here).
    #[must_use]
    pub fn with_connector(mut self, connector: Arc<dyn CodexAuthConnector>) -> Self {
        self.connector = Some(connector);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project's own Codex home.
    pub fn home(&self, slug: &str) -> PathBuf {
        self.root.join(slug)
    }

    /// True when the project's home holds a sign-in, even one completed while nobody watched.
    pub async fn signed_in(&self, slug: &str) -> bool {
        let auth_file = self.home(slug).join("auth.json");
        tokio::fs::try_exists(auth_file).await.unwrap_or(false)
    }

    /// The project's own credential store (ChatGPT sign-in only), created on first use.
    pub async fn manager(&self, slug: &str) -> Result<CodexAuthManager> {
        let mut managers = self.managers.lock().await;
        if let Some(manager) = managers.get(slug) {
            return Ok(manager.clone());
        }
        let options = CodexAuthOptions {
            connector: self.connector.clone(),
            ..CodexAuthOptions::chatgpt_only(self.codex_bin.clone())
        };
        let manager = CodexAuthManager::new(self.home(slug), options).await?;
        managers.insert(slug.to_owned(), manager.clone());
        Ok(manager)
    }

    /// Signs the project out. The credential file is removed even if Codex's own logout fails,
    /// and that failure is then returned.
    pub async fn sign_out(&self, slug: &str) -> Result<()> {
        if !self.signed_in(slug).await {
            return Ok(());
        }
        let logged_out = match self.manager(slug).await {
            Ok(manager) => manager.logout().await.map(|_| ()),
            Err(error) => Err(error),
        };
        let auth_file = self.home(slug).join("auth.json");
        let removed = blocking(move || match std::fs::remove_file(&auth_file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        })
        .await
        .map_err(internal);
        logged_out.and(removed)
    }

    /// Removes a deleted project's sign-in from disk.
    pub async fn forget(&self, slug: &str) {
        let manager = self.managers.lock().await.remove(slug);
        if let Some(manager) = manager {
            manager.close().await;
        }
        let home = self.home(slug);
        let _ = blocking(move || match std::fs::remove_dir_all(&home) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        })
        .await;
    }

    /// Cancels pending logins of every project manager.
    pub async fn close(&self) {
        let managers: Vec<CodexAuthManager> =
            self.managers.lock().await.drain().map(|(_, manager)| manager).collect();
        for manager in managers {
            manager.close().await;
        }
    }
}

/// Chooses the Codex credential store for a project: its own sign-in when it has one,
/// otherwise the runtime's.
pub struct CodexAuthRouter {
    runtime: CodexAuthManager,
    projects: Arc<ProjectCodexAccounts>,
}

impl CodexAuthRouter {
    pub fn new(runtime: CodexAuthManager, projects: Arc<ProjectCodexAccounts>) -> Self {
        Self { runtime, projects }
    }

    pub fn runtime(&self) -> &CodexAuthManager {
        &self.runtime
    }

    pub fn projects(&self) -> &Arc<ProjectCodexAccounts> {
        &self.projects
    }

    /// `codex_auth_for(slug)`. Resolved per turn, so an account connected mid-run applies from
    /// the next turn.
    pub async fn for_project(&self, slug: &str) -> Result<CodexAuthManager> {
        if self.projects.signed_in(slug).await {
            return self.projects.manager(slug).await;
        }
        Ok(self.runtime.clone())
    }
}

/// Resolves the credential store a Codex turn runs under.
#[async_trait]
pub trait CodexAuthResolver: Send + Sync {
    async fn resolve(&self) -> Result<CodexAuthManager>;
}

/// Always the same store.
#[async_trait]
impl CodexAuthResolver for CodexAuthManager {
    async fn resolve(&self) -> Result<CodexAuthManager> {
        Ok(self.clone())
    }
}

/// One project's store, through a [`CodexAuthRouter`].
pub struct ProjectCodexAuth {
    router: Arc<CodexAuthRouter>,
    slug: String,
}

impl ProjectCodexAuth {
    pub fn new(router: Arc<CodexAuthRouter>, slug: impl Into<String>) -> Self {
        Self { router, slug: slug.into() }
    }
}

#[async_trait]
impl CodexAuthResolver for ProjectCodexAuth {
    async fn resolve(&self) -> Result<CodexAuthManager> {
        self.router.for_project(&self.slug).await
    }
}
