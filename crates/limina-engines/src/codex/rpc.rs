//! JSON-RPC over stdio with `codex app-server`, spoken the way the official Python SDK
//! (`openai_codex` 0.1.0b2, `client.py` and `_message_router.py`) speaks it:
//!
//! - launch `codex [--config k=v]… app-server --listen stdio://`;
//! - one JSON object per line, requests `{"id", "method", "params"}` with UUID string ids and no
//!   `jsonrpc` member;
//! - `initialize` with the SDK's client info, then the `initialized` notification;
//! - server requests (approvals) answered with the SDK's default handler;
//! - a single stdout reader routes responses to their waiters and forwards notifications.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::process::ChildStdout;
use tokio::sync::{mpsc, oneshot};

use crate::error::ProviderError;
use crate::process::{
    AbortOnDrop, LineReader, MAX_LINE_BYTES, ProcessSpec, ProviderProcess, STDERR_DRAIN,
    spawn_failure,
};
use crate::text::{clip, py_int, py_str};

/// Client identity sent in `initialize`. Codex derives its user agent from it; app-server
/// clients are expected to name themselves rather than borrow another client's identity.
pub const CLIENT_NAME: &str = "limina";
pub const CLIENT_TITLE: &str = "Limina";
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const LOG_TARGET: &str = "limina_engines::codex";
/// The SDK waits this long after closing stdin before killing the process.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// How to start one `codex app-server`.
#[derive(Debug, Clone)]
pub(crate) struct CodexLaunch {
    pub binary: String,
    pub config_overrides: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    /// The connected MCP servers whose tool calls Limina approves when Codex asks.
    pub mcp_servers: BTreeSet<String>,
}

impl CodexLaunch {
    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for value in &self.config_overrides {
            args.push("--config".to_owned());
            args.push(value.clone());
        }
        args.extend(["app-server", "--listen", "stdio://"].map(str::to_owned));
        args
    }
}

/// What the reader forwards to the client's consumer.
#[derive(Debug, Clone)]
pub(crate) enum Incoming {
    Notification { method: String, params: Value },
    Closed(ProviderError),
}

#[derive(Default)]
struct Router {
    pending: HashMap<String, oneshot::Sender<Result<Value, ProviderError>>>,
    closed: Option<ProviderError>,
}

type SharedRouter = Arc<Mutex<Router>>;

fn lock(router: &SharedRouter) -> std::sync::MutexGuard<'_, Router> {
    router.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One running `codex app-server`.
pub(crate) struct AppServerClient {
    process: Arc<ProviderProcess>,
    router: SharedRouter,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<Incoming>>,
    _reader: AbortOnDrop,
}

impl AppServerClient {
    /// Starts the server and completes the `initialize` handshake.
    pub(crate) async fn start(launch: &CodexLaunch) -> Result<Self, ProviderError> {
        let args = launch.args();
        let spec = ProcessSpec {
            label: "codex",
            program: &launch.binary,
            args: &args,
            env: &launch.env,
            cwd: launch.cwd.as_deref(),
        };
        let (process, stdout) = match ProviderProcess::spawn(&spec) {
            Ok(spawned) => spawned,
            Err(error) => {
                let reason =
                    spawn_failure(launch.cwd.as_deref(), &error).await.unwrap_or_else(|| {
                        format!("Could not start Codex ({}): {error}", launch.binary)
                    });
                return Err(ProviderError::Spawn(reason));
            }
        };
        let process = Arc::new(process);
        let router = SharedRouter::default();
        let (sender, receiver) = mpsc::unbounded_channel();
        let reader = AbortOnDrop::spawn(read_loop(
            stdout,
            Arc::clone(&process),
            Arc::clone(&router),
            sender,
            Arc::new(launch.mcp_servers.clone()),
        ));
        let client =
            Self { process, router, incoming: tokio::sync::Mutex::new(receiver), _reader: reader };
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&self) -> Result<(), ProviderError> {
        let params = json!({
            "clientInfo": {"name": CLIENT_NAME, "title": CLIENT_TITLE, "version": CLIENT_VERSION},
            "capabilities": {"experimentalApi": true},
        });
        let result = self.request("initialize", Some(params)).await?;
        validate_initialize(&result)?;
        self.notify("initialized", None).await
    }

    /// Sends a request and waits for its response.
    pub(crate) async fn request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, ProviderError> {
        let id = limina_core::new_uuid();
        let (sender, receiver) = oneshot::channel();
        {
            let mut router = lock(&self.router);
            if let Some(error) = &router.closed {
                return Err(error.clone());
            }
            router.pending.insert(id.clone(), sender);
        }
        let mut message = Map::new();
        message.insert("id".to_owned(), Value::String(id.clone()));
        message.insert("method".to_owned(), Value::String(method.to_owned()));
        if let Some(params) = params {
            message.insert("params".to_owned(), params);
        }
        if let Err(error) = self.process.write_line(&Value::Object(message).to_string()).await {
            lock(&self.router).pending.remove(&id);
            return Err(ProviderError::Closed(format!("Codex process is not running: {error}")));
        }
        receiver
            .await
            .unwrap_or_else(|_| Err(ProviderError::Closed("Codex process closed stdout.".into())))
    }

