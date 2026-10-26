//! `ScriptedAgent`: a deterministic [`AgentSession`] that behaves like a managed Codex session.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use limina_core::{LiminaError, Result};
use limina_domain::Engine;
use limina_domain::usage::RuntimeUsage;
use limina_engines::events::{CODEX_EVENT, KNOWLEDGE_SUMMARY, KNOWLEDGE_TOOL};
use limina_engines::{
    AgentFactory, AgentSession, RuntimeDecision, RuntimeEvent, RuntimeTurn, TurnObserver,
    TurnRequest,
};
use serde_json::{Map, Value, json};
use tokio::sync::watch;

use super::client::{InternalClient, hex_id};

/// The model the scripted engine reports for its runs.
pub const SCRIPTED_MODEL: &str = "scripted-dev";
/// Why an interrupted scripted turn fails (a `transport_error`).
pub const INTERRUPTED: &str = "The scripted turn was interrupted before checkpointing.";
/// The first line of the message that acknowledges live steering; the notes follow, quoted.
pub const STEERING_ACK: &str = "Noted — adjusting course with your guidance:";
/// Every turn's token count starts here; each message adds a third of its length.
const BASE_TOKENS: i64 = 1800;
/// The keys of every event's `detail`, in the order the console's stored events have them.
const DETAIL_KEYS: [&str; 13] = [
    "method",
    "item_type",
    "kind",
    "text",
    "tool",
    "input",
    "item_id",
    "parent_item_id",
    "phase",
    "state",
    "exit_code",
    "duration_ms",
    "files",
];
const SUMMARY_LIMIT: usize = 200;
const COMMAND_SUMMARY_LIMIT: usize = 80;

/// A deterministic engine that plays the demo scenario against the internal agent API.
///
/// Every step waits a multiple of `pace` (the script's `LIMINA_DEV_PACE`). Steering is quoted back
/// in a message at the next step; an interrupt ends the turn with a `transport_error`
/// ([`INTERRUPTED`]). It keeps no provider thread, so its turns return no continuation.
pub struct ScriptedAgent {
    slug: String,
    pace: Duration,
    active: AtomicBool,
    interrupted: watch::Sender<bool>,
    steering: Mutex<Vec<String>>,
}

impl ScriptedAgent {
    pub fn new(slug: impl Into<String>, pace: Duration) -> Self {
        Self {
            slug: slug.into(),
            pace,
            active: AtomicBool::new(false),
            interrupted: watch::Sender::new(false),
            steering: Mutex::new(Vec::new()),
        }
    }

    pub fn slug(&self) -> &str {
        &self.slug
    }

    pub fn pace(&self) -> Duration {
        self.pace
    }

    fn steering(&self) -> MutexGuard<'_, Vec<String>> {
        self.steering.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A factory that gives every project its own [`ScriptedAgent`], whatever its engine.
pub fn scripted_factory(pace: Duration) -> AgentFactory {
    Arc::new(move |slug: &str, _engine: Engine| {
        Ok(Arc::new(ScriptedAgent::new(slug, pace)) as Arc<dyn AgentSession>)
    })
}

/// Clears the active flag however the turn ends (including when the supervisor drops it).
struct Active<'a>(&'a AtomicBool);

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[async_trait]
impl AgentSession for ScriptedAgent {
    async fn models(
        &self,
        _environment: &std::collections::BTreeMap<String, String>,
    ) -> Result<limina_engines::models::ModelCatalog> {
        // The development provider advertises its own deterministic test capabilities.
        Ok(limina_engines::models::ModelCatalog {
            default_model: Some(SCRIPTED_MODEL.to_owned()),
            models: vec![
                limina_engines::models::ModelOption {
                    id: SCRIPTED_MODEL.to_owned(),
                    label: "Scripted research model (demo)".to_owned(),
                    resolved_model: None,
                    reasoning_efforts: vec!["low".to_owned(), "high".to_owned()],
                    default_reasoning_effort: Some("high".to_owned()),
                },
                limina_engines::models::ModelOption {
                    id: "scripted-fast".to_owned(),
                    label: "Scripted fast model (demo)".to_owned(),
                    resolved_model: None,
                    reasoning_efforts: Vec::new(),
                    default_reasoning_effort: None,
                },
            ],
        })
    }

