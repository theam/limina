//! Instance-owned Codex authentication with durable state and turn-safe mutation.
//!
//! A [`CodexAuthManager`] owns one Codex home (`auth.json` and Codex's other state). Turns share
//! a read gate for their whole duration; logins and logouts take the writer and are refused while
//! a turn is active. ChatGPT turns are additionally serialized, since Codex refreshes that token
//! in place. Every account probe and login runs in a short-lived `codex app-server` started with
//! the same isolated environment as a turn, so credentials never pass through Limina's own
//! environment.
//!
//! `auth.json` is intentionally readable by Codex itself. The manager removes raw provider
//! credentials from the child environment, but does not hide the credential store from the
//! provider process that must use it.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use limina_core::{LiminaError, Result, Timestamp, internal, iso, now};
use limina_domain::environment::{
    codex_environment, ensure_private_directory, process_environment,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::OwnedMutexGuard;

use super::gate::{Gate, ReadGuard};
use super::rpc::{AppServerClient, CodexLaunch, Incoming, LOG_TARGET};
use crate::error::ProviderError;
use crate::process::blocking;
use crate::text::py_str;

pub const AUTH_MODES: [&str; 4] = ["auto", "chatgpt", "api-key", "access-token"];
/// Keeps Codex's credentials in `auth.json` inside the managed home.
pub const FILE_CREDENTIAL_STORE: &str = r#"cli_auth_credentials_store="file""#;
const MAX_LOGIN_ATTEMPTS: usize = 32;
const ACCESS_TOKEN_LOGIN_TIMEOUT: Duration = Duration::from_secs(60);
const LOGIN_FAILED: &str = "ChatGPT login failed or expired.";

// The provider seam --------------------------------------------------------------------------

/// A Codex account as `account/read` reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexAccount {
    /// The wire `type`: `chatgpt`, `apiKey`, `amazonBedrock`, …
    pub account_type: String,
    pub email: Option<String>,
    pub plan_type: Option<String>,
}

/// A started device-code login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub login_id: String,
    pub verification_url: String,
    pub user_code: String,
}

/// `account/login/completed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCompletion {
    pub success: bool,
    pub error: Option<String>,
}

/// One connection to Codex used for account operations. Methods may run concurrently (a device
/// login is cancelled while another task waits for it).
#[async_trait]
pub trait CodexAuthClient: Send + Sync {
    async fn account(&self) -> std::result::Result<Option<CodexAccount>, ProviderError>;
    async fn login_api_key(&self, api_key: &str) -> std::result::Result<(), ProviderError>;
    async fn login_chatgpt_device_code(&self) -> std::result::Result<DeviceCode, ProviderError>;
    async fn wait_login(
        &self,
        login_id: &str,
    ) -> std::result::Result<LoginCompletion, ProviderError>;
    async fn cancel_login(&self, login_id: &str) -> std::result::Result<(), ProviderError>;
    async fn logout(&self) -> std::result::Result<(), ProviderError>;
    async fn close(&self);
}

/// Opens [`CodexAuthClient`]s for a child environment (the manager's Codex home). Tests inject
/// fakes here, like the Python runtime's `codex_factory`.
#[async_trait]
pub trait CodexAuthConnector: Send + Sync {
    async fn connect(
        &self,
        environment: BTreeMap<String, String>,
    ) -> std::result::Result<Arc<dyn CodexAuthClient>, ProviderError>;
}

/// The real connector: a `codex app-server` per operation, using the file credential store.
#[derive(Debug, Clone)]
pub struct AppServerAuthConnector {
    binary: String,
}

impl AppServerAuthConnector {
    pub fn new(binary: impl Into<String>) -> Self {
        Self { binary: binary.into() }
    }
}