    /// Sends a notification.
    pub(crate) async fn notify(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), ProviderError> {
        let mut message = Map::new();
        message.insert("method".to_owned(), Value::String(method.to_owned()));
        if let Some(params) = params {
            message.insert("params".to_owned(), params);
        }
        self.process.write_line(&Value::Object(message).to_string()).await.map_err(|error| {
            ProviderError::Closed(format!("Codex process is not running: {error}"))
        })
    }

    /// The next notification, or the transport failure once the server is gone.
    pub(crate) async fn next_incoming(&self) -> Incoming {
        let mut receiver = self.incoming.lock().await;
        match receiver.recv().await {
            Some(incoming) => incoming,
            None => {
                Incoming::Closed(lock(&self.router).closed.clone().unwrap_or_else(|| {
                    ProviderError::Closed("Codex process closed stdout.".into())
                }))
            }
        }
    }

    /// Closes stdin, waits briefly for a clean exit, then kills the server.
    pub(crate) async fn shutdown(&self) {
        self.process.shutdown(SHUTDOWN_GRACE).await;
    }
}

/// The SDK's `validate_initialize_metadata`: a user agent and a server name and version, taken
/// from `serverInfo` or parsed from the user agent.
fn validate_initialize(result: &Value) -> Result<(), ProviderError> {
    let Some(result) = result.as_object() else {
        return Err(ProviderError::Protocol("initialize response must be a JSON object".into()));
    };
    fn text(value: Option<&Value>) -> Option<&str> {
        value.and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty())
    }
    let user_agent = text(result.get("userAgent")).unwrap_or("");
    let server = result.get("serverInfo");
    let mut name = text(server.and_then(|info| info.get("name"))).map(str::to_owned);
    let mut version = text(server.and_then(|info| info.get("version"))).map(str::to_owned);
    if (name.is_none() || version.is_none()) && !user_agent.is_empty() {
        let (parsed_name, parsed_version) = split_user_agent(user_agent);
        name = name.or(parsed_name);
        version = version.or(parsed_version);
    }
    let name = name.unwrap_or_default();
    let version = version.unwrap_or_default();
    if user_agent.is_empty() || name.trim().is_empty() || version.trim().is_empty() {
        return Err(ProviderError::Protocol(format!(
            "initialize response missing required metadata (user_agent={user_agent:?}, \
             server_name={:?}, server_version={:?})",
            name.trim(),
            version.trim()
        )));
    }
    Ok(())
}

fn split_user_agent(user_agent: &str) -> (Option<String>, Option<String>) {
    let some = |text: &str| (!text.is_empty()).then(|| text.to_owned());
    if let Some((name, version)) = user_agent.split_once('/') {
        return (some(name), some(version));
    }
    let mut parts = user_agent.splitn(2, char::is_whitespace);
    match (parts.next(), parts.next().map(str::trim_start)) {
        (Some(name), Some(version)) if !version.is_empty() => (some(name), some(version)),
        _ => (some(user_agent), None),
    }
}

/// The SDK's default approval handler: accept command and file-change approvals, answer
/// anything else with an empty object. MCP elicitations are answered by [`mcp_elicitation`].
fn server_request_response(method: &str, params: &Value, mcp_servers: &BTreeSet<String>) -> Value {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({"decision": "accept"})
        }
        "mcpServer/elicitation/request" => mcp_elicitation(params, mcp_servers),
        _ => json!({}),
    }
}

/// Codex asks to approve an MCP tool call through an elicitation marked
/// `_meta.codex_approval_kind = "mcp_tool_call"`. Calls to a server connected to the project are
/// approved (its owner enabled it for this project); nobody is there to answer any other
/// elicitation during a managed turn, so those are declined.
fn mcp_elicitation(params: &Value, mcp_servers: &BTreeSet<String>) -> Value {
    let server = params.get("serverName").and_then(Value::as_str).unwrap_or_default();
    let tool_call = params.pointer("/_meta/codex_approval_kind").and_then(Value::as_str)
        == Some("mcp_tool_call");
    if tool_call && mcp_servers.contains(server) {
        json!({"action": "accept", "content": {}})
    } else {
        json!({"action": "decline", "content": null})
    }
}