    async fn run_turn(&self, request: TurnRequest) -> Result<RuntimeTurn> {
        self.active.store(true, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.interrupted.send_replace(false);
        let client = InternalClient::new(&request.runtime_env, &self.slug)?;
        let workspace = request.workspace.clone();
        let mut turn = Turn {
            agent: self,
            client,
            observer: Arc::clone(&request.observer),
            prompt: request.prompt,
            tokens: BASE_TOKENS,
        };
        let decision = turn.play().await?;
        if let Some(update) = &decision.mission_update {
            let outputs = workspace.join("outputs");
            tokio::fs::create_dir_all(&outputs).await.map_err(limina_core::internal)?;
            tokio::fs::write(outputs.join("threshold-comparison.csv"),
                "# Simulated demo data, not real measurements\nthreshold,unsupported_citations_percent,answer_rate_percent\n0.5,8.1,90.2\n0.6,6.4,89.1\n0.7,4.9,87.4\n0.8,3.8,84.0\n").await.map_err(limina_core::internal)?;
            let evidence =
                update.evidence.iter().map(|id| format!("[[{id}]]")).collect::<Vec<_>>().join(", ");
            let report = format!(
                "# {}\n\n> Scripted demonstration. All measurements are illustrative.\n\n## Established\n\n{}\n\n## Ruled out\n\n{}\n\n## Limitations\n\n{}\n\n## Evidence\n\n{}\n",
                update.headline, update.established, update.ruled_out, update.uncertain, evidence
            );
            tokio::fs::write(outputs.join("research-summary.md"), report)
                .await
                .map_err(limina_core::internal)?;
        }
        let tokens = turn.tokens;
        // No provider thread exists, so none is recorded: a real engine starts fresh from kb state.
        Ok(RuntimeTurn {
            continuation_id: None,
            turn_id: format!("turn-{}", &hex_id()[..8]),
            decision,
            usage: Some(RuntimeUsage {
                input_tokens: Some(tokens * 9),
                output_tokens: Some(tokens),
                cached_input_tokens: Some(tokens * 4),
                total_tokens: Some(tokens * 10),
                usage_source: Some("provider".to_owned()),
                ..RuntimeUsage::default()
            }),
        })
    }

    async fn steer(&self, message: &str) -> Result<bool> {
        if !self.active.load(Ordering::SeqCst) {
            return Ok(false);
        }
        self.steering().push(message.to_owned());
        Ok(true)
    }

    async fn interrupt(&self) -> Result<bool> {
        if !self.active.load(Ordering::SeqCst) {
            return Ok(false);
        }
        self.interrupted.send_replace(true);
        Ok(true)
    }

    async fn close(&self) {
        self.interrupted.send_replace(true);
    }

    fn model(&self) -> Option<String> {
        Some(SCRIPTED_MODEL.to_owned())
    }
}

/// One turn in progress: the streaming helpers the scenario ([`super::scenario`]) is written in.
pub(super) struct Turn<'a> {
    agent: &'a ScriptedAgent,
    client: InternalClient,
    observer: Arc<dyn TurnObserver>,
    pub(super) prompt: String,
    /// Grows with every message; the turn's usage is derived from it.
    pub(super) tokens: i64,
}