#[async_trait]
impl CodexAuthConnector for AppServerAuthConnector {
    async fn connect(
        &self,
        environment: BTreeMap<String, String>,
    ) -> std::result::Result<Arc<dyn CodexAuthClient>, ProviderError> {
        let launch = CodexLaunch {
            binary: self.binary.clone(),
            config_overrides: vec![FILE_CREDENTIAL_STORE.to_owned()],
            cwd: None,
            env: environment,
            mcp_servers: Default::default(),
        };
        Ok(Arc::new(AppServerAuthClient { client: AppServerClient::start(&launch).await? }))
    }
}

struct AppServerAuthClient {
    client: AppServerClient,
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

#[async_trait]
impl CodexAuthClient for AppServerAuthClient {
    async fn account(&self) -> std::result::Result<Option<CodexAccount>, ProviderError> {
        let response =
            self.client.request("account/read", Some(json!({"refreshToken": false}))).await?;
        let Some(account) = response.get("account").and_then(Value::as_object) else {
            return Ok(None);
        };
        Ok(Some(CodexAccount {
            account_type: account.get("type").map(py_str).unwrap_or_default(),
            email: text(account.get("email")),
            plan_type: account.get("planType").filter(|plan| !plan.is_null()).map(py_str),
        }))
    }

    async fn login_api_key(&self, api_key: &str) -> std::result::Result<(), ProviderError> {
        let params = json!({"type": "apiKey", "apiKey": api_key});
        self.client.request("account/login/start", Some(params)).await.map(|_| ())
    }

    async fn login_chatgpt_device_code(&self) -> std::result::Result<DeviceCode, ProviderError> {
        let params = json!({"type": "chatgptDeviceCode"});
        let response = self.client.request("account/login/start", Some(params)).await?;
        let field = |name: &str| text(response.get(name));
        match (
            response.get("type").and_then(Value::as_str),
            field("loginId"),
            field("verificationUrl"),
            field("userCode"),
        ) {
            (
                Some("chatgptDeviceCode"),
                Some(login_id),
                Some(verification_url),
                Some(user_code),
            ) => Ok(DeviceCode { login_id, verification_url, user_code }),
            _ => Err(ProviderError::Protocol(format!(
                "unexpected device-code login response: {response}"
            ))),
        }
    }

    async fn wait_login(
        &self,
        login_id: &str,
    ) -> std::result::Result<LoginCompletion, ProviderError> {
        loop {
            match self.client.next_incoming().await {
                Incoming::Closed(error) => return Err(error),
                Incoming::Notification { method, params }
                    if method == "account/login/completed"
                        && params.get("loginId").and_then(Value::as_str) == Some(login_id) =>
                {
                    return Ok(LoginCompletion {
                        success: params.get("success").and_then(Value::as_bool).unwrap_or(false),
                        error: text(params.get("error")),
                    });
                }
                Incoming::Notification { .. } => {}
            }
        }
    }

    async fn cancel_login(&self, login_id: &str) -> std::result::Result<(), ProviderError> {
        let params = json!({"loginId": login_id});
        self.client.request("account/login/cancel", Some(params)).await.map(|_| ())
    }

    async fn logout(&self) -> std::result::Result<(), ProviderError> {
        self.client.request("account/logout", None).await.map(|_| ())
    }

    async fn close(&self) {
        self.client.shutdown().await;
    }
}

// Public views ---------------------------------------------------------------------------------

/// The account status the API and availability checks report. Field order matches the Python
/// dict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexAuthStatus {
    pub engine: String,
    pub configured_mode: String,
    pub configured: bool,
    /// `chatgpt` or `api-key`.
    pub active_method: Option<String>,
    pub account_email: Option<String>,
    pub account_plan: Option<String>,
    /// `cached_chatgpt`, `server_access_token`, `server_api_key`, `cached_api_key`, `none` or
    /// `unavailable`.
    pub source: String,
    pub error: Option<String>,
    pub single_runtime_node: bool,
}