async fn read_loop(
    stdout: ChildStdout,
    process: Arc<ProviderProcess>,
    router: SharedRouter,
    sender: mpsc::UnboundedSender<Incoming>,
    mcp_servers: Arc<BTreeSet<String>>,
) {
    let mut lines = LineReader::new(stdout, MAX_LINE_BYTES);
    while let Some(line) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let message = match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Object(message)) => message,
            Ok(_) | Err(_) => {
                // The SDK treated this as fatal; one bad line no longer ends the session.
                let line = clip(trimmed, 200);
                tracing::warn!(target: LOG_TARGET, %line, "skipping a malformed line from Codex");
                continue;
            }
        };
        let method = message.get("method").and_then(Value::as_str);
        match (method, message.get("id")) {
            (Some(method), Some(id)) => {
                let params = message.get("params").unwrap_or(&Value::Null);
                let result = server_request_response(method, params, &mcp_servers);
                let response = json!({"id": id, "result": result});
                let process = Arc::clone(&process);
                // Answer from another task, so a full stdin pipe never stalls this reader.
                tokio::spawn(async move {
                    if let Err(error) = process.write_line(&response.to_string()).await {
                        tracing::debug!(target: LOG_TARGET, %error, "could not answer a request");
                    }
                });
            }
            (Some(method), None) => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let _ = sender.send(Incoming::Notification { method: method.to_owned(), params });
            }
            (None, _) => route_response(&router, &message),
        }
    }
    process.stderr_drained(STDERR_DRAIN).await;
    let tail: String = process.stderr_tail(40).chars().take(2000).collect();
    let error = ProviderError::Closed(format!("Codex process closed stdout. stderr_tail={tail}"));
    let pending = {
        let mut router = lock(&router);
        router.closed = Some(error.clone());
        std::mem::take(&mut router.pending)
    };
    for (_, waiter) in pending {
        let _ = waiter.send(Err(error.clone()));
    }
    let _ = sender.send(Incoming::Closed(error));
}

fn route_response(router: &SharedRouter, message: &Map<String, Value>) {
    let id = match message.get("id") {
        Some(Value::String(id)) => id.clone(),
        Some(other) => py_str(other),
        None => "None".to_owned(),
    };
    let Some(waiter) = lock(router).pending.remove(&id) else {
        return;
    };
    let outcome = match message.get("error") {
        Some(Value::Object(error)) => Err(ProviderError::Rpc {
            code: error.get("code").and_then(py_int).unwrap_or(-32000),
            message: error.get("message").map_or_else(|| "unknown".to_owned(), py_str),
            data: error.get("data").cloned(),
        }),
        Some(_) => Err(ProviderError::Protocol("Malformed JSON-RPC error response".into())),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = waiter.send(outcome);
}

/// The turn a notification belongs to: `params.turnId`, or `params.turn.id` for turn
/// lifecycle notifications. Others are thread- or connection-scoped.
pub(crate) fn notification_turn_id(params: &Value) -> Option<&str> {
    params
        .get("turnId")
        .and_then(Value::as_str)
        .or_else(|| params.get("turn").and_then(|turn| turn.get("id")).and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_arguments_match_the_sdk() {
        let launch = CodexLaunch {
            binary: "codex".into(),
            config_overrides: vec!["a=b".into()],
            cwd: None,
            env: BTreeMap::new(),
            mcp_servers: BTreeSet::new(),
        };
        assert_eq!(launch.args(), ["--config", "a=b", "app-server", "--listen", "stdio://"]);
    }

    #[test]
    fn initialize_metadata_falls_back_to_the_user_agent() {
        assert!(validate_initialize(&json!({"userAgent": "codex/0.144.4 (Mac OS)"})).is_ok());
        assert!(
            validate_initialize(
                &json!({"userAgent": "x", "serverInfo": {"name": "codex", "version": "1"}})
            )
            .is_ok()
        );
        assert!(validate_initialize(&json!({"userAgent": "codex 1.0"})).is_ok());
        let error = validate_initialize(&json!({"userAgent": "codex"})).unwrap_err();
        assert!(error.to_string().contains("missing required metadata"));
        assert!(validate_initialize(&json!({})).is_err());
    }

    #[test]
    fn approvals_are_accepted_like_the_sdk_default_handler() {
        let none = BTreeSet::new();
        assert_eq!(
            server_request_response("item/commandExecution/requestApproval", &Value::Null, &none),
            json!({"decision": "accept"})
        );
        assert_eq!(
            server_request_response("item/tool/requestUserInput", &Value::Null, &none),
            json!({}),
        );
    }

    #[test]
    fn only_connected_mcp_tool_calls_are_approved() {
        let servers = BTreeSet::from(["notes".to_owned()]);
        let ask = |server: &str, kind: Option<&str>| {
            let params = json!({"serverName": server, "threadId": "t", "mode": "form",
                                "message": "Allow?", "requestedSchema": {"type": "object"},
                                "_meta": kind.map(|kind| json!({"codex_approval_kind": kind}))});
            server_request_response("mcpServer/elicitation/request", &params, &servers)
        };
        assert_eq!(ask("notes", Some("mcp_tool_call")), json!({"action": "accept", "content": {}}));
        assert_eq!(ask("other", Some("mcp_tool_call"))["action"], "decline");
        assert_eq!(ask("notes", None)["action"], "decline");
    }

    #[test]
    fn turn_ids_come_from_direct_or_nested_fields() {
        assert_eq!(notification_turn_id(&json!({"turnId": "t1"})), Some("t1"));
        assert_eq!(notification_turn_id(&json!({"turn": {"id": "t2"}})), Some("t2"));
        assert_eq!(notification_turn_id(&json!({"threadId": "x"})), None);
    }
}
