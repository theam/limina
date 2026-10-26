//! A Claude project session over the Claude CLI's streaming protocol.
//!
//! Each turn starts the CLI in the project workspace with Limina's options (the `claude_code`
//! tool and prompt presets, an enforced sandbox that cannot read Limina's own state, no
//! filesystem setting sources, the checkpoint JSON schema, `can_use_tool` answered by Limina),
//! resumes the project's session, and streams messages into transcript events. Steering
//! interrupts the running query and continues the same checkpoint with the guidance. The turn's
//! connected MCP servers go in a private `--mcp-config` file with `--strict-mcp-config` (see
//! [`crate::mcp`]).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use limina_core::{LiminaError, Result, internal};
use limina_domain::environment::{claude_environment, process_environment};
use serde_json::{Value, json};
use tokio::sync::watch;

use super::connection::{CLIENT_APP, ClaudeConnection, ClaudeIncoming, ClaudeLaunch, LOG_TARGET};
use super::permissions::{ToolPolicy, resolve_lenient};
use crate::error::ProviderError;
use crate::events::{ClaudeToolNames, map_claude_message};
use crate::mcp::{CLAUDE_MCP_CONFIG_FILE, McpServer, claude_mcp_config};
use crate::process::blocking;
use crate::redaction::redact_text;
use crate::session::{AgentSession, TurnRequest};
use crate::text::{py_str, truthy};
use crate::turn::{RuntimeTurn, parse_runtime_decision, turn_output_schema};
use crate::usage::claude_result_usage;

/// Keeps resumable sessions well beyond the CLI's interactive default.
pub const SETTINGS_FILE: &str = "limina-settings.json";
const CLEANUP_PERIOD_DAYS: u32 = 3650;
const RESULT_FIELDS: [&str; 6] =
    ["subtype", "duration_ms", "duration_api_ms", "is_error", "num_turns", "session_id"];

#[derive(Debug, Clone)]
pub struct ClaudeSessionConfig {
    /// `LIMINA_CLAUDE_BIN`, default `claude`.
    pub claude_bin: String,
    /// The project's private Claude configuration directory (`CLAUDE_CONFIG_DIR`).
    pub config_dir: PathBuf,
    /// Limina's state root; the sandbox denies reads of it.
    pub state_root: PathBuf,
    /// `LIMINA_CLAUDE_MODEL`; the CLI default when `None`.
    pub model: Option<String>,
    /// `LIMINA_CLAUDE_WEAKER_NESTED_SANDBOX`, for runtimes that are themselves containers.
    pub weaker_nested_sandbox: bool,
}

impl ClaudeSessionConfig {
    pub fn new(config_dir: impl Into<PathBuf>, state_root: impl Into<PathBuf>) -> Self {
        Self {
            claude_bin: "claude".to_owned(),
            config_dir: config_dir.into(),
            state_root: state_root.into(),
            model: None,
            weaker_nested_sandbox: false,
        }
    }
}

#[derive(Default)]
struct State {
    connection: Option<Arc<ClaudeConnection>>,
    /// A turn is in progress (from before connecting until cleanup).
    active: bool,
    /// A query is streaming, so steering can redirect it.
    query_active: bool,
    stop_requested: bool,
    steering: VecDeque<String>,
}

pub struct ClaudeAgentSession {
    config: ClaudeSessionConfig,
    state: Mutex<State>,
    /// Steering interrupts not yet acknowledged; the next query waits for them so a late
    /// interrupt never cancels the guidance it carried.
    steering_interrupts: watch::Sender<usize>,
}

enum Failure {
    Limina(LiminaError),
    Provider(ProviderError),
}

impl From<LiminaError> for Failure {
    fn from(error: LiminaError) -> Self {
        Self::Limina(error)
    }
}

/// Writes the settings file, private to the config directory, only when it changed.
fn write_settings(config_dir: &Path) -> std::io::Result<PathBuf> {
    limina_domain::environment::ensure_private_directory(config_dir)?;
    let path = config_dir.join(SETTINGS_FILE);
    let content = format!("{{\"cleanupPeriodDays\": {CLEANUP_PERIOD_DAYS}}}\n");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
        std::fs::write(&path, &content)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

/// The settings passed with `--settings`: the settings file merged with the sandbox, as the SDK
/// merges `settings` and `sandbox`.
pub fn claude_settings(
    workspace_root: &Path,
    protected_roots: &[String],
    weaker_nested_sandbox: bool,
) -> Value {
    json!({
        "cleanupPeriodDays": CLEANUP_PERIOD_DAYS,
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "autoAllowBashIfSandboxed": true,
            "allowUnsandboxedCommands": false,
            "enableWeakerNestedSandbox": weaker_nested_sandbox,
            "filesystem": {
                "denyRead": protected_roots,
                "allowRead": [workspace_root.to_string_lossy()],
            },
        },
    })
}

