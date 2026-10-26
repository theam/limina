//! Fixtures for the development runtime tests: a runtime prepared by `limina_server::dev` on a
//! temporary directory, and the internal agent API (`/internal/v1/projects/{slug}/…`) served over
//! HTTP by the real application layer, recording every request. Every value here is fake.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use limina_core::{LiminaError, Principal, Result, new_uuid};
use limina_domain::research::NewHypothesis;
use limina_domain::research::{ExperimentOutcome, HypothesisDecision, NewExperiment, NewFinding};
use limina_engines::{
    AgentSession, ChannelObserver, RuntimeEvent, RuntimeTurn, TurnRequest, TurnSignal,
};
use limina_server::app::{NewProject, Runtime};
use limina_server::dev::{self, DEMO_ACTOR, DevOptions};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// A turn prompt without new guidance, in the supervisor's wording.
pub const QUIET: &str = "# Turn\n\n## New human guidance\n\n- No new human guidance.\n";

/// One request the internal API received: its path below `/internal/v1/projects/{slug}/`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Value,
}

#[derive(Clone)]
struct Api {
    runtime: Arc<Runtime>,
    requests: Arc<Mutex<Vec<Request>>>,
}

/// A development runtime with the internal agent API listening on `options.port`.
pub struct DevRuntime {
    pub runtime: Arc<Runtime>,
    pub options: DevOptions,
    requests: Arc<Mutex<Vec<Request>>>,
    _dir: tempfile::TempDir,
}

impl DevRuntime {
    /// Prepares a runtime with `dev::prepare` (a 1 ms pace and a 50 ms sign-in unless
    /// `configure` changes them) and serves its internal agent API.
    pub async fn start(configure: impl FnOnce(&mut DevOptions)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut options = DevOptions::from_map(&BTreeMap::new()).unwrap();
        options.data = dir.path().join("ui-dev");
        options.database_url =
            format!("sqlite:///{}", dir.path().join("ui-dev").join("runtime.db").display());
        options.port = listener.local_addr().unwrap().port();
        options.pace = Duration::from_millis(1);
        options.sign_in_delay = Duration::from_millis(50);
        configure(&mut options);
        let mut config = dev::prepare(&options).await.unwrap();
        config.poll_interval = Duration::from_millis(10);
        let runtime = Arc::new(Runtime::new(config).await.unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let dev = Self { runtime, options, requests, _dir: dir };
        dev.serve(listener);
        dev
    }

    /// Serves the internal agent API on `listener` (another one can join later).
    pub fn serve(&self, listener: TcpListener) {
        let api = Api { runtime: Arc::clone(&self.runtime), requests: Arc::clone(&self.requests) };
        let app = Router::new()
            .route(
                "/internal/v1/projects/{slug}/{*rest}",
                axum::routing::get(internal).post(internal),
            )
            .with_state(api);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
    }

    pub fn data(&self) -> PathBuf {
        self.options.data.clone()
    }

    /// The requests received since the last call.
    pub fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }

    /// Creates a project as the demo's operator, without starting it.
    pub async fn create_project(&self, slug: &str) {
        let project = NewProject {
            slug: slug.into(),
            name: "Scripted".into(),
            mission: "Exercise the scripted engine.".into(),
            success_criteria: "Every step runs.".into(),
            context: String::new(),
            runtime: Some("codex".into()),
            model: None,
            reasoning_effort: None,
        };
        self.runtime.operations().create_project(project, &new_uuid(), &operator()).await.unwrap();
    }

    /// The environment a turn of `slug` gets: the internal API and a fresh capability.
    pub fn turn_env(&self, slug: &str) -> BTreeMap<String, String> {
        let token = self.runtime.supervisor().issue_capability(slug).unwrap();
        BTreeMap::from([
            ("LIMINA_INTERNAL_URL".to_owned(), self.options.internal_url()),
            ("LIMINA_INTERNAL_TOKEN".to_owned(), token),
        ])
    }

