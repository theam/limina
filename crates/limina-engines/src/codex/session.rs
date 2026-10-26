//! A Codex project session over `codex app-server`.
//!
//! Each turn starts a fresh app-server in the project workspace with an isolated environment
//! pinned to the project's Codex home, starts or resumes the project's thread, runs one turn
//! with the checkpoint output schema and streams its notifications into transcript events.
//! Steering and interruption go to the active turn over the same connection. The turn's
//! connected MCP servers are `mcp_servers` overrides whose credentials travel in the
//! app-server's environment only (see [`crate::mcp`]).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use limina_core::{LiminaError, Result, internal};
use limina_domain::environment::{codex_environment, process_environment};
use limina_domain::usage::TokenPricing;
use serde_json::{Value, json};

use super::accounts::CodexAuthResolver;
use super::rpc::{AppServerClient, CodexLaunch, Incoming, LOG_TARGET, notification_turn_id};
use crate::error::ProviderError;
use crate::events::map_codex_notification;
use crate::mcp::codex_mcp_launch;
use crate::process::blocking;
use crate::redaction::redact_text;
use crate::session::{AgentSession, TurnObserver, TurnRequest};
use crate::turn::{RuntimeTurn, parse_runtime_decision, turn_output_schema};
use crate::usage::codex_turn_usage;

/// Lets sandboxed commands reach the network, as the Python runtime configured it.
pub const WORKSPACE_NETWORK_OVERRIDE: &str = "sandbox_workspace_write.network_access=true";

#[derive(Debug, Clone)]
pub struct CodexSessionConfig {
    /// `LIMINA_CODEX_BIN`, default `codex`.
    pub codex_bin: String,
    /// `LIMINA_CODEX_MODEL`, default `gpt-5.4`.
    pub model: String,
    /// `LIMINA_CODEX_SANDBOX`: `read-only`, `workspace-write` (default) or `full-access`.
    pub sandbox: String,
    /// `LIMINA_CODEX_*_USD_PER_MILLION_TOKENS`, to estimate costs Codex does not report.
    pub pricing: Option<TokenPricing>,
}

impl Default for CodexSessionConfig {
    fn default() -> Self {
        Self {
            codex_bin: "codex".to_owned(),
            model: "gpt-5.4".to_owned(),
            sandbox: "workspace-write".to_owned(),
            pricing: None,
        }
    }
}

/// The wire `SandboxMode` for a configured preset.
fn sandbox_mode(sandbox: &str) -> Result<&'static str> {
    match sandbox {
        "read-only" => Ok("read-only"),
        "workspace-write" => Ok("workspace-write"),
        "full-access" => Ok("danger-full-access"),
        other => Err(LiminaError::transport(format!("Unsupported Codex sandbox '{other}'."))),
    }
}

struct Live {
    client: Arc<AppServerClient>,
    /// `(thread_id, turn_id)` once the turn has started.
    turn: Option<(String, String)>,
}

pub struct CodexAgentSession {
    config: CodexSessionConfig,
    auth: Arc<dyn CodexAuthResolver>,
    live: Mutex<Option<Live>>,
}

enum Failure {
    /// Limina's own errors (auth, observer) pass through unchanged.
    Limina(LiminaError),
    /// Provider failures become the turn-failed transport error.
    Provider(ProviderError),
}

struct Streamed {
    thread_id: String,
    turn_id: String,
    final_response: Option<String>,
    usage: Option<Value>,
}

/// Clears the live connection when a turn ends, even if its future is dropped, so the provider
/// process is never kept alive by a stale handle.
struct LiveGuard<'a>(&'a Mutex<Option<Live>>);

impl Drop for LiveGuard<'_> {
    fn drop(&mut self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
}

impl CodexAgentSession {
    pub fn new(config: CodexSessionConfig, auth: Arc<dyn CodexAuthResolver>) -> Self {
        Self { config, auth, live: Mutex::new(None) }
    }