/// Writes the turn's MCP servers to the private configuration file Claude reads with
/// `--mcp-config` (mode `0600`, replaced atomically), or removes a previous turn's file when the
/// project uses none. Returns the file to pass.
pub fn write_mcp_config(
    config_dir: &Path,
    servers: &[McpServer],
) -> std::io::Result<Option<PathBuf>> {
    let path = config_dir.join(CLAUDE_MCP_CONFIG_FILE);
    if servers.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        };
    }
    limina_domain::environment::ensure_private_directory(config_dir)?;
    let staging = config_dir.join(format!("{CLAUDE_MCP_CONFIG_FILE}.tmp"));
    let _ = std::fs::remove_file(&staging);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write as _;
        let mut file = options.open(&staging)?;
        file.write_all(claude_mcp_config(servers).to_string().as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&staging, &path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(Some(path))
}

/// The CLI arguments the SDK builds for Limina's options, in the SDK's order.
pub fn claude_arguments(
    model: Option<&str>,
    resume: Option<&str>,
    settings: &Value,
) -> Vec<String> {
    claude_launch_arguments(model, resume, settings, None)
}

/// [`claude_arguments`] plus the turn's MCP configuration file, when it has one: only the
/// servers in that file apply (`--strict-mcp-config`).
pub fn claude_launch_arguments(
    model: Option<&str>,
    resume: Option<&str>,
    settings: &Value,
    mcp_config: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> =
        ["--output-format", "stream-json", "--verbose", "--tools", "default"]
            .map(str::to_owned)
            .into();
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    // A `can_use_tool` callback makes the CLI ask the host over stdio.
    args.extend(
        ["--permission-prompt-tool", "stdio", "--permission-mode", "default"].map(str::to_owned),
    );
    if let Some(resume) = resume.filter(|resume| !resume.is_empty()) {
        args.extend(["--resume".to_owned(), resume.to_owned()]);
    }
    args.extend([
        "--settings".to_owned(),
        settings.to_string(),
        // No user, project or local settings files: the session sees only Limina's settings.
        "--setting-sources=".to_owned(),
    ]);
    if let Some(path) = mcp_config {
        args.extend([
            "--mcp-config".to_owned(),
            path.to_string_lossy().into_owned(),
            "--strict-mcp-config".to_owned(),
        ]);
    }
    args.extend([
        "--json-schema".to_owned(),
        turn_output_schema().to_string(),
        "--input-format".to_owned(),
        "stream-json".to_owned(),
    ]);
    args
}

/// The steering prompt that continues an interrupted query.
fn steering_prompt(steering: &[String]) -> String {
    format!(
        "Human steering arrived while you were working. Incorporate it now and continue the same \
         Limina checkpoint:\n\n- {}",
        steering.join("\n- ")
    )
}

/// The session a message reports. User messages carry none in the SDK.
fn message_session_id(message: &Value) -> Option<String> {
    match message.get("type").and_then(Value::as_str) {
        Some("assistant" | "system" | "result" | "stream_event" | "rate_limit_event") => {
            message.get("session_id").filter(|id| truthy(id)).map(py_str)
        }
        _ => None,
    }
}

struct Prepared {
    launch: ClaudeLaunch,
    policy: Arc<ToolPolicy>,
}

impl ClaudeAgentSession {
    pub fn new(config: ClaudeSessionConfig) -> Self {
        Self {
            config,
            state: Mutex::new(State::default()),
            steering_interrupts: watch::Sender::new(0),
        }
    }

