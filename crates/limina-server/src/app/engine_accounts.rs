//! Per-project provider accounts: each project can run on the account its owner connects.
//!
//! - **Claude**: the owner signs in with their own Claude account from the console, through the
//!   official CLI login ([`ClaudeLogins`]); the credentials land in the project's private Claude
//!   configuration directory, where the CLI keeps refreshing them. Alternatively the owner pastes
//!   a long-lived token (`claude setup-token`) or an API key, stored encrypted and injected only
//!   into that project's Claude process. Either way it replaces any runtime-wide credential, and
//!   no view ever contains a secret.
//! - **Codex**: the owner signs in with a device code. The sign-in lands in a Codex home that
//!   belongs to the project alone ([`ProjectCodexAccounts`]), where Codex keeps refreshing it.
//!
//! Projects without an account fall back to the runtime's shared engine credentials.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use limina_core::time::serde_iso_opt;
use limina_core::{LiminaError, Principal, Result, Timestamp, now};
use limina_domain::Engine;
use limina_engines::{ClaudeLogins, CodexAuthManager, DeviceLoginAttempt, ProjectCodexAccounts};
use limina_store::models::{Challenge, ProjectEngineAccount};
use limina_store::{Db, SecretCipher, Tx, args};
use serde::{Deserialize, Serialize};

/// The cipher binds the stored token to this name (and to the project).
const SECRET_NAME: &str = "engine-account:claude-code";
/// Claude Code prefers these over a subscription token, so a project account must clear them.
pub const CLAUDE_COMPETING_ENV: [&str; 6] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];
/// Shortest value accepted as a Claude token.
const TOKEN_MINIMUM: usize = 20;

/// Which account a project runs one engine on (`EngineAccount`). Never contains a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineAccount {
    pub engine: Engine,
    /// `claude-login`, `subscription` or `api-key` for Claude, `chatgpt` for Codex.
    pub method: String,
    /// `…abcd` for Claude tokens, `email · plan` for sign-ins.
    pub label: Option<String>,
    pub connected_by: Option<String>,
    #[serde(with = "serde_iso_opt")]
    pub connected_at: Option<Timestamp>,
}

impl EngineAccount {
    fn from_row(row: &ProjectEngineAccount, engine: Engine) -> Self {
        Self {
            engine,
            method: row.method.clone(),
            label: row.label.clone(),
            connected_by: Some(row.connected_by.clone()),
            connected_at: Some(row.updated_at),
        }
    }
}

/// The Claude credential variables for a project's turns and the values to redact from them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeOverlay {
    /// Every competing credential set to `""` (withheld), plus the project's token.
    pub environment: BTreeMap<String, String>,
    pub secrets: Vec<String>,
}

/// `api-key` for Anthropic API keys, otherwise `subscription`.
pub fn claude_method(token: &str) -> &'static str {
    if token.starts_with("sk-ant-api") { "api-key" } else { "subscription" }
}

/// The account method for a Claude sign-in made through the console.
pub const CLAUDE_LOGIN: &str = "claude-login";

/// Project accounts: rows in `project_engine_accounts`, the project Codex homes, and the
/// project Claude configuration directories that hold console sign-ins.
pub struct ProjectEngineAccounts {
    db: Db,
    cipher: Option<Arc<SecretCipher>>,
    codex: Arc<ProjectCodexAccounts>,
    claude: Arc<ClaudeLogins>,
    claude_config_root: PathBuf,
}

impl ProjectEngineAccounts {
    /// `claude_config_root` is the directory with one Claude configuration directory per
    /// project (the same one the project's Claude sessions use).
    pub fn new(
        db: Db,
        cipher: Option<Arc<SecretCipher>>,
        codex: Arc<ProjectCodexAccounts>,
        claude: Arc<ClaudeLogins>,
        claude_config_root: PathBuf,
    ) -> Self {
        Self { db, cipher, codex, claude, claude_config_root }
    }

    /// The project's own Claude configuration directory.
    pub fn claude_config_dir(&self, slug: &str) -> PathBuf {
        self.claude_config_root.join(canonical(slug))
    }