    /// Plays one turn of `agent` with `prompt` and returns it with its events.
    pub async fn play(
        &self,
        agent: &dyn AgentSession,
        slug: &str,
        prompt: &str,
    ) -> (Result<RuntimeTurn>, Vec<RuntimeEvent>) {
        let (observer, mut signals) = ChannelObserver::new();
        let request = turn_request(prompt, self.turn_env(slug), observer, &self.data());
        let outcome = agent.run_turn(request).await;
        (outcome, drain(&mut signals))
    }

    /// Artifact ID → status for `slug`.
    pub async fn statuses(&self, slug: &str) -> HashMap<String, String> {
        let artifacts = self.runtime.service().list_artifacts(slug, None).await.unwrap();
        artifacts.into_iter().map(|artifact| (artifact.id, artifact.status)).collect()
    }

    pub async fn close(self) {
        self.runtime.shutdown().await;
        self.runtime.db().close().await;
    }
}

/// The demo's trusted local operator.
pub fn operator() -> Principal {
    Principal::local(DEMO_ACTOR, false)
}

pub fn turn_request(
    prompt: &str,
    runtime_env: BTreeMap<String, String>,
    observer: ChannelObserver,
    workspace: &std::path::Path,
) -> TurnRequest {
    TurnRequest {
        settings: Default::default(),
        prompt: prompt.to_owned(),
        workspace: workspace.to_path_buf(),
        continuation_id: None,
        runtime_env,
        mcp_servers: Vec::new(),
        observer: Arc::new(observer),
    }
}

/// Every event received so far.
pub fn drain(signals: &mut mpsc::UnboundedReceiver<TurnSignal>) -> Vec<RuntimeEvent> {
    let mut events = Vec::new();
    while let Ok(signal) = signals.try_recv() {
        match signal {
            TurnSignal::Event(event) => events.push(event),
            TurnSignal::Continuation(id) => panic!("the scripted engine bound continuation {id}"),
        }
    }
    events
}

/// Events as the Python fixture records them: item IDs numbered in order of appearance.
pub fn numbered(events: &[RuntimeEvent]) -> Value {
    let mut seen: HashMap<String, String> = HashMap::new();
    let events: Vec<Value> = events
        .iter()
        .map(|event| {
            let mut detail = event.detail.clone();
            if let Some(Value::String(item)) = detail.get("item_id") {
                let next = format!("item-{}", seen.len() + 1);
                let numbered = seen.entry(item.clone()).or_insert(next).clone();
                detail.insert("item_id".to_owned(), Value::String(numbered));
            }
            json!({"event_type": event.event_type, "summary": event.summary, "detail": detail})
        })
        .collect();
    Value::Array(events)
}

/// Polls `condition` every 10 ms for up to 10 s.
pub async fn wait_until<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ─── The internal agent API ───────────────────────────────────────────────────

async fn internal(
    State(api): State<Api>,
    method: Method,
    Path((slug, rest)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: Value = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(body) => body,
            Err(_) => return (StatusCode::BAD_REQUEST, "invalid JSON").into_response(),
        }
    };
    api.requests.lock().unwrap().push(Request {
        method: method.to_string(),
        path: rest.clone(),
        body: body.clone(),
    });
    match dispatch(&api.runtime, &method, &slug, &rest, &headers, &body).await {
        Ok((status, value)) => (status, axum::Json(value)).into_response(),
        Err(error) => {
            let status = StatusCode::from_u16(error.http_status()).unwrap();
            (status, axum::Json(error.to_body())).into_response()
        }
    }
}

