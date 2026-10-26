//! The Claude CLI in streaming mode, spoken the way the official Python SDK
//! (`claude_agent_sdk` 0.2.118: `subprocess_cli.py`, `_internal/query.py`, `client.py`) speaks it:
//!
//! - `claude --output-format stream-json --verbose … --input-format stream-json`, one JSON
//!   object per line in both directions;
//! - an `initialize` control request before the first prompt;
//! - prompts as `{"type": "user", …}` messages;
//! - `can_use_tool` control requests answered by Limina's [`ToolPolicy`];
//! - `interrupt` as a control request;
//! - everything else forwarded to the turn in order.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::ChildStdout;
use tokio::sync::{mpsc, oneshot};

use super::permissions::ToolPolicy;
use crate::error::ProviderError;
use crate::process::{
    AbortOnDrop, LineReader, MAX_LINE_BYTES, ProcessSpec, ProviderProcess, STDERR_DRAIN,
    spawn_failure,
};
use crate::text::{clip, py_str, truthy};

pub(crate) const LOG_TARGET: &str = "limina_engines::claude";
/// How Limina identifies itself to the Claude CLI (`CLAUDE_AGENT_SDK_CLIENT_APP`). The entrypoint
/// is left to the CLI: Limina speaks the SDK protocol but is not one of the official SDKs.
pub const CLIENT_APP: &str = concat!("limina/", env!("CARGO_PKG_VERSION"));
/// After stdin closes, the CLI gets this long to flush its session file before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(60);
/// Waiting for the exit code once stdout closes.
const EXIT_WAIT: Duration = Duration::from_secs(5);

/// How to start one Claude CLI session.
#[derive(Debug, Clone)]
pub(crate) struct ClaudeLaunch {
    pub binary: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
}

/// What the reader forwards to the turn.
#[derive(Debug, Clone)]
pub(crate) enum ClaudeIncoming {
    Message(Value),
    /// Stdout closed after a clean exit.
    End,
    /// Stdout closed after a failure.
    Error(String),
}

#[derive(Default)]
struct Control {
    pending: HashMap<String, oneshot::Sender<Result<Value, String>>>,
    closed: Option<String>,
    counter: u64,
}

type SharedControl = Arc<Mutex<Control>>;

fn lock(control: &SharedControl) -> std::sync::MutexGuard<'_, Control> {
    control.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct ClaudeConnection {
    process: Arc<ProviderProcess>,
    control: SharedControl,
    messages: tokio::sync::Mutex<mpsc::UnboundedReceiver<ClaudeIncoming>>,
    _reader: AbortOnDrop,
    initialization: Value,
}

/// `initialize` waits at least a minute, longer if `CLAUDE_CODE_STREAM_CLOSE_TIMEOUT` (ms) says so.
fn initialize_timeout() -> Duration {
    let configured = std::env::var("CLAUDE_CODE_STREAM_CLOSE_TIMEOUT")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or(CONTROL_TIMEOUT, Duration::from_millis);
    configured.max(CONTROL_TIMEOUT)
}

impl ClaudeConnection {
    /// Starts the CLI and completes the `initialize` handshake.
    pub(crate) async fn connect(
        launch: &ClaudeLaunch,
        policy: Arc<ToolPolicy>,
    ) -> Result<Self, ProviderError> {
        let spec = ProcessSpec {
            label: "claude",
            program: &launch.binary,
            args: &launch.args,
            env: &launch.env,
            cwd: Some(&launch.cwd),
        };
        let (process, stdout) = match ProviderProcess::spawn(&spec) {
            Ok(spawned) => spawned,
            Err(error) => {
                let reason = match spawn_failure(Some(&launch.cwd), &error).await {
                    Some(reason) => reason,
                    None if error.kind() == std::io::ErrorKind::NotFound => {
                        format!("Claude was not found at: {} ({error})", launch.binary)
                    }
                    None => format!("Failed to start Claude: {error}"),
                };
                return Err(ProviderError::Spawn(reason));
            }
        };
        let process = Arc::new(process);
        let control = SharedControl::default();
        let (sender, receiver) = mpsc::unbounded_channel();
        let reader = AbortOnDrop::spawn(read_loop(
            stdout,
            Arc::clone(&process),
            Arc::clone(&control),
            policy,
            sender,
        ));
        let mut connection = Self {
            process,
            control,
            messages: tokio::sync::Mutex::new(receiver),
            _reader: reader,
            initialization: Value::Null,
        };
        connection.initialization = connection
            .send_control(json!({"subtype": "initialize", "hooks": null}), initialize_timeout())
            .await?;
        Ok(connection)
    }

    pub(crate) fn server_info(&self) -> &Value {
        &self.initialization
    }