    pub fn codex_accounts(&self) -> &Arc<ProjectCodexAccounts> {
        &self.codex
    }

    // Reading ----------------------------------------------------------------------------------

    /// The account a project runs `engine` on, if it has its own. A Codex sign-in completed
    /// while nobody was watching still counts; its owner is then unknown.
    pub async fn account(&self, slug: &str, engine: Engine) -> Result<Option<EngineAccount>> {
        let (project, row) = self.row(slug, engine).await?;
        let public = row.as_ref().map(|row| EngineAccount::from_row(row, engine));
        if engine == Engine::Codex {
            if !self.codex.signed_in(&project).await {
                return Ok(None);
            }
            return Ok(Some(public.unwrap_or(EngineAccount {
                engine,
                method: "chatgpt".to_owned(),
                label: None,
                connected_by: None,
                connected_at: None,
            })));
        }
        Ok(public)
    }

    pub async fn has_account(&self, slug: &str, engine: Engine) -> Result<bool> {
        Ok(self.account(slug, engine).await?.is_some())
    }

    /// The environment overlay for the project's Claude process, plus the values to redact.
    pub async fn claude_environment(&self, slug: &str) -> Result<ClaudeOverlay> {
        let (project, Some(row)) = self.row(slug, Engine::ClaudeCode).await? else {
            return Ok(ClaudeOverlay::default());
        };
        let mut environment: BTreeMap<String, String> =
            CLAUDE_COMPETING_ENV.iter().map(|name| ((*name).to_owned(), String::new())).collect();
        if row.method == CLAUDE_LOGIN {
            // The CLI reads the sign-in from the project's configuration directory.
            return Ok(ClaudeOverlay { environment, secrets: Vec::new() });
        }
        let Some(ciphertext) = row.secret_ciphertext.as_deref().filter(|text| !text.is_empty())
        else {
            return Ok(ClaudeOverlay::default());
        };
        let token = self.cipher()?.decrypt(&project, SECRET_NAME, ciphertext)?;
        let name =
            if row.method == "api-key" { "ANTHROPIC_API_KEY" } else { "CLAUDE_CODE_OAUTH_TOKEN" };
        environment.insert(name.to_owned(), token.clone());
        Ok(ClaudeOverlay { environment, secrets: vec![token] })
    }

    /// The project's own Codex home.
    pub fn codex_home(&self, slug: &str) -> PathBuf {
        self.codex.home(&canonical(slug))
    }

    /// The project's own Codex credential store (ChatGPT sign-in only).
    pub async fn codex_manager(&self, slug: &str) -> Result<CodexAuthManager> {
        self.codex.manager(&canonical(slug)).await
    }

    // Claude -----------------------------------------------------------------------------------

    /// Stores a Claude token (encrypted) as the project's Claude account.
    pub async fn connect_claude(
        &self,
        slug: &str,
        token: &str,
        principal: &Principal,
    ) -> Result<EngineAccount> {
        let value = token.trim_matches(is_python_space);
        if value.chars().count() < TOKEN_MINIMUM || value.chars().any(is_python_space) {
            return Err(LiminaError::invariant("That doesn't look like a Claude token.")
                .with("suggestion", "Run `claude setup-token` and paste the token it prints."));
        }
        let cipher = self.cipher()?;
        let project = self.project_slug(slug).await?;
        let ciphertext = cipher.encrypt(&project, SECRET_NAME, value);
        let tail: String = {
            let characters: Vec<char> = value.chars().collect();
            characters[characters.len().saturating_sub(4)..].iter().collect()
        };
        self.upsert(
            slug,
            Engine::ClaudeCode,
            principal,
            claude_method(value),
            Some(&format!("…{tail}")),
            Some(&ciphertext),
        )
        .await
    }

    /// Starts a Claude sign-in into the project's configuration directory; the result carries
    /// the claude.com URL to open.
    pub async fn start_claude_login(
        &self,
        slug: &str,
        command_id: &str,
    ) -> Result<DeviceLoginAttempt> {
        let project = self.project_slug(slug).await?;
        let config_dir = self.claude_config_dir(&project);
        tokio::fs::create_dir_all(&config_dir).await.map_err(|error| {
            LiminaError::transport(format!("Cannot prepare Claude's directory: {error}"))
        })?;
        self.claude.start(&project, &config_dir, command_id).await
    }