    fn live(&self) -> MutexGuard<'_, Option<Live>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn active_turn(&self) -> Option<(Arc<AppServerClient>, String, String)> {
        let live = self.live();
        let live = live.as_ref()?;
        let (thread_id, turn_id) = live.turn.clone()?;
        Some((Arc::clone(&live.client), thread_id, turn_id))
    }

    async fn drive(
        &self,
        request: &TurnRequest,
        launch: CodexLaunch,
        sandbox: &str,
        auth: &super::auth::CodexAuthManager,
    ) -> std::result::Result<Streamed, Failure> {
        let _credentials = auth.turn().await.map_err(Failure::Limina)?;
        let client = Arc::new(AppServerClient::start(&launch).await.map_err(Failure::Provider)?);
        *self.live() = Some(Live { client: Arc::clone(&client), turn: None });
        let guard = LiveGuard(&self.live);
        let outcome = self.converse(&client, request, sandbox).await;
        drop(guard);
        client.shutdown().await;
        outcome
    }

    async fn converse(
        &self,
        client: &AppServerClient,
        request: &TurnRequest,
        sandbox: &str,
    ) -> std::result::Result<Streamed, Failure> {
        let cwd = request.workspace.to_string_lossy().into_owned();
        let model = request.settings.model.as_deref().unwrap_or(&self.config.model);
        let effort = match &request.settings.reasoning_effort {
            Some(effort) => Some(effort.clone()),
            None => tokio::time::timeout(
                std::time::Duration::from_secs(5),
                client.request("model/list", Some(json!({"includeHidden": true, "limit": 100}))),
            )
            .await
            .ok()
            .and_then(std::result::Result::ok)
            .and_then(|catalog| {
                catalog
                    .get("data")
                    .and_then(Value::as_array)
                    .and_then(|models| {
                        models
                            .iter()
                            .find(|item| item.get("model").and_then(Value::as_str) == Some(model))
                    })
                    .and_then(|item| item.get("defaultReasoningEffort").and_then(Value::as_str))
                    .map(str::to_owned)
            }),
        };
        let thread = match request.continuation_id.as_deref().filter(|id| !id.is_empty()) {
            Some(thread_id) => {
                let params = json!({
                    "threadId": thread_id,
                    "cwd": cwd,
                    "model": model,
                    "sandbox": sandbox,
                });
                client.request("thread/resume", Some(params)).await
            }
            None => {
                // The SDK's default approval mode for new threads (`auto_review`).
                let params = json!({
                    "approvalPolicy": "on-request",
                    "approvalsReviewer": "auto_review",
                    "cwd": cwd,
                    "model": model,
                    "sandbox": sandbox,
                });
                client.request("thread/start", Some(params)).await
            }
        }
        .map_err(Failure::Provider)?;
        let thread_id = required_id(&thread, "/thread/id", "thread")?;
        request.observer.continuation(&thread_id).await.map_err(Failure::Limina)?;

        let params = json!({
            "threadId": thread_id,
            "model": model,
            "effort": effort,
            "input": [{"type": "text", "text": request.prompt}],
            "outputSchema": turn_output_schema(),
        });
        let started =
            client.request("turn/start", Some(params)).await.map_err(Failure::Provider)?;
        let turn_id = required_id(&started, "/turn/id", "turn")?;
        if let Some(live) = self.live().as_mut() {
            live.turn = Some((thread_id.clone(), turn_id.clone()));
        }
        self.stream(client, thread_id, turn_id, request.observer.as_ref()).await
    }

    /// Consumes this turn's notifications until it completes, like the SDK's
    /// `_collect_turn_result` over a tapped turn stream.
    async fn stream(
        &self,
        client: &AppServerClient,
        thread_id: String,
        turn_id: String,
        observer: &dyn TurnObserver,
    ) -> std::result::Result<Streamed, Failure> {
        let mut items: Vec<Value> = Vec::new();
        let mut usage: Option<Value> = None;
        loop {
            let (method, params) = match client.next_incoming().await {
                Incoming::Closed(error) => return Err(Failure::Provider(error)),
                Incoming::Notification { method, params } => (method, params),
            };
            if notification_turn_id(&params) != Some(turn_id.as_str()) {
                continue;
            }
            if let Some(event) = map_codex_notification(&method, &params) {
                observer.event(event).await.map_err(Failure::Limina)?;
            }
            match method.as_str() {
                "item/completed" => {
                    if let Some(item) = params.get("item") {
                        items.push(item.clone());
                    }
                }
                "thread/tokenUsage/updated" => usage = params.get("tokenUsage").cloned(),
                "turn/completed" => {
                    let turn = params.get("turn").unwrap_or(&Value::Null);
                    if turn.get("status").and_then(Value::as_str) == Some("failed") {
                        let message = turn
                            .get("error")
                            .and_then(|error| error.get("message"))
                            .and_then(Value::as_str)
                            .filter(|message| !message.is_empty())
                            .map_or_else(
                                || "turn failed with status failed".to_owned(),
                                str::to_owned,
                            );
                        return Err(Failure::Provider(ProviderError::Protocol(message)));
                    }
                    let completed_id =
                        turn.get("id").and_then(Value::as_str).unwrap_or(&turn_id).to_owned();
                    return Ok(Streamed {
                        thread_id,
                        turn_id: completed_id,
                        final_response: final_response(&items),
                        usage,
                    });
                }
                _ => {}
            }
        }
    }
}

fn required_id(
    response: &Value,
    pointer: &str,
    what: &str,
) -> std::result::Result<String, Failure> {
    response.pointer(pointer).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| {
        Failure::Provider(ProviderError::Protocol(format!(
            "Codex returned no {what} id: {response}"
        )))
    })
}

/// The SDK's final answer: the last `final_answer` agent message, else the last agent message
/// without a phase.
fn final_response(items: &[Value]) -> Option<String> {
    let mut unknown_phase = None;
    for item in items.iter().rev() {
        if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
            continue;
        }
        let text = item.get("text").and_then(Value::as_str).unwrap_or("");
        match item.get("phase") {
            Some(Value::String(phase)) if phase == "final_answer" => return Some(text.to_owned()),
            None | Some(Value::Null) if unknown_phase.is_none() => {
                unknown_phase = Some(text.to_owned());
            }
            _ => {}
        }
    }
    unknown_phase
}