fn text(body: &Value, key: &str) -> String {
    body.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn version(body: &Value) -> i32 {
    body.get("expected_version").and_then(Value::as_i64).unwrap_or(0) as i32
}

fn reply(status: StatusCode, value: impl Serialize) -> Result<(StatusCode, Value)> {
    Ok((status, serde_json::to_value(value).unwrap()))
}

/// The Python runtime's `internal_api.py`, on the real services.
async fn dispatch(
    runtime: &Runtime,
    method: &Method,
    slug: &str,
    rest: &str,
    headers: &HeaderMap,
    body: &Value,
) -> Result<(StatusCode, Value)> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let actor = runtime.internal_actor(
        header(AUTHORIZATION.as_str()),
        slug,
        header("x-limina-agent-lane"),
    )?;
    let service = runtime.service();
    let segments: Vec<&str> = rest.split('/').collect();
    if method == Method::GET {
        return match segments.as_slice() {
            ["artifacts"] => reply(StatusCode::OK, service.list_artifacts(slug, None).await?),
            ["artifacts", id] => reply(StatusCode::OK, service.get_artifact(slug, id).await?),
            _ => Err(LiminaError::not_found("No such internal route.")),
        };
    }
    let command_id = header("idempotency-key")
        .filter(|key| (1..=64).contains(&key.len()))
        .ok_or_else(|| LiminaError::invariant("An Idempotency-Key header is required."))?
        .to_owned();
    match segments.as_slice() {
        ["hypotheses"] => {
            let new = NewHypothesis {
                title: text(body, "title"),
                statement: text(body, "statement"),
                mechanism: text(body, "mechanism"),
                generalization: text(body, "generalization"),
                shortcut_risks: text(body, "shortcut_risks"),
                test_plan: text(body, "test_plan"),
            };
            reply(
                StatusCode::CREATED,
                service.create_hypothesis(slug, new, &actor, &command_id).await?,
            )
        }
        ["hypotheses", id, "decision"] => {
            let decision = HypothesisDecision {
                status: text(body, "status"),
                conclusion: text(body, "conclusion"),
                expected_version: version(body),
            };
            let decided =
                service.decide_hypothesis(slug, id, decision, &actor, &command_id).await?;
            reply(StatusCode::OK, decided)
        }
        ["experiments"] => {
            let new = NewExperiment {
                hypothesis_id: text(body, "hypothesis_id"),
                title: text(body, "title"),
                objective: text(body, "objective"),
                procedure: text(body, "procedure"),
                success_criteria: text(body, "success_criteria"),
                guardrails: text(body, "guardrails"),
            };
            reply(
                StatusCode::CREATED,
                service.create_experiment(slug, new, &actor, &command_id).await?,
            )
        }
        ["experiments", id, "claim"] => {
            let ttl = body.get("ttl_seconds").and_then(Value::as_i64).unwrap_or(1800);
            reply(
                StatusCode::OK,
                service.claim_experiment(slug, id, ttl, &actor, &command_id).await?,
            )
        }
        ["experiments", id, "observations"] => {
            let evidence = body.get("evidence_ref").and_then(Value::as_str);
            let observed = service
                .append_observation(slug, id, &text(body, "body"), evidence, &actor, &command_id)
                .await?;
            reply(StatusCode::CREATED, observed)
        }
        ["experiments", id, "complete"] => {
            let outcome = ExperimentOutcome {
                results: text(body, "results"),
                analysis: text(body, "analysis"),
                decision: text(body, "decision"),
                expected_version: version(body),
            };
            let completed =
                service.complete_experiment(slug, id, outcome, &actor, &command_id).await?;
            reply(StatusCode::OK, completed)
        }
        ["findings"] => {
            let new = NewFinding {
                experiment_id: text(body, "experiment_id"),
                title: text(body, "title"),
                finding: text(body, "finding"),
                evidence: text(body, "evidence"),
                improvement: text(body, "improvement"),
                remaining_debt: text(body, "remaining_debt"),
                next_move: text(body, "next_move"),
                impact: body.get("impact").and_then(Value::as_str).unwrap_or("HIGH").to_owned(),
            };
            reply(
                StatusCode::CREATED,
                service.publish_finding(slug, new, &actor, &command_id).await?,
            )
        }
        _ => Err(LiminaError::not_found("No such internal route.")),
    }
}
