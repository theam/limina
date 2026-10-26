//! Fixtures for the application-layer tests: a runtime on a temporary SQLite database, scripted
//! provider sessions (the Python suites' fake agents), an in-process Codex account fake and the
//! principals the Python tests sign in as. Every value here is fake.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use limina_core::{LiminaError, Principal, Result, new_uuid};
use limina_domain::Engine;
use limina_domain::collaboration::MemberChange;
use limina_domain::usage::RuntimeUsage;
use limina_engines::codex::auth::{
    CodexAccount, CodexAuthClient, CodexAuthConnector, DeviceCode, LoginCompletion,
};
use limina_engines::{
    AgentFactory, AgentSession, ProviderError, RuntimeDecision, RuntimeEvent, RuntimeTurn,
    TurnRequest,
};
use limina_server::app::{NewProject, ProjectOperations, ProjectResponse, Runtime, RuntimeConfig};
use limina_store::SecretCipher;
use serde_json::{Map, Value};
use tokio::sync::watch;

// ─── The runtime ──────────────────────────────────────────────────────────────

/// A runtime on its own temporary directory: `runtime.db`, `workspaces/`, Codex homes.
pub struct TestRuntime {
    pub runtime: Runtime,
    pub codex: Arc<FakeCodex>,
    /// The environment server credentials are read from (`ANTHROPIC_API_KEY`, `PATH`, …).
    pub environment: Arc<Mutex<BTreeMap<String, String>>>,
    pub root: PathBuf,
    _dir: tempfile::TempDir,
}

impl TestRuntime {
    pub fn ops(&self) -> &ProjectOperations {
        self.runtime.operations()
    }

    pub fn set_env(&self, name: &str, value: &str) {
        self.environment.lock().unwrap().insert(name.to_owned(), value.to_owned());
    }

    pub fn remove_env(&self, name: &str) {
        self.environment.lock().unwrap().remove(name);
    }

    /// Signs the runtime's shared Codex home in (the fake reads `auth.json`).
    pub fn sign_in_runtime_codex(&self) {
        let home = self.root.join("codex");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("auth.json"), "{}\n").unwrap();
        self.runtime.supervisor().forget_codex_auth_status();
    }

    pub async fn close(self) {
        self.runtime.shutdown().await;
        self.runtime.db().close().await;
    }
}

/// A runtime whose projects all use `session`.
pub async fn runtime(session: Arc<dyn AgentSession>) -> TestRuntime {
    runtime_with(factory(session), |_| {}).await
}

/// A runtime with a custom factory and configuration.
pub async fn runtime_with(
    agent_factory: AgentFactory,
    configure: impl FnOnce(&mut RuntimeConfig),
) -> TestRuntime {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let environment =
        Arc::new(Mutex::new(BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())])));
    let codex = FakeCodex::new("olivia@example.test", Some("pro"));
    let mut config = RuntimeConfig::new(
        format!("sqlite:///{}", root.join("runtime.db").display()),
        root.join("workspaces"),
    );
    config.secret_key = Some(SecretCipher::generate_key());
    config.poll_interval = Duration::from_millis(10);
    config.lease_ttl_seconds = 300;
    config.agent_factory = Some(agent_factory);
    config.codex_connector = Some(Arc::clone(&codex) as Arc<dyn CodexAuthConnector>);
    let source = Arc::clone(&environment);
    config.environment = Arc::new(move || source.lock().unwrap().clone());
    configure(&mut config);
    let runtime = Runtime::new(config).await.unwrap();
    TestRuntime { runtime, codex, environment, root, _dir: dir }
}

/// A factory that gives every project the same session.
pub fn factory(session: Arc<dyn AgentSession>) -> AgentFactory {
    Arc::new(move |_slug: &str, _engine: Engine| Ok(Arc::clone(&session)))
}

/// A factory that also records which engine each project asked for.
pub fn recording_factory(
    session: Arc<dyn AgentSession>,
    engines: Arc<Mutex<Vec<Engine>>>,
) -> AgentFactory {
    Arc::new(move |_slug: &str, engine: Engine| {
        engines.lock().unwrap().push(engine);
        Ok(Arc::clone(&session))
    })
}

// ─── Principals ───────────────────────────────────────────────────────────────

pub fn owner() -> Principal {
    Principal::new("owner", "Olivia Owner").with_email(Some("owner@example.test".into()))
}

pub fn editor() -> Principal {
    Principal::new("editor", "Erin Editor").with_email(Some("editor@example.test".into()))
}

pub fn viewer() -> Principal {
    Principal::new("viewer", "Victor Viewer").with_email(Some("viewer@example.test".into()))
}