    pub fn config(&self) -> &ClaudeSessionConfig {
        &self.config
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn prepare(&self, request: &TurnRequest) -> std::io::Result<Prepared> {
        let config_dir = self.config.config_dir.clone();
        let state_root = self.config.state_root.clone();
        let workspace = request.workspace.clone();
        let overlay = request.runtime_env.clone();
        let servers = request.mcp_servers.clone();
        let (workspace_root, protected_roots, mut env, mcp_config) = blocking(move || {
            write_settings(&config_dir)?;
            let mcp_config = write_mcp_config(&config_dir, &servers)?;
            let workspace_root = resolve_lenient(&workspace);
            let mut protected_roots: Vec<String> = [
                resolve_lenient(&state_root),
                resolve_lenient(config_dir.parent().unwrap_or(&config_dir)),
            ]
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
            protected_roots.sort();
            protected_roots.dedup();
            let env = claude_environment(&process_environment(), &overlay, &config_dir)?;
            Ok((workspace_root, protected_roots, env, mcp_config))
        })
        .await?;
        env.insert("CLAUDE_AGENT_SDK_CLIENT_APP".to_owned(), CLIENT_APP.to_owned());
        env.insert("PWD".to_owned(), request.workspace.to_string_lossy().into_owned());
        let settings =
            claude_settings(&workspace_root, &protected_roots, self.config.weaker_nested_sandbox);
        let mut args = claude_launch_arguments(
            Some(
                request
                    .settings
                    .model
                    .as_deref()
                    .or(self.config.model.as_deref())
                    .unwrap_or("default"),
            ),
            request.continuation_id.as_deref(),
            &settings,
            mcp_config.as_deref(),
        );
        if let Some(effort) = &request.settings.reasoning_effort {
            args.extend(["--effort".to_owned(), effort.clone()]);
        }
        // Explicit defaults clear a resumed session's previous model/effort overrides.
        env.insert(
            "CLAUDE_CODE_EFFORT_LEVEL".to_owned(),
            request.settings.reasoning_effort.clone().unwrap_or_else(|| "auto".to_owned()),
        );
        let servers = request.mcp_servers.iter().map(|server| server.name.clone());
        Ok(Prepared {
            launch: ClaudeLaunch {
                binary: self.config.claude_bin.clone(),
                args,
                env,
                cwd: request.workspace.clone(),
            },
            policy: Arc::new(ToolPolicy::new(workspace_root).with_mcp_servers(servers)),
        })
    }

    async fn drive(
        &self,
        request: &TurnRequest,
        prepared: Prepared,
    ) -> std::result::Result<RuntimeTurn, Failure> {
        let connection = Arc::new(
            ClaudeConnection::connect(&prepared.launch, prepared.policy)
                .await
                .map_err(Failure::Provider)?,
        );
        self.state().connection = Some(Arc::clone(&connection));
        let mut tools = ClaudeToolNames::new();
        let mut next_prompt = request.prompt.clone();
        loop {
            {
                let mut state = self.state();
                if state.stop_requested {
                    return Err(interrupted().into());
                }
                state.query_active = true;
            }
            let streamed = self.stream_query(&connection, &next_prompt, request, &mut tools).await;
            let (steering, stop_requested) = {
                let mut state = self.state();
                state.query_active = false;
                (state.steering.drain(..).collect::<Vec<_>>(), state.stop_requested)
            };
            let streamed = streamed?;
            if !steering.is_empty() && !stop_requested {
                // Let the interrupt that carried the guidance settle before the next query.
                let mut pending = self.steering_interrupts.subscribe();
                let _ = pending.wait_for(|count| *count == 0).await;
                next_prompt = steering_prompt(&steering);
                continue;
            }
            if stop_requested {
                return Err(interrupted().into());
            }
            return finish_turn(streamed).map_err(Failure::Limina);
        }
    }

    /// Sends one query and reads until its result (or the end of the stream).
    async fn stream_query(
        &self,
        connection: &ClaudeConnection,
        prompt: &str,
        request: &TurnRequest,
        tools: &mut ClaudeToolNames,
    ) -> std::result::Result<Streamed, Failure> {
        connection.query(prompt).await.map_err(Failure::Provider)?;
        let mut streamed = Streamed::default();
        loop {
            let message = match connection.next().await {
                ClaudeIncoming::Message(message) => message,
                ClaudeIncoming::End => return Ok(streamed),
                ClaudeIncoming::Error(text) => {
                    return Err(Failure::Provider(ProviderError::Closed(text)));
                }
            };
            let message_type = message.get("type").and_then(Value::as_str).unwrap_or("");
            if message_type == "result"
                && let Some(missing) =
                    RESULT_FIELDS.iter().find(|field| message.get(**field).is_none())
            {
                return Err(Failure::Provider(ProviderError::Protocol(format!(
                    "Missing required field in result message: '{missing}'"
                ))));
            }
            if let Some(session_id) = message_session_id(&message) {
                request.observer.continuation(&session_id).await?;
            }
            if message_type == "assistant"
                && let Some(id) = message.pointer("/message/id").filter(|id| truthy(id))
            {
                streamed.turn_id = Some(py_str(id));
            }
            for event in map_claude_message(&message, tools) {
                request.observer.event(event).await?;
            }
            if message_type == "result" {
                streamed.result = Some(message);
                return Ok(streamed);
            }
        }
    }
}

#[derive(Default)]
struct Streamed {
    turn_id: Option<String>,
    result: Option<Value>,
}

fn interrupted() -> LiminaError {
    LiminaError::transport("The managed Claude turn was interrupted before checkpointing.")
}

fn finish_turn(streamed: Streamed) -> Result<RuntimeTurn> {
    let Some(result) = streamed.result else {
        return Err(LiminaError::transport(
            "Claude completed without returning a terminal result.",
        ));
    };
    if result.get("is_error").is_some_and(truthy) {
        return Err(LiminaError::transport(
            "The managed Claude turn failed before Limina accepted its checkpoint.",
        )
        .with("stop_reason", result.get("stop_reason").cloned().unwrap_or(Value::Null)));
    }
    // `structured_output or result`
    let response = result
        .get("structured_output")
        .filter(|output| truthy(output))
        .or_else(|| result.get("result"))
        .filter(|response| !response.is_null());
    let Some(response) = response else {
        return Err(LiminaError::transport(
            "Claude completed without the required project decision.",
        ));
    };
    let decision = parse_runtime_decision(response, "Claude")?;
    Ok(RuntimeTurn {
        continuation_id: result.get("session_id").map(py_str),
        turn_id: streamed.turn_id.unwrap_or_else(limina_core::new_uuid),
        decision,
        usage: claude_result_usage(&result),
    })
}

/// Resets the session state when a turn ends, even if its future is dropped, and ends the CLI
/// session in the background in that case.
struct TurnGuard<'a> {
    session: &'a ClaudeAgentSession,
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.session.state();
        state.active = false;
        state.query_active = false;
        state.connection.take();
    }
}