    /// Sends a control request and waits for its response body.
    async fn send_control(
        &self,
        request: Value,
        timeout: Duration,
    ) -> Result<Value, ProviderError> {
        let subtype = request.get("subtype").map(py_str).unwrap_or_default();
        let (sender, receiver) = oneshot::channel();
        let request_id = {
            let mut control = lock(&self.control);
            if let Some(reason) = &control.closed {
                return Err(ProviderError::Closed(reason.clone()));
            }
            control.counter += 1;
            let random: String =
                limina_core::new_uuid().chars().filter(|c| *c != '-').take(8).collect();
            let request_id = format!("req_{}_{random}", control.counter);
            control.pending.insert(request_id.clone(), sender);
            request_id
        };
        let message =
            json!({"type": "control_request", "request_id": request_id, "request": request});
        if let Err(error) = self.process.write_line(&message.to_string()).await {
            lock(&self.control).pending.remove(&request_id);
            return Err(ProviderError::Closed(format!("Claude is not accepting input: {error}")));
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(error))) => Err(ProviderError::Protocol(error)),
            Ok(Err(_)) => Err(ProviderError::Closed("Claude closed its output.".into())),
            Err(_) => {
                lock(&self.control).pending.remove(&request_id);
                Err(ProviderError::Closed(format!("Control request timeout: {subtype}")))
            }
        }
    }

    /// Sends a prompt as a user message.
    pub(crate) async fn query(&self, prompt: &str) -> Result<(), ProviderError> {
        let message = json!({
            "type": "user",
            "message": {"role": "user", "content": prompt},
            "parent_tool_use_id": null,
            "session_id": "default",
        });
        self.process.write_line(&message.to_string()).await.map_err(|error| {
            ProviderError::Closed(format!("Claude is not accepting input: {error}"))
        })
    }

    /// Asks the CLI to stop the running query; waits for its acknowledgement.
    pub(crate) async fn interrupt(&self) -> Result<(), ProviderError> {
        self.send_control(json!({"subtype": "interrupt"}), CONTROL_TIMEOUT).await.map(|_| ())
    }

    /// The next message for the turn.
    pub(crate) async fn next(&self) -> ClaudeIncoming {
        let mut messages = self.messages.lock().await;
        messages.recv().await.unwrap_or(ClaudeIncoming::End)
    }

    /// Ends the session: stdin closes, the CLI flushes and exits, or is killed after a grace
    /// period.
    pub(crate) async fn disconnect(&self) {
        self.process.shutdown(SHUTDOWN_GRACE).await;
    }
}

async fn read_loop(
    stdout: ChildStdout,
    process: Arc<ProviderProcess>,
    control: SharedControl,
    policy: Arc<ToolPolicy>,
    sender: mpsc::UnboundedSender<ClaudeIncoming>,
) {
    let mut lines = LineReader::new(stdout, MAX_LINE_BYTES);
    // The error text of the last `result` if it failed, to explain the exit that follows it.
    let mut last_error_result: Option<String> = None;
    while let Some(line) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('{') {
            // Some CLI builds print diagnostics such as `[SandboxDebug] …` on stdout.
            let line = clip(line, 200);
            tracing::debug!(target: LOG_TARGET, %line, "skipping non-JSON output");
            continue;
        }
        let message = match serde_json::from_str::<Value>(line) {
            Ok(message @ Value::Object(_)) => message,
            Ok(_) | Err(_) => {
                // The SDK treated this as fatal; one bad line no longer ends the session.
                let line = clip(line, 200);
                tracing::warn!(target: LOG_TARGET, %line, "skipping a malformed line from Claude");
                continue;
            }
        };
        match message.get("type").and_then(Value::as_str) {
            Some("control_response") => route_control_response(&control, &message),
            Some("control_request") => {
                let process = Arc::clone(&process);
                let policy = Arc::clone(&policy);
                tokio::spawn(answer_control_request(message, policy, process));
            }
            Some("control_cancel_request" | "transcript_mirror") => {}
            message_type => {
                if message_type == Some("result") {
                    last_error_result = message
                        .get("is_error")
                        .is_some_and(truthy)
                        .then(|| result_error_text(&message));
                } else if !(message_type == Some("system")
                    && message.get("subtype").and_then(Value::as_str)
                        == Some("session_state_changed"))
                {
                    last_error_result = None;
                }
                let _ = sender.send(ClaudeIncoming::Message(message));
            }
        }
    }

    process.stderr_drained(STDERR_DRAIN).await;
    let exit_code = tokio::time::timeout(EXIT_WAIT, process.wait_exit()).await.ok().flatten();
    let failure = match exit_code {
        Some(0) => None,
        code => Some(match &last_error_result {
            Some(text) => format!("Claude returned an error result: {text}"),
            None => {
                let code = code.map_or_else(|| "-1".to_owned(), |code| code.to_string());
                let stderr: String = process.stderr_tail(40).chars().take(2000).collect();
                let stderr = if stderr.is_empty() {
                    "Check stderr output for details".to_owned()
                } else {
                    stderr
                };
                // The SDK's `ProcessError` text, with the stderr it did not capture.
                let failed = format!("Command failed with exit code {code} (exit code: {code})");
                format!("{failed}\nError output: {stderr}")
            }
        }),
    };
    let reason = failure.clone().unwrap_or_else(|| "Claude closed its output.".to_owned());
    let pending = {
        let mut control = lock(&control);
        control.closed = Some(reason.clone());
        std::mem::take(&mut control.pending)
    };
    for (_, waiter) in pending {
        let _ = waiter.send(Err(reason.clone()));
    }
    let _ = sender.send(match failure {
        Some(text) => ClaudeIncoming::Error(text),
        None => ClaudeIncoming::End,
    });
}