#[async_trait]
impl AgentSession for CodexAgentSession {
    async fn models(
        &self,
        _runtime_env: &std::collections::BTreeMap<String, String>,
    ) -> Result<crate::models::ModelCatalog> {
        let auth = self.auth.resolve().await?;
        let models = crate::models::codex_models(&self.config.codex_bin, auth.home()).await?;
        Ok(crate::models::ModelCatalog { default_model: Some(self.config.model.clone()), models })
    }

    async fn run_turn(&self, request: TurnRequest) -> Result<RuntimeTurn> {
        let sandbox = sandbox_mode(&self.config.sandbox)?;
        let auth = self.auth.resolve().await?;
        let home = auth.home().to_path_buf();
        let mcp = codex_mcp_launch(&request.mcp_servers);
        let mut overlay = request.runtime_env.clone();
        overlay.extend(mcp.env);
        let env = blocking(move || codex_environment(&process_environment(), &overlay, &home))
            .await
            .map_err(internal)?;
        let mut config_overrides = if self.config.sandbox == "workspace-write" {
            vec![WORKSPACE_NETWORK_OVERRIDE.to_owned()]
        } else {
            Vec::new()
        };
        config_overrides.extend(mcp.config_overrides);
        let launch = CodexLaunch {
            binary: self.config.codex_bin.clone(),
            config_overrides,
            cwd: Some(PathBuf::from(&request.workspace)),
            env,
            mcp_servers: request.mcp_servers.iter().map(|server| server.name.clone()).collect(),
        };
        let streamed = match self.drive(&request, launch, sandbox, &auth).await {
            Ok(streamed) => streamed,
            Err(Failure::Limina(error)) => return Err(error),
            Err(Failure::Provider(error)) => {
                let secrets = request.redactions();
                return Err(LiminaError::transport(
                    "The managed Codex turn failed before Limina accepted its checkpoint.",
                )
                .with("reason", redact_text(&error.to_string(), &secrets))
                .with("continuation_id", request.continuation_id.clone())
                .with("retryable", error.is_retryable()));
            }
        };
        let Some(response) = streamed.final_response.filter(|text| !text.is_empty()) else {
            return Err(LiminaError::transport(
                "Codex completed without the required project decision.",
            )
            .with("continuation_id", streamed.thread_id)
            .with("turn_id", streamed.turn_id));
        };
        let decision = parse_runtime_decision(&Value::String(response), "Codex")?;
        Ok(RuntimeTurn {
            continuation_id: Some(streamed.thread_id),
            turn_id: streamed.turn_id,
            decision,
            usage: codex_turn_usage(streamed.usage.as_ref(), self.config.pricing.as_ref()),
        })
    }

    async fn steer(&self, message: &str) -> Result<bool> {
        let Some((client, thread_id, turn_id)) = self.active_turn() else {
            return Ok(false);
        };
        let params = json!({
            "threadId": thread_id,
            "expectedTurnId": turn_id,
            "input": [{"type": "text", "text": message}],
        });
        client.request("turn/steer", Some(params)).await.map_err(internal)?;
        Ok(true)
    }

    async fn interrupt(&self) -> Result<bool> {
        let Some((client, thread_id, turn_id)) = self.active_turn() else {
            return Ok(false);
        };
        let params = json!({"threadId": thread_id, "turnId": turn_id});
        client.request("turn/interrupt", Some(params)).await.map_err(internal)?;
        Ok(true)
    }

    async fn close(&self) {
        if let Err(error) = self.interrupt().await {
            tracing::debug!(target: LOG_TARGET, %error, "interrupt on close failed");
        }
        let client = self.live().as_ref().map(|live| Arc::clone(&live.client));
        if let Some(client) = client {
            client.shutdown().await;
        }
    }

    fn model(&self) -> Option<String> {
        Some(self.config.model.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandboxes_map_to_wire_modes() {
        assert_eq!(sandbox_mode("full-access").unwrap(), "danger-full-access");
        let error = sandbox_mode("yolo").unwrap_err();
        assert_eq!(error.message, "Unsupported Codex sandbox 'yolo'.");
        assert_eq!(error.code(), "transport_error");
    }

    #[test]
    fn the_final_answer_wins_over_commentary() {
        let items = vec![
            json!({"type": "agentMessage", "text": "first", "phase": null}),
            json!({"type": "agentMessage", "text": "final", "phase": "final_answer"}),
            json!({"type": "agentMessage", "text": "later", "phase": "commentary"}),
        ];
        assert_eq!(final_response(&items).as_deref(), Some("final"));
        let items = vec![
            json!({"type": "agentMessage", "text": "older"}),
            json!({"type": "agentMessage", "text": "newer"}),
            json!({"type": "agentMessage", "text": "chatter", "phase": "commentary"}),
        ];
        assert_eq!(final_response(&items).as_deref(), Some("newer"));
        assert_eq!(final_response(&[json!({"type": "plan", "text": "x"})]), None);
    }
}