    /// A Claude sign-in attempt; once it succeeded the account is recorded for the project.
    pub async fn claude_login(
        &self,
        slug: &str,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.owned_claude_attempt(slug, login_id).await?;
        let attempt = self.claude.attempt(login_id).await?;
        self.record_claude_login(slug, &attempt, principal).await?;
        Ok(attempt)
    }

    /// Hands the code claude.com showed to the waiting sign-in.
    pub async fn submit_claude_code(
        &self,
        slug: &str,
        login_id: &str,
        code: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.owned_claude_attempt(slug, login_id).await?;
        let attempt = self.claude.submit_code(login_id, code).await?;
        self.record_claude_login(slug, &attempt, principal).await?;
        Ok(attempt)
    }

    pub async fn cancel_claude_login(
        &self,
        slug: &str,
        login_id: &str,
    ) -> Result<DeviceLoginAttempt> {
        self.owned_claude_attempt(slug, login_id).await?;
        self.claude.cancel(login_id).await
    }

    async fn owned_claude_attempt(&self, slug: &str, login_id: &str) -> Result<()> {
        let project = self.project_slug(slug).await?;
        if self.claude.project_of(login_id)? != project {
            return Err(LiminaError::not_found(
                "That Claude sign-in does not exist or has expired.",
            )
            .with("login_id", login_id));
        }
        Ok(())
    }

    async fn record_claude_login(
        &self,
        slug: &str,
        attempt: &DeviceLoginAttempt,
        principal: &Principal,
    ) -> Result<()> {
        if attempt.status != "SUCCEEDED" {
            return Ok(());
        }
        let (_, row) = self.row(slug, Engine::ClaudeCode).await?;
        if row.is_some_and(|row| {
            row.method == CLAUDE_LOGIN && row.connected_by_subject == principal.subject
        }) {
            return Ok(());
        }
        let label = self.claude.status(&self.claude_config_dir(slug)).await.label();
        self.upsert(slug, Engine::ClaudeCode, principal, CLAUDE_LOGIN, label.as_deref(), None)
            .await?;
        Ok(())
    }

    // Codex ------------------------------------------------------------------------------------

    /// Starts a device-code sign-in into the project's Codex home.
    pub async fn start_codex_login(
        &self,
        slug: &str,
        command_id: &str,
    ) -> Result<DeviceLoginAttempt> {
        self.project_slug(slug).await?;
        self.codex_manager(slug).await?.start_device_login(command_id).await
    }

    /// A sign-in attempt. Once it succeeded, the account is recorded as connected by
    /// `principal` and labelled with the signed-in account (this reads it from Codex).
    pub async fn codex_login(
        &self,
        slug: &str,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        let manager = self.codex_manager(slug).await?;
        let attempt = manager.login_attempt(login_id)?;
        if attempt.status == "SUCCEEDED" {
            let label = manager.status().await.account_label();
            self.upsert(slug, Engine::Codex, principal, "chatgpt", label.as_deref(), None).await?;
        }
        Ok(attempt)
    }

    pub async fn cancel_codex_login(
        &self,
        slug: &str,
        login_id: &str,
    ) -> Result<DeviceLoginAttempt> {
        self.codex_manager(slug).await?.cancel_device_login(login_id).await
    }

    // Disconnect -------------------------------------------------------------------------------