/// Another person with the owner's display name.
pub fn namesake() -> Principal {
    Principal::new("other-owner", "Olivia Owner").with_email(Some("other@example.test".into()))
}

pub fn admin() -> Principal {
    Principal::new("instance-admin", "Alice Admin")
        .with_email(Some("admin@example.test".into()))
        .admin(true)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

pub fn key() -> String {
    new_uuid()
}

pub fn new_project(slug: &str) -> NewProject {
    NewProject {
        slug: slug.into(),
        name: "UI ready".into(),
        mission: "Make every project surface queryable.".into(),
        success_criteria: "The typed API supports a collaborative UI.".into(),
        context: String::new(),
        runtime: None,
        model: None,
        reasoning_effort: None,
    }
}

pub async fn create_project(ops: &ProjectOperations, slug: &str) -> ProjectResponse {
    ops.create_project(new_project(slug), &key(), &owner()).await.unwrap()
}

pub async fn add_member(ops: &ProjectOperations, slug: &str, who: &Principal, role: &str) {
    let change = MemberChange {
        subject: who.subject.clone(),
        display_name: who.display_name.clone(),
        email: None,
        role: role.into(),
    };
    ops.set_member(slug, &change, &owner()).await.unwrap();
}

/// Polls `condition` every 10 ms for up to 5 s.
pub async fn wait_until<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A turn result.
pub fn turn(
    continuation: Option<&str>,
    summary: &str,
    status: &str,
    objective: &str,
    next_step: &str,
    blocker: &str,
) -> RuntimeTurn {
    RuntimeTurn {
        continuation_id: continuation.map(str::to_owned),
        turn_id: "turn-1".into(),
        decision: RuntimeDecision {
            mission_update: None,
            summary: summary.into(),
            status: status.into(),
            current_objective: objective.into(),
            next_step: next_step.into(),
            blocker: blocker.into(),
        },
        usage: None,
    }
}

pub fn complete() -> RuntimeTurn {
    turn(Some("thread-1"), "Checkpoint complete.", "COMPLETE", "Verify.", "Review.", "None")
}

pub fn detail(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

pub fn retryable(message: &str) -> LiminaError {
    LiminaError::transport(message).with("retryable", true)
}

// ─── Sessions ─────────────────────────────────────────────────────────────────

/// What one recorded turn received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub settings: limina_engines::ModelSettings,
    pub mcp_servers: Vec<limina_engines::McpServer>,
    pub prompt: String,
    pub workspace: PathBuf,
    pub continuation_id: Option<String>,
    pub runtime_env: BTreeMap<String, String>,
}

impl From<&TurnRequest> for Recorded {
    fn from(request: &TurnRequest) -> Self {
        Self {
            settings: request.settings.clone(),
            mcp_servers: request.mcp_servers.clone(),
            prompt: request.prompt.clone(),
            workspace: request.workspace.clone(),
            continuation_id: request.continuation_id.clone(),
            runtime_env: request.runtime_env.clone(),
        }
    }
}

/// Plays one scripted reply per turn (the last one repeats): binds the continuation, emits the
/// events and returns the result. Records every request.
pub struct ScriptedSession {
    pub catalog_calls: Mutex<Vec<BTreeMap<String, String>>>,
    replies: Mutex<Vec<Reply>>,
    pub turns: Mutex<Vec<Recorded>>,
    pub steering: Mutex<Vec<String>>,
    model: Option<String>,
}

#[derive(Clone)]
pub struct Reply {
    pub continuation: Option<String>,
    pub events: Vec<RuntimeEvent>,
    pub result: std::result::Result<RuntimeTurn, LiminaError>,
    pub panic: Option<&'static str>,
}

impl Reply {
    pub fn ok(turn: RuntimeTurn) -> Self {
        Self { continuation: None, events: Vec::new(), result: Ok(turn), panic: None }
    }

    pub fn err(error: LiminaError) -> Self {
        Self { continuation: None, events: Vec::new(), result: Err(error), panic: None }
    }

    pub fn panics(message: &'static str) -> Self {
        Self { panic: Some(message), ..Self::ok(complete()) }
    }

    #[must_use]
    pub fn continuation(mut self, continuation: &str) -> Self {
        self.continuation = Some(continuation.into());
        self
    }

    #[must_use]
    pub fn event(mut self, event: RuntimeEvent) -> Self {
        self.events.push(event);
        self
    }
}

impl ScriptedSession {
    pub fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
            turns: Mutex::new(Vec::new()),
            steering: Mutex::new(Vec::new()),
            model: Some("test-model".into()),
            catalog_calls: Mutex::new(Vec::new()),
        })
    }

    pub fn always(reply: Reply) -> Arc<Self> {
        Self::new(vec![reply])
    }

    pub fn turns(&self) -> Vec<Recorded> {
        self.turns.lock().unwrap().clone()
    }

    fn next(&self) -> Reply {
        let mut replies = self.replies.lock().unwrap();
        if replies.len() > 1 { replies.remove(0) } else { replies[0].clone() }
    }
}