impl Turn<'_> {
    async fn emit(&self, summary: String, detail: Map<String, Value>) -> Result<()> {
        self.observer.event(RuntimeEvent::new(CODEX_EVENT, summary, detail)).await
    }

    /// Waits `factor` × pace, then fails if the turn was interrupted, then acknowledges any
    /// steering that arrived.
    pub(super) async fn pause(&mut self, factor: f64) -> Result<()> {
        let delay = Duration::try_from_secs_f64(self.agent.pace.as_secs_f64() * factor)
            .unwrap_or(Duration::ZERO);
        let mut interrupted = self.agent.interrupted.subscribe();
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            _ = interrupted.wait_for(|interrupted| *interrupted) => {}
        }
        if *self.agent.interrupted.borrow() {
            return Err(LiminaError::transport(INTERRUPTED));
        }
        let notes = std::mem::take(&mut *self.agent.steering());
        if !notes.is_empty() {
            // The supervisor delivers `<actor>: <body>`; only the body is quoted.
            let quoted: Vec<String> = notes
                .iter()
                .map(|note| note.split_once(": ").map_or(note.as_str(), |(_actor, body)| body))
                .map(|body| format!("> {body}"))
                .collect();
            self.message(&format!("{STEERING_ACK}\n\n{}", quoted.join("\n"))).await?;
        }
        Ok(())
    }

    /// An agent message, after a pause.
    pub(super) async fn say(&mut self, text: &str) -> Result<()> {
        self.pause(1.4).await?;
        self.message(text).await
    }

    /// An agent message, now.
    async fn message(&mut self, text: &str) -> Result<()> {
        self.tokens += i64::try_from(text.chars().count() / 3).unwrap_or(0);
        let detail = detail([
            ("method", json!("item/completed")),
            ("item_type", json!("AgentMessage")),
            ("kind", json!("message")),
            ("text", json!(text)),
            ("phase", json!("completed")),
        ]);
        self.emit(clip(first_line(text), SUMMARY_LIMIT), detail).await
    }

    /// A shell command that runs for `seconds` × pace and exits with `exit_code`.
    pub(super) async fn run(&mut self, command: &str, seconds: f64, exit_code: i64) -> Result<()> {
        let item = item_id();
        let shown = clip(command, COMMAND_SUMMARY_LIMIT);
        let started = detail([
            ("method", json!("item/started")),
            ("item_type", json!("CommandExecution")),
            ("kind", json!("tool")),
            ("tool", json!("command")),
            ("input", json!(command)),
            ("item_id", json!(item)),
            ("phase", json!("started")),
            ("state", json!("running")),
        ]);
        self.emit(format!("Running {shown}"), started).await?;
        self.pause(seconds).await?;
        // `int(seconds * PACE * 1000)`, in the same floating-point order.
        let duration_ms = (seconds * self.agent.pace.as_secs_f64() * 1000.0) as i64;
        let completed = detail([
            ("method", json!("item/completed")),
            ("item_type", json!("CommandExecution")),
            ("kind", json!("tool")),
            ("tool", json!("command")),
            ("input", json!(command)),
            ("item_id", json!(item)),
            ("phase", json!("completed")),
            ("state", json!(if exit_code == 0 { "ok" } else { "failed" })),
            ("exit_code", json!(exit_code)),
            ("duration_ms", json!(duration_ms)),
        ]);
        self.emit(format!("Ran {shown}"), completed).await
    }

    /// A completed edit of `paths`, after a pause.
    pub(super) async fn edit(&mut self, paths: &[&str]) -> Result<()> {
        self.pause(0.8).await?;
        let files: Vec<Value> =
            paths.iter().map(|path| json!({"path": path, "kind": "update"})).collect();
        let detail = detail([
            ("method", json!("item/completed")),
            ("item_type", json!("FileChange")),
            ("kind", json!("tool")),
            ("tool", json!("edit")),
            ("input", json!(paths.join(", "))),
            ("item_id", json!(item_id())),
            ("phase", json!("completed")),
            ("state", json!("ok")),
            ("files", Value::Array(files)),
        ]);
        let plural = if paths.len() > 1 { "s" } else { "" };
        self.emit(format!("Edited {} file{plural}", paths.len()), detail).await
    }

    /// A knowledge write: `POST /internal/v1/projects/{slug}/{path}`, shown as one knowledge item.
    pub(super) async fn knowledge(&mut self, path: &str, body: Value) -> Result<Value> {
        let item = item_id();
        let started = detail([
            ("method", json!("item/started")),
            ("item_type", json!("CommandExecution")),
            ("kind", json!("knowledge")),
            ("tool", json!(KNOWLEDGE_TOOL)),
            ("item_id", json!(item)),
            ("phase", json!("started")),
            ("state", json!("running")),
        ]);
        self.emit(KNOWLEDGE_SUMMARY.to_owned(), started).await?;
        self.pause(0.6).await?;
        let response = self.client.post(path, &body).await?;
        let completed = detail([
            ("method", json!("item/completed")),
            ("item_type", json!("CommandExecution")),
            ("kind", json!("knowledge")),
            ("tool", json!(KNOWLEDGE_TOOL)),
            ("item_id", json!(item)),
            ("phase", json!("completed")),
            ("state", json!("ok")),
            ("exit_code", json!(0)),
        ]);
        self.emit(KNOWLEDGE_SUMMARY.to_owned(), completed).await?;
        Ok(response)
    }

    /// One artifact (`GET …/artifacts/{id}`).
    pub(super) async fn artifact(&self, artifact_id: &str) -> Result<Value> {
        self.client.get(&format!("artifacts/{artifact_id}")).await
    }

    /// The display IDs of the project's artifacts, waiting for the server to start.
    pub(super) async fn artifact_ids(&self) -> Result<Vec<String>> {
        let artifacts = self.client.artifacts().await?;
        Ok(artifacts
            .iter()
            .filter_map(|artifact| artifact.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect())
    }
}