#[async_trait]
impl AgentSession for ClaudeAgentSession {
    async fn models(
        &self,
        runtime_env: &std::collections::BTreeMap<String, String>,
    ) -> Result<crate::models::ModelCatalog> {
        let models = crate::models::claude_models(
            &self.config.claude_bin,
            &self.config.config_dir,
            runtime_env,
        )
        .await?;
        Ok(crate::models::ModelCatalog {
            default_model: self.config.model.clone().or_else(|| Some("default".to_owned())),
            models,
        })
    }

    async fn run_turn(&self, request: TurnRequest) -> Result<RuntimeTurn> {
        let prepared = self.prepare(&request).await.map_err(internal)?;
        {
            let mut state = self.state();
            state.active = true;
            state.stop_requested = false;
        }
        let guard = TurnGuard { session: self };
        let outcome = self.drive(&request, prepared).await;
        let connection = self.state().connection.clone();
        drop(guard);
        if let Some(connection) = connection {
            connection.disconnect().await;
        }
        match outcome {
            Ok(turn) => Ok(turn),
            Err(Failure::Limina(error)) => Err(error),
            Err(Failure::Provider(error)) => {
                let secrets = request.redactions();
                Err(LiminaError::transport(
                    "The managed Claude turn failed before Limina accepted its checkpoint.",
                )
                .with("reason", redact_text(&error.to_string(), &secrets))
                .with("continuation_id", request.continuation_id.clone()))
            }
        }
    }

    async fn steer(&self, message: &str) -> Result<bool> {
        let connection = {
            let mut state = self.state();
            let Some(connection) = state.connection.clone() else { return Ok(false) };
            if !state.query_active || state.stop_requested {
                return Ok(false);
            }
            state.steering.push_back(message.to_owned());
            self.steering_interrupts.send_modify(|count| *count += 1);
            connection
        };
        if let Err(error) = connection.interrupt().await {
            tracing::debug!(target: LOG_TARGET, %error, "steering interrupt failed");
        }
        self.steering_interrupts.send_modify(|count| *count = count.saturating_sub(1));
        Ok(true)
    }