fn result_error_text(result: &Value) -> String {
    let errors: Vec<String> = result
        .get("errors")
        .and_then(Value::as_array)
        .map(|errors| errors.iter().map(py_str).collect())
        .unwrap_or_default();
    if errors.is_empty() {
        result.get("subtype").map_or_else(|| "unknown error".to_owned(), py_str)
    } else {
        errors.join("; ")
    }
}

fn route_control_response(control: &SharedControl, message: &Value) {
    let Some(response) = message.get("response") else { return };
    let Some(request_id) = response.get("request_id").and_then(Value::as_str) else { return };
    let Some(waiter) = lock(control).pending.remove(request_id) else { return };
    let outcome = if response.get("subtype").and_then(Value::as_str) == Some("error") {
        Err(response.get("error").map_or_else(|| "Unknown error".to_owned(), py_str))
    } else {
        Ok(response.get("response").filter(|body| body.is_object()).cloned().unwrap_or(json!({})))
    };
    let _ = waiter.send(outcome);
}

/// Answers a request from the CLI. Only tool permissions are served; Limina registers no hooks
/// and no in-process MCP servers, so those get the SDK's answers for unknown ids and servers.
async fn answer_control_request(
    message: Value,
    policy: Arc<ToolPolicy>,
    process: Arc<ProviderProcess>,
) {
    let Some(request_id) = message.get("request_id").cloned() else {
        tracing::warn!(target: LOG_TARGET, "control request without an id");
        return;
    };
    let request = message.get("request").cloned().unwrap_or(Value::Null);
    let outcome: Result<Value, String> = match request.get("subtype").and_then(Value::as_str) {
        Some("can_use_tool") => match (request.get("tool_name"), request.get("input")) {
            (Some(tool_name), Some(input)) => {
                let tool_name = py_str(tool_name);
                let input = input.clone();
                tokio::task::spawn_blocking(move || policy.decide(&tool_name, &input).response())
                    .await
                    .map_err(|error| error.to_string())
            }
            (None, _) => Err("'tool_name'".to_owned()),
            (_, None) => Err("'input'".to_owned()),
        },
        Some("hook_callback") => Err(format!(
            "No hook callback found for ID: {}",
            request.get("callback_id").map(py_str).unwrap_or_default()
        )),
        Some("mcp_message") => {
            let server = request.get("server_name").filter(|value| truthy(value));
            let mcp = request.get("message").filter(|value| truthy(value));
            match (server, mcp) {
                (Some(server), Some(mcp)) => {
                    let not_found = format!("Server '{}' not found", py_str(server));
                    Ok(json!({"mcp_response": {
                        "jsonrpc": "2.0",
                        "id": mcp.get("id"),
                        "error": {"code": -32601, "message": not_found},
                    }}))
                }
                _ => Err("Missing server_name or message for MCP request".to_owned()),
            }
        }
        Some(other) => Err(format!("Unsupported control request subtype: {other}")),
        None => Err("'subtype'".to_owned()),
    };
    let response = match outcome {
        Ok(body) => json!({"type": "control_response", "response": {
            "subtype": "success", "request_id": request_id, "response": body,
        }}),
        Err(error) => json!({"type": "control_response", "response": {
            "subtype": "error", "request_id": request_id, "error": error,
        }}),
    };
    if let Err(error) = process.write_line(&response.to_string()).await {
        tracing::debug!(target: LOG_TARGET, %error, "could not answer a control request");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_results_explain_the_exit() {
        let result = json!({"type": "result", "is_error": true, "subtype": "error_max_turns"});
        assert_eq!(result_error_text(&result), "error_max_turns");
        let result = json!({"is_error": true, "errors": ["a", "b"]});
        assert_eq!(result_error_text(&result), "a; b");
    }
}