#[async_trait]
impl AgentSession for ScriptedSession {
    async fn models(
        &self,
        environment: &BTreeMap<String, String>,
    ) -> Result<limina_engines::models::ModelCatalog> {
        self.catalog_calls.lock().unwrap().push(environment.clone());
        Ok(limina_engines::models::ModelCatalog {
            default_model: self.model.clone(),
            models: vec![limina_engines::models::ModelOption {
                id: "test-model".into(),
                label: "Test model".into(),
                resolved_model: None,
                reasoning_efforts: vec!["low".into(), "future".into()],
                default_reasoning_effort: Some("low".into()),
            }],
        })
    }

    async fn run_turn(&self, request: TurnRequest) -> Result<RuntimeTurn> {
        self.turns.lock().unwrap().push(Recorded::from(&request));
        let reply = self.next();
        if let Some(message) = reply.panic {
            panic!("{message}");
        }
        if let Some(continuation) = &reply.continuation {
            request.observer.continuation(continuation).await?;
        }
        for event in reply.events {
            request.observer.event(event).await?;
        }
        tokio::task::yield_now().await;
        reply.result
    }

    async fn steer(&self, message: &str) -> Result<bool> {
        self.steering.lock().unwrap().push(message.to_owned());
        Ok(false)
    }

    async fn interrupt(&self) -> Result<bool> {
        Ok(false)
    }

    async fn close(&self) {}

    fn model(&self) -> Option<String> {
        self.model.clone()
    }
}

/// How a [`BlockingSession`]'s turn ends when it is released.
#[derive(Clone)]
pub enum Release {
    Complete(RuntimeTurn),
    Fail(LiminaError),
}

/// Blocks each turn until it is steered, interrupted or released (Python's `BlockingSession`
/// and `FakeAgentSession`). Steering completes the turn; interrupting ends it with
/// `on_interrupt`.
pub struct BlockingSession {
    started: watch::Sender<usize>,
    release: watch::Sender<Option<Release>>,
    active: AtomicBool,
    pub steering: Mutex<Vec<String>>,
    pub turns: Mutex<Vec<Recorded>>,
    pub interrupts: AtomicUsize,
    pub closed: AtomicBool,
    on_steer: RuntimeTurn,
    on_interrupt: Release,
}

impl BlockingSession {
    pub fn new(on_steer: RuntimeTurn, on_interrupt: Release) -> Arc<Self> {
        Arc::new(Self {
            started: watch::Sender::new(0),
            release: watch::Sender::new(None),
            active: AtomicBool::new(false),
            steering: Mutex::new(Vec::new()),
            turns: Mutex::new(Vec::new()),
            interrupts: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            on_steer,
            on_interrupt,
        })
    }

    /// Completes on steering; an interruption fails the turn.
    pub fn steerable() -> Arc<Self> {
        Self::new(
            turn(Some("thread-live"), "Steered.", "COMPLETE", "Done", "Review", "None"),
            Release::Fail(LiminaError::transport("The managed turn was interrupted.")),
        )
    }

    /// Waits until `count` turns have started.
    pub async fn started(&self, count: usize) {
        let mut started = self.started.subscribe();
        tokio::time::timeout(Duration::from_secs(5), started.wait_for(|started| *started >= count))
            .await
            .expect("the turn never started")
            .unwrap();
    }

    pub fn release(&self, release: Release) {
        self.release.send_replace(Some(release));
    }

    pub fn turns(&self) -> Vec<Recorded> {
        self.turns.lock().unwrap().clone()
    }
}

#[async_trait]
impl AgentSession for BlockingSession {
    async fn run_turn(&self, request: TurnRequest) -> Result<RuntimeTurn> {
        self.turns.lock().unwrap().push(Recorded::from(&request));
        request.observer.continuation("thread-live").await?;
        let mut release = self.release.subscribe();
        self.release.send_replace(None);
        release.borrow_and_update();
        self.active.store(true, Ordering::SeqCst);
        self.started.send_modify(|started| *started += 1);
        let outcome = release.wait_for(Option::is_some).await.unwrap().clone();
        self.active.store(false, Ordering::SeqCst);
        match outcome {
            Some(Release::Complete(turn)) => Ok(turn),
            Some(Release::Fail(error)) => Err(error),
            None => Err(LiminaError::transport("released without an outcome")),
        }
    }