impl CodexAuthStatus {
    /// `"email · plan"` from whatever the account reports, as project accounts label it.
    pub fn account_label(&self) -> Option<String> {
        let parts: Vec<&str> = [self.account_email.as_deref(), self.account_plan.as_deref()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    fn active_is(&self, method: &str) -> bool {
        self.active_method.as_deref() == Some(method)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginStatus {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

impl LoginStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
        }
    }
}

/// A device-code login attempt as the API returns it. The user code is shown only while the
/// attempt is pending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLoginAttempt {
    pub login_id: String,
    pub status: String,
    pub verification_url: String,
    pub user_code: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

struct Attempt {
    code: DeviceCode,
    command_id: String,
    status: LoginStatus,
    error: Option<String>,
    created_at: Timestamp,
    completed_at: Option<Timestamp>,
    client: Option<Arc<dyn CodexAuthClient>>,
    cancel_requested: bool,
}

impl Attempt {
    fn public(&self) -> DeviceLoginAttempt {
        DeviceLoginAttempt {
            login_id: self.code.login_id.clone(),
            status: self.status.as_str().to_owned(),
            verification_url: self.code.verification_url.clone(),
            user_code: (self.status == LoginStatus::Pending).then(|| self.code.user_code.clone()),
            error: self.error.clone(),
            created_at: iso(&self.created_at),
            completed_at: self.completed_at.as_ref().map(iso),
        }
    }
}

#[derive(Default)]
struct Attempts {
    by_id: HashMap<String, Attempt>,
    by_command: HashMap<String, String>,
}

impl Attempts {
    /// Keeps the registry bounded by dropping the oldest finished attempts.
    fn prune(&mut self) {
        let mut finished: Vec<(Timestamp, String)> = self
            .by_id
            .values()
            .filter(|attempt| attempt.status != LoginStatus::Pending)
            .map(|attempt| {
                (attempt.completed_at.unwrap_or(attempt.created_at), attempt.code.login_id.clone())
            })
            .collect();
        finished.sort();
        let mut finished = finished.into_iter();
        while self.by_id.len() >= MAX_LOGIN_ATTEMPTS {
            let Some((_, login_id)) = finished.next() else { break };
            if let Some(attempt) = self.by_id.remove(&login_id) {
                self.by_command.remove(&attempt.command_id);
            }
        }
    }
}

// The manager ----------------------------------------------------------------------------------

/// How a [`CodexAuthManager`] authenticates.
#[derive(Clone)]
pub struct CodexAuthOptions {
    /// `auto`, `chatgpt`, `api-key` or `access-token` (`LIMINA_CODEX_AUTH_MODE`).
    pub mode: String,
    /// `OPENAI_API_KEY`, materialized into `auth.json` once per process.
    pub api_key: Option<String>,
    /// `CODEX_ACCESS_TOKEN`, materialized with `codex login --with-access-token`.
    pub access_token: Option<String>,
    /// The Codex binary (`LIMINA_CODEX_BIN`).
    pub codex_bin: String,
    /// Defaults to [`AppServerAuthConnector`] over `codex_bin`.
    pub connector: Option<Arc<dyn CodexAuthConnector>>,
    /// Defaults to `[codex_bin, "login", "--with-access-token"]`.
    pub login_command: Option<Vec<String>>,
}

impl Default for CodexAuthOptions {
    fn default() -> Self {
        Self {
            mode: "auto".to_owned(),
            api_key: None,
            access_token: None,
            codex_bin: "codex".to_owned(),
            connector: None,
            login_command: None,
        }
    }
}

impl CodexAuthOptions {
    /// A project's own store: ChatGPT sign-in only.
    pub fn chatgpt_only(codex_bin: impl Into<String>) -> Self {
        Self { mode: "chatgpt".to_owned(), codex_bin: codex_bin.into(), ..Self::default() }
    }
}

/// Owns one Codex home's credential state. Cloning shares the same manager.
#[derive(Clone)]
pub struct CodexAuthManager {
    inner: Arc<Inner>,
}

struct Inner {
    home: PathBuf,
    mode: String,
    api_key: Option<String>,
    access_token: Option<String>,
    connector: Arc<dyn CodexAuthConnector>,
    login_command: Vec<String>,
    gate: Arc<Gate>,
    chatgpt_turn: Arc<tokio::sync::Mutex<()>>,
    materialized: AtomicBool,
    attempts: Mutex<Attempts>,
}

/// Held for a whole Codex turn: credentials cannot change underneath it.
pub struct CodexTurnGuard {
    status: CodexAuthStatus,
    _chatgpt_turn: Option<OwnedMutexGuard<()>>,
    _read: ReadGuard,
}

impl CodexTurnGuard {
    pub fn status(&self) -> &CodexAuthStatus {
        &self.status
    }
}

fn busy(active_turns: usize) -> LiminaError {
    LiminaError::conflict("Codex authentication cannot change while a managed turn is active.")
        .with("active_turns", active_turns)
}

fn missing_attempt(login_id: &str) -> LiminaError {
    LiminaError::invariant("The Codex login attempt does not exist.").with("login_id", login_id)
}

fn missing_auth() -> LiminaError {
    LiminaError::authentication_with(
        "Codex is not authenticated. Configure OPENAI_API_KEY/CODEX_ACCESS_TOKEN on the server \
         or run `limina runtime codex login`.",
    )
}

async fn secure_state_files(home: &Path) -> io::Result<()> {
    let home = home.to_path_buf();
    blocking(move || {
        ensure_private_directory(&home)?;
        let auth_file = home.join("auth.json");
        #[cfg(unix)]
        if auth_file.exists() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&auth_file, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    })
    .await
}

impl CodexAuthManager {
    /// Validates the configuration and creates the home as a private directory.
    pub async fn new(home: impl Into<PathBuf>, options: CodexAuthOptions) -> Result<Self> {
        let mode = options.mode.trim().to_lowercase();
        if !AUTH_MODES.contains(&mode.as_str()) {
            let mut sorted = AUTH_MODES;
            sorted.sort_unstable();
            let listed = sorted.map(|mode| format!("'{mode}'")).join(", ");
            return Err(LiminaError::invariant(format!(
                "LIMINA_CODEX_AUTH_MODE must be one of [{listed}]."
            )));
        }
        let api_key = options.api_key.filter(|value| !value.is_empty());
        let access_token = options.access_token.filter(|value| !value.is_empty());
        if api_key.is_some() && access_token.is_some() && mode == "auto" {
            return Err(LiminaError::invariant(
                "OPENAI_API_KEY and CODEX_ACCESS_TOKEN cannot both be set in auto auth mode.",
            ));
        }
        let home = home.into();
        let created = home.clone();
        blocking(move || ensure_private_directory(&created)).await.map_err(internal)?;
        let connector = options
            .connector
            .unwrap_or_else(|| Arc::new(AppServerAuthConnector::new(options.codex_bin.clone())));
        let login_command = options.login_command.unwrap_or_else(|| {
            vec![options.codex_bin.clone(), "login".to_owned(), "--with-access-token".to_owned()]
        });
        Ok(Self {
            inner: Arc::new(Inner {
                home,
                mode,
                api_key,
                access_token,
                connector,
                login_command,
                gate: Arc::new(Gate::default()),
                chatgpt_turn: Arc::new(tokio::sync::Mutex::new(())),
                materialized: AtomicBool::new(false),
                attempts: Mutex::new(Attempts::default()),
            }),
        })
    }

    pub fn home(&self) -> &Path {
        &self.inner.home
    }

    pub fn mode(&self) -> &str {
        &self.inner.mode
    }

    /// True when both handles share one manager (Python's `is`).
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Turns currently holding the read gate.
    pub fn active_turns(&self) -> usize {
        self.inner.gate.readers()
    }

    /// The account status, waiting while a login or logout owns the credential store.
    pub async fn status(&self) -> CodexAuthStatus {
        let _read = self.inner.gate.read().await;
        self.inner.account_status().await
    }

    /// The account status, or `None` while a login or logout owns the credential store.
    pub async fn status_if_idle(&self) -> Option<CodexAuthStatus> {
        let _read = self.inner.gate.try_read()?;
        Some(self.inner.account_status().await)
    }

    /// Makes sure Codex can run a turn, materializing server credentials if the mode needs it.
    pub async fn ensure_ready(&self) -> Result<CodexAuthStatus> {
        // Established credentials are read-only for a turn, so the common path stays shared and
        // independent API-key projects can run in parallel.
        {
            let _read = self.inner.gate.read().await;
            let status = self.inner.account_status().await;
            if self.inner.ready_without_mutation(&status) {
                return Ok(status);
            }
        }
        // Materialization is rare and must exclude turns. Re-check under the writer: another
        // caller may have completed it in between.
        let _write = self.inner.gate.write().await;
        self.inner.ensure_ready_unlocked().await
    }

    /// Enters a turn: ready credentials, held unchanged until the guard is dropped.
    pub async fn turn(&self) -> Result<CodexTurnGuard> {
        self.ensure_ready().await?;
        let read = self.inner.gate.read().await;
        let status = self.inner.account_status().await;
        if !status.configured {
            return Err(missing_auth());
        }
        let chatgpt_turn = if status.active_is("chatgpt") {
            Some(Arc::clone(&self.inner.chatgpt_turn).lock_owned().await)
        } else {
            None
        };
        Ok(CodexTurnGuard { status, _chatgpt_turn: chatgpt_turn, _read: read })
    }

    /// Materializes the server's API key or access token now.
    pub async fn login_from_environment(&self, method: &str) -> Result<CodexAuthStatus> {
        let method = method.trim().to_lowercase();
        if method != "api-key" && method != "access-token" {
            return Err(LiminaError::invariant(
                "Runtime login method must be api-key or access-token.",
            ));
        }
        if !self.inner.gate.try_begin_write() {
            return Err(busy(self.active_turns()));
        }
        let outcome = async {
            if method == "api-key" {
                self.inner.login_api_key().await?;
            } else {
                self.inner.login_access_token().await?;
            }
            Ok(self.inner.account_status().await)
        }
        .await;
        self.inner.gate.end_write();
        outcome
    }

    /// Starts a ChatGPT device-code login. It holds the auth writer until it completes, fails or
    /// is cancelled. Replaying a command id returns the same attempt.
    pub async fn start_device_login(&self, command_id: &str) -> Result<DeviceLoginAttempt> {
        if self.inner.mode == "api-key" || self.inner.mode == "access-token" {
            return Err(LiminaError::invariant(
                "ChatGPT login is disabled by LIMINA_CODEX_AUTH_MODE.",
            )
            .with("configured_mode", self.inner.mode.as_str())
            .with(
                "suggestion",
                "Use auto/chatgpt mode or log in with the configured server credential.",
            ));
        }
        {
            let attempts = self.inner.attempts();
            if let Some(attempt) =
                attempts.by_command.get(command_id).and_then(|id| attempts.by_id.get(id))
            {
                return Ok(attempt.public());
            }
        }
        if !self.inner.gate.try_begin_write() {
            return Err(busy(self.active_turns()));
        }
        let started = async {
            let environment = self.inner.auth_environment().await.map_err(internal)?;
            let client = self.inner.connector.connect(environment).await.map_err(internal)?;
            match client.login_chatgpt_device_code().await {
                Ok(code) => Ok((client, code)),
                Err(error) => {
                    client.close().await;
                    Err(internal(error))
                }
            }
        }
        .await;
        let (client, code) = match started {
            Ok(started) => started,
            Err(error) => {
                self.inner.gate.end_write();
                return Err(error);
            }
        };
        let login_id = code.login_id.clone();
        let public = {
            let mut attempts = self.inner.attempts();
            attempts.prune();
            let attempt = Attempt {
                code,
                command_id: command_id.to_owned(),
                status: LoginStatus::Pending,
                error: None,
                created_at: now(),
                completed_at: None,
                client: Some(Arc::clone(&client)),
                cancel_requested: false,
            };
            let public = attempt.public();
            attempts.by_id.insert(login_id.clone(), attempt);
            attempts.by_command.insert(command_id.to_owned(), login_id.clone());
            public
        };
        tokio::spawn(wait_for_device_login(Arc::clone(&self.inner), login_id, client));
        Ok(public)
    }

    pub fn login_attempt(&self, login_id: &str) -> Result<DeviceLoginAttempt> {
        self.inner
            .attempts()
            .by_id
            .get(login_id)
            .map(Attempt::public)
            .ok_or_else(|| missing_attempt(login_id))
    }

    pub async fn cancel_device_login(&self, login_id: &str) -> Result<DeviceLoginAttempt> {
        let client = {
            let mut attempts = self.inner.attempts();
            let attempt =
                attempts.by_id.get_mut(login_id).ok_or_else(|| missing_attempt(login_id))?;
            if attempt.status != LoginStatus::Pending {
                return Ok(attempt.public());
            }
            attempt.cancel_requested = true;
            attempt.client.clone()
        };
        let cancelled = match client {
            Some(client) => client.cancel_login(login_id).await,
            None => Ok(()),
        };
        // Finish outside the registry lock, whatever the provider said.
        self.inner.finish_attempt(login_id, LoginStatus::Cancelled, None).await;
        cancelled.map_err(internal)?;
        self.login_attempt(login_id)
    }

    pub async fn logout(&self) -> Result<CodexAuthStatus> {
        if !self.inner.gate.try_begin_write() {
            return Err(busy(self.active_turns()));
        }
        let outcome = async {
            let environment = self.inner.auth_environment().await.map_err(internal)?;
            let client = self.inner.connector.connect(environment).await.map_err(internal)?;
            let logged_out = client.logout().await;
            client.close().await;
            logged_out.map_err(internal)?;
            self.inner.materialized.store(false, Ordering::SeqCst);
            secure_state_files(&self.inner.home).await.map_err(internal)?;
            Ok(self.inner.account_status().await)
        }
        .await;
        self.inner.gate.end_write();
        outcome
    }

    /// Cancels every pending device login.
    pub async fn close(&self) {
        let pending: Vec<String> = self
            .inner
            .attempts()
            .by_id
            .values()
            .filter(|attempt| attempt.status == LoginStatus::Pending)
            .map(|attempt| attempt.code.login_id.clone())
            .collect();
        for login_id in pending {
            if let Err(error) = self.cancel_device_login(&login_id).await {
                tracing::warn!(target: LOG_TARGET, %error, "could not cancel a Codex login");
            }
        }
    }
}

async fn wait_for_device_login(
    inner: Arc<Inner>,
    login_id: String,
    client: Arc<dyn CodexAuthClient>,
) {
    let succeeded = match client.wait_login(&login_id).await {
        Ok(completion) if completion.success => secure_state_files(&inner.home).await.is_ok(),
        _ => false,
    };
    if succeeded {
        inner.finish_attempt(&login_id, LoginStatus::Succeeded, None).await;
        return;
    }
    // A cancellation in progress records CANCELLED itself; the failed wait is its echo.
    if inner.attempts().by_id.get(&login_id).is_some_and(|attempt| attempt.cancel_requested) {
        return;
    }
    inner.finish_attempt(&login_id, LoginStatus::Failed, Some(LOGIN_FAILED)).await;
}

impl Inner {
    fn attempts(&self) -> MutexGuard<'_, Attempts> {
        self.attempts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn finish_attempt(&self, login_id: &str, status: LoginStatus, error: Option<&str>) {
        let client = {
            let mut attempts = self.attempts();
            let Some(attempt) = attempts.by_id.get_mut(login_id) else { return };
            if attempt.status != LoginStatus::Pending {
                return;
            }
            attempt.status = status;
            attempt.error = error.map(str::to_owned);
            attempt.completed_at = Some(now());
            attempt.client.take()
        };
        if let Some(client) = client {
            client.close().await;
        }
        // A provider cleanup problem must never strand the instance-wide auth writer.
        self.gate.end_write();
    }

    async fn auth_environment(&self) -> io::Result<BTreeMap<String, String>> {
        let home = self.home.clone();
        blocking(move || codex_environment(&process_environment(), &BTreeMap::new(), &home)).await
    }

    async fn account_status(&self) -> CodexAuthStatus {
        let account = async {
            let environment = self.auth_environment().await.ok()?;
            let client = self.connector.connect(environment).await.ok()?;
            let account = client.account().await;
            client.close().await;
            account.ok()
        }
        .await;
        match account {
            Some(account) => self.status_from(account.as_ref()),
            None => CodexAuthStatus {
                engine: "codex".to_owned(),
                configured_mode: self.mode.clone(),
                configured: false,
                active_method: None,
                account_email: None,
                account_plan: None,
                source: "unavailable".to_owned(),
                error: Some("Codex account status is unavailable.".to_owned()),
                single_runtime_node: true,
            },
        }
    }

    fn status_from(&self, account: Option<&CodexAccount>) -> CodexAuthStatus {
        let active_method = match account.map(|account| account.account_type.as_str()) {
            Some("chatgpt") => Some("chatgpt"),
            Some("apiKey") => Some("api-key"),
            _ => None,
        };
        let materialized = self.materialized.load(Ordering::SeqCst);
        let source = match active_method {
            Some("chatgpt") if !materialized => "cached_chatgpt",
            Some("chatgpt") if self.access_token.is_some() => "server_access_token",
            Some("api-key") if self.api_key.is_some() => "server_api_key",
            Some("api-key") => "cached_api_key",
            _ => "none",
        };
        CodexAuthStatus {
            engine: "codex".to_owned(),
            configured_mode: self.mode.clone(),
            configured: active_method.is_some(),
            active_method: active_method.map(str::to_owned),
            account_email: account.and_then(|account| account.email.clone()),
            account_plan: account.and_then(|account| account.plan_type.clone()),
            source: source.to_owned(),
            error: None,
            single_runtime_node: true,
        }
    }

    fn ready_without_mutation(&self, status: &CodexAuthStatus) -> bool {
        let materialized = self.materialized.load(Ordering::SeqCst);
        match self.mode.as_str() {
            "chatgpt" => status.active_is("chatgpt"),
            "api-key" | "access-token" => materialized && status.configured,
            _ => {
                (status.active_is("chatgpt") && status.source == "cached_chatgpt")
                    || (materialized && status.configured)
                    || (self.api_key.is_none() && self.access_token.is_none() && status.configured)
            }
        }
    }

    async fn ensure_ready_unlocked(&self) -> Result<CodexAuthStatus> {
        let status = self.account_status().await;
        let materialized = self.materialized.load(Ordering::SeqCst);
        match self.mode.as_str() {
            "chatgpt" => {
                return if status.active_is("chatgpt") { Ok(status) } else { Err(missing_auth()) };
            }
            "api-key" => {
                if !materialized {
                    self.login_api_key().await?;
                }
                return Ok(self.account_status().await);
            }
            "access-token" => {
                if !materialized {
                    self.login_access_token().await?;
                }
                return Ok(self.account_status().await);
            }
            _ => {}
        }
        // Auto keeps a human ChatGPT session. Machine credentials are re-materialized once per
        // server process, so rotation replaces stale cached API-key or access-token state.
        if status.active_is("chatgpt") && status.source == "cached_chatgpt" {
            return Ok(status);
        }
        if self.access_token.is_some() {
            if !materialized {
                self.login_access_token().await?;
            }
            return Ok(self.account_status().await);
        }
        if self.api_key.is_some() {
            if !materialized {
                self.login_api_key().await?;
            }
            return Ok(self.account_status().await);
        }
        if status.configured {
            return Ok(status);
        }
        Err(missing_auth())
    }

    async fn login_api_key(&self) -> Result<()> {
        let Some(api_key) = self.api_key.as_deref() else {
            return Err(LiminaError::authentication_with(
                "Codex API-key auth requires OPENAI_API_KEY on the Limina server.",
            ));
        };
        let logged_in = async {
            let environment = self.auth_environment().await.ok()?;
            let client = self.connector.connect(environment).await.ok()?;
            let outcome = client.login_api_key(api_key).await;
            client.close().await;
            outcome.ok()
        }
        .await;
        if logged_in.is_none() {
            return Err(LiminaError::transport(
                "Limina could not materialize Codex API-key authentication.",
            )
            .with("reason", "The Codex SDK rejected or could not persist the server credential."));
        }
        self.materialized.store(true, Ordering::SeqCst);
        secure_state_files(&self.home).await.map_err(internal)
    }

    async fn login_access_token(&self) -> Result<()> {
        let Some(token) = self.access_token.clone() else {
            return Err(LiminaError::authentication_with(
                "Codex access-token auth requires CODEX_ACCESS_TOKEN on the Limina server.",
            ));
        };
        let could_not_start =
            || LiminaError::transport("Limina could not start Codex access-token login.");
        let environment = self.auth_environment().await.map_err(|_| could_not_start())?;
        let run = run_with_input(&self.login_command, environment, token);
        match tokio::time::timeout(ACCESS_TOKEN_LOGIN_TIMEOUT, run).await {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                return Err(LiminaError::transport(
                    "Limina could not materialize Codex access-token authentication.",
                )
                .with("reason", "Codex login rejected the configured access token."));
            }
            Ok(Err(_)) | Err(_) => return Err(could_not_start()),
        }
        self.materialized.store(true, Ordering::SeqCst);
        secure_state_files(&self.home).await.map_err(internal)
    }
}

/// Runs a command with `input` on stdin (then closed), discarding its output. True on success.
async fn run_with_input(
    command: &[String],
    environment: BTreeMap<String, String>,
    input: String,
) -> io::Result<bool> {
    let (program, args) =
        command.split_first().ok_or_else(|| io::Error::other("the login command is empty"))?;
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .env_clear()
        .envs(environment)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await?;
    }
    let output = child.wait_with_output().await?;
    Ok(output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_labels_join_email_and_plan() {
        let mut status = CodexAuthStatus {
            engine: "codex".into(),
            configured_mode: "chatgpt".into(),
            configured: true,
            active_method: Some("chatgpt".into()),
            account_email: Some("olivia@example.test".into()),
            account_plan: Some("pro".into()),
            source: "cached_chatgpt".into(),
            error: None,
            single_runtime_node: true,
        };
        assert_eq!(status.account_label().as_deref(), Some("olivia@example.test · pro"));
        status.account_email = None;
        assert_eq!(status.account_label().as_deref(), Some("pro"));
        status.account_plan = None;
        assert_eq!(status.account_label(), None);
    }

    #[tokio::test]
    async fn invalid_modes_and_conflicting_credentials_are_rejected() {
        let home = tempfile::tempdir().unwrap();
        let options = CodexAuthOptions { mode: "magic".into(), ..CodexAuthOptions::default() };
        let error = CodexAuthManager::new(home.path().join("a"), options).await.err().unwrap();
        assert_eq!(
            error.message,
            "LIMINA_CODEX_AUTH_MODE must be one of ['access-token', 'api-key', 'auto', 'chatgpt']."
        );
        let options = CodexAuthOptions {
            api_key: Some("k".into()),
            access_token: Some("t".into()),
            ..CodexAuthOptions::default()
        };
        let error = CodexAuthManager::new(home.path().join("b"), options).await.err().unwrap();
        assert!(error.message.contains("cannot both be set"));
    }
}