    async fn interrupt(&self) -> Result<bool> {
        let connection = {
            let mut state = self.state();
            if !state.active {
                return Ok(false);
            }
            state.stop_requested = true;
            state.connection.clone()
        };
        // Before the CLI is connected there is nothing to signal: the turn stops right after
        // connecting.
        if let Some(connection) = connection {
            connection.interrupt().await.map_err(internal)?;
        }
        Ok(true)
    }

    async fn close(&self) {
        if let Err(error) = self.interrupt().await {
            tracing::debug!(target: LOG_TARGET, %error, "interrupt on close failed");
        }
        let connection = self.state().connection.clone();
        if let Some(connection) = connection {
            connection.disconnect().await;
        }
    }

    fn model(&self) -> Option<String> {
        self.config.model.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_mirror_the_sdk_command_line() {
        let settings = claude_settings(Path::new("/w"), &["/state".to_owned()], false);
        let args = claude_arguments(Some("claude-test"), Some("previous"), &settings);
        let position = |flag: &str| args.iter().position(|arg| arg == flag).unwrap();
        assert_eq!(
            &args[..5],
            ["--output-format", "stream-json", "--verbose", "--tools", "default"]
        );
        assert_eq!(args[position("--model") + 1], "claude-test");
        assert_eq!(args[position("--permission-prompt-tool") + 1], "stdio");
        assert_eq!(args[position("--permission-mode") + 1], "default");
        assert_eq!(args[position("--resume") + 1], "previous");
        assert!(args.contains(&"--setting-sources=".to_owned()));
        let schema: Value = serde_json::from_str(&args[position("--json-schema") + 1]).unwrap();
        assert_eq!(schema, turn_output_schema());
        assert_eq!(&args[args.len() - 2..], ["--input-format", "stream-json"]);
        let without = claude_arguments(None, None, &settings);
        assert!(!without.contains(&"--model".to_owned()));
        assert!(!without.contains(&"--resume".to_owned()));
        assert!(!without.contains(&"--mcp-config".to_owned()));
        let with_mcp =
            claude_launch_arguments(None, None, &settings, Some(Path::new("/c/limina-mcp.json")));
        let at = with_mcp.iter().position(|arg| arg == "--mcp-config").unwrap();
        assert_eq!(with_mcp[at + 1..at + 3], ["/c/limina-mcp.json", "--strict-mcp-config"]);
        assert_eq!(&with_mcp[with_mcp.len() - 2..], ["--input-format", "stream-json"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_mcp_config_file_is_private_and_removed_when_unused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut server = McpServer::new("notes", "https://mcp.example.test/mcp");
        server.bearer_token = Some("tok-123".into());
        let path = write_mcp_config(dir.path(), &[server]).unwrap().unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["mcpServers"]["notes"]["headers"]["Authorization"], "Bearer tok-123");
        assert_eq!(write_mcp_config(dir.path(), &[]).unwrap(), None);
        assert!(!path.exists());
    }

    #[test]
    fn settings_merge_the_sandbox() {
        let settings = claude_settings(Path::new("/w"), &["/a".to_owned(), "/b".to_owned()], true);
        assert_eq!(settings["cleanupPeriodDays"], 3650);
        assert_eq!(settings["sandbox"]["enableWeakerNestedSandbox"], true);
        assert_eq!(settings["sandbox"]["filesystem"]["allowRead"], json!(["/w"]));
        assert_eq!(settings["sandbox"]["filesystem"]["denyRead"], json!(["/a", "/b"]));
    }

    #[test]
    fn only_non_user_messages_report_sessions() {
        assert_eq!(message_session_id(&json!({"type": "user", "session_id": "s"})), None);
        assert_eq!(
            message_session_id(&json!({"type": "system", "session_id": "s"})).as_deref(),
            Some("s")
        );
        assert_eq!(message_session_id(&json!({"type": "assistant", "session_id": ""})), None);
    }

    #[test]
    fn steering_continues_the_same_checkpoint() {
        let prompt = steering_prompt(&["a: one".to_owned(), "b: two".to_owned()]);
        assert!(prompt.ends_with("checkpoint:\n\n- a: one\n- b: two"));
    }
}