    async fn steer(&self, message: &str) -> Result<bool> {
        self.steering.lock().unwrap().push(message.to_owned());
        if !self.active.load(Ordering::SeqCst) {
            return Ok(false);
        }
        self.release(Release::Complete(self.on_steer.clone()));
        Ok(true)
    }

    async fn interrupt(&self) -> Result<bool> {
        self.interrupts.fetch_add(1, Ordering::SeqCst);
        if !self.active.load(Ordering::SeqCst) {
            return Ok(false);
        }
        self.release(self.on_interrupt.clone());
        Ok(true)
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn model(&self) -> Option<String> {
        Some("blocking".into())
    }
}

// ─── Codex accounts ───────────────────────────────────────────────────────────

/// An in-process Codex (Python's `FakeCodex`): a home is signed in while its `auth.json` exists.
/// Device logins succeed at once by writing it, unless logins are held.
pub struct FakeCodex {
    email: String,
    plan: Option<String>,
    hold_logins: AtomicBool,
    released: watch::Sender<bool>,
}

impl FakeCodex {
    pub fn new(email: &str, plan: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            email: email.into(),
            plan: plan.map(str::to_owned),
            hold_logins: AtomicBool::new(false),
            released: watch::Sender::new(false),
        })
    }

    /// Keeps device logins pending until they are cancelled.
    pub fn hold_logins(&self) {
        self.hold_logins.store(true, Ordering::SeqCst);
        self.released.send_replace(false);
    }
}

struct FakeCodexClient {
    codex: Arc<FakeCodex>,
    home: PathBuf,
}

#[async_trait]
impl CodexAuthConnector for FakeCodex {
    async fn connect(
        &self,
        environment: BTreeMap<String, String>,
    ) -> std::result::Result<Arc<dyn CodexAuthClient>, ProviderError> {
        let codex = Arc::new(Self {
            email: self.email.clone(),
            plan: self.plan.clone(),
            hold_logins: AtomicBool::new(self.hold_logins.load(Ordering::SeqCst)),
            released: self.released.clone(),
        });
        Ok(Arc::new(FakeCodexClient { codex, home: PathBuf::from(&environment["CODEX_HOME"]) }))
    }
}

fn auth_file(home: &Path) -> PathBuf {
    home.join("auth.json")
}

#[async_trait]
impl CodexAuthClient for FakeCodexClient {
    async fn account(&self) -> std::result::Result<Option<CodexAccount>, ProviderError> {
        if !auth_file(&self.home).exists() {
            return Ok(None);
        }
        Ok(Some(CodexAccount {
            account_type: "chatgpt".into(),
            email: Some(self.codex.email.clone()),
            plan_type: self.codex.plan.clone(),
        }))
    }

    async fn login_api_key(&self, _api_key: &str) -> std::result::Result<(), ProviderError> {
        std::fs::write(auth_file(&self.home), "{}\n").unwrap();
        Ok(())
    }

    async fn login_chatgpt_device_code(&self) -> std::result::Result<DeviceCode, ProviderError> {
        Ok(DeviceCode {
            login_id: "login-1".into(),
            verification_url: "https://example.invalid/device".into(),
            user_code: "WXYZ-1234".into(),
        })
    }

    async fn wait_login(
        &self,
        _login_id: &str,
    ) -> std::result::Result<LoginCompletion, ProviderError> {
        if self.codex.hold_logins.load(Ordering::SeqCst) {
            let mut released = self.codex.released.subscribe();
            let _ = released.wait_for(|released| *released).await;
            return Ok(LoginCompletion { success: false, error: None });
        }
        std::fs::write(auth_file(&self.home), "{}\n").unwrap();
        Ok(LoginCompletion { success: true, error: None })
    }

    async fn cancel_login(&self, _login_id: &str) -> std::result::Result<(), ProviderError> {
        self.codex.released.send_replace(true);
        Ok(())
    }

    async fn logout(&self) -> std::result::Result<(), ProviderError> {
        let _ = std::fs::remove_file(auth_file(&self.home));
        Ok(())
    }

    async fn close(&self) {}
}

/// Default usage for turns that report tokens.
pub fn usage(input: i64, output: i64, cached: i64, cost: i64) -> RuntimeUsage {
    RuntimeUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        cached_input_tokens: Some(cached),
        cost_microusd: Some(cost),
        ..RuntimeUsage::default()
    }
}