/// A checkpoint decision.
pub(super) fn decision(
    summary: &str,
    status: &str,
    current_objective: &str,
    next_step: &str,
    blocker: &str,
) -> RuntimeDecision {
    RuntimeDecision {
        mission_update: None,
        summary: summary.to_owned(),
        status: status.to_owned(),
        current_objective: current_objective.to_owned(),
        next_step: next_step.to_owned(),
        blocker: blocker.to_owned(),
    }
}

/// The script's `_detail(**values)`: every key, `null` unless given, in the stored order.
fn detail<const N: usize>(values: [(&str, Value); N]) -> Map<String, Value> {
    let mut detail: Map<String, Value> =
        DETAIL_KEYS.iter().map(|key| ((*key).to_owned(), Value::Null)).collect();
    for (key, value) in values {
        detail.insert(key.to_owned(), value);
    }
    detail
}

/// A short random item id (`uuid4().hex[:10]`).
fn item_id() -> String {
    hex_id()[..10].to_owned()
}

/// `text.splitlines()[0]`, on Python's line boundaries.
fn first_line(text: &str) -> &str {
    text.split([
        '\n', '\r', '\u{0b}', '\u{0c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
        '\u{2029}',
    ])
    .next()
    .unwrap_or_default()
}

/// `text[:limit]` in characters.
fn clip(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_keep_every_key_in_order() {
        let detail = detail([("kind", json!("message")), ("method", json!("item/completed"))]);
        assert_eq!(detail.keys().map(String::as_str).collect::<Vec<_>>(), DETAIL_KEYS);
        assert_eq!(detail["method"], "item/completed");
        assert_eq!(detail["kind"], "message");
        assert!(detail["files"].is_null());
    }

    #[test]
    fn summaries_follow_python_slicing() {
        assert_eq!(first_line("Baseline:\n\n| a |"), "Baseline:");
        assert_eq!(first_line("one\r\ntwo"), "one");
        assert_eq!(first_line(""), "");
        assert_eq!(clip("añb", 2), "añ");
        assert_eq!(item_id().len(), 10);
    }
}