    /// Removes the project's account. A Codex sign-in is logged out and its credential file is
    /// removed even when the provider logout fails (that failure is then returned).
    pub async fn disconnect(&self, slug: &str, engine: Engine) -> Result<()> {
        if engine == Engine::Codex {
            self.codex.sign_out(&canonical(slug)).await?;
        } else {
            // Sign the CLI out too, so no console sign-in survives in the project directory.
            self.claude.logout(&self.claude_config_dir(slug)).await;
        }
        let mut tx = self.db.begin_write().await?;
        let challenge = challenge(&mut tx, slug).await?;
        tx.execute(
            "DELETE FROM project_engine_accounts WHERE challenge_id = $1 AND engine = $2",
            args![&challenge.id, engine.id()],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Removes on-disk sign-ins when a project goes away.
    pub async fn forget_project(&self, slug: &str) {
        self.codex.forget(&canonical(slug)).await;
    }

    /// Cancels pending project logins.
    pub async fn close(&self) {
        self.codex.close().await;
        self.claude.close().await;
    }

    // Internals --------------------------------------------------------------------------------

    /// The project's canonical slug and its account row for `engine`.
    async fn row(
        &self,
        slug: &str,
        engine: Engine,
    ) -> Result<(String, Option<ProjectEngineAccount>)> {
        let mut tx = self.db.begin().await?;
        let challenge = challenge(&mut tx, slug).await?;
        let row: Option<ProjectEngineAccount> = tx
            .fetch_optional(
                "SELECT * FROM project_engine_accounts WHERE challenge_id = $1 AND engine = $2",
                args![&challenge.id, engine.id()],
            )
            .await?;
        tx.commit().await?;
        Ok((challenge.slug, row))
    }

    async fn upsert(
        &self,
        slug: &str,
        engine: Engine,
        principal: &Principal,
        method: &str,
        label: Option<&str>,
        secret_ciphertext: Option<&str>,
    ) -> Result<EngineAccount> {
        let changed = now();
        let mut tx = self.db.begin_write().await?;
        let challenge = challenge(&mut tx, slug).await?;
        let row: ProjectEngineAccount = tx
            .fetch_one(
                "INSERT INTO project_engine_accounts (challenge_id, engine, method, label, \
                 secret_ciphertext, connected_by, connected_by_subject, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8) \
                 ON CONFLICT (challenge_id, engine) DO UPDATE SET method = excluded.method, \
                 label = excluded.label, secret_ciphertext = excluded.secret_ciphertext, \
                 connected_by = excluded.connected_by, \
                 connected_by_subject = excluded.connected_by_subject, \
                 updated_at = excluded.updated_at RETURNING *",
                args![
                    &challenge.id,
                    engine.id(),
                    method,
                    label,
                    secret_ciphertext,
                    principal.actor(),
                    &principal.subject,
                    changed
                ],
            )
            .await?;
        tx.commit().await?;
        Ok(EngineAccount::from_row(&row, engine))
    }

    fn cipher(&self) -> Result<&SecretCipher> {
        self.cipher.as_deref().ok_or_else(|| {
            LiminaError::invariant("This runtime has no secret key, so it cannot store tokens.")
        })
    }

    /// The project's canonical slug, or the not-found error.
    async fn project_slug(&self, slug: &str) -> Result<String> {
        let mut tx = self.db.begin().await?;
        let challenge = challenge(&mut tx, slug).await?;
        tx.commit().await?;
        Ok(challenge.slug)
    }
}

/// The project with this slug (case-insensitive), or `Project '<slug>' does not exist.`
async fn challenge(tx: &mut Tx, slug: &str) -> Result<Challenge> {
    let challenge: Option<Challenge> = tx
        .fetch_optional("SELECT * FROM challenges WHERE slug = $1", args![slug.to_lowercase()])
        .await?;
    challenge.ok_or_else(|| {
        LiminaError::not_found(format!("Project '{slug}' does not exist.")).with("project", slug)
    })
}

/// Slugs are lower-case; homes are keyed by the stored form.
fn canonical(slug: &str) -> String {
    slug.to_lowercase()
}

/// Python's `str.isspace` (Unicode whitespace, including the ASCII separators U+001C–U+001F).
fn is_python_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_methods_and_whitespace_follow_python() {
        assert_eq!(claude_method("sk-ant-api03-abc"), "api-key");
        assert_eq!(claude_method("sk-ant-oat01-abc"), "subscription");
        assert!(is_python_space('\u{1f}'));
        assert!(is_python_space('\u{a0}'));
        assert!(!is_python_space('\u{200b}'));
    }
}
