//! Codex sessions end to end over the real `app-server` protocol, against the fake provider:
//! streaming events, the checkpoint decision, live steering, interrupt, thread resume, provider
//! failures, approvals, and process cleanup.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use limina_domain::retry::RetryPolicy;
use limina_domain::usage::TokenPricing;
use limina_engines::{
    AgentSession, CodexAgentSession, CodexAuthManager, CodexAuthOptions, CodexSessionConfig,
    TurnRequest, turn_output_schema,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use support::{
    Recorder, eventually, fake_provider, process_alive, read_log, received, requests, starts,
};

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    workspace: PathBuf,
    auth: CodexAuthManager,
}

/// A Codex home that already holds an API-key sign-in, with the fake's scenario.
async fn fixture(scenario: Value) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(home.join("auth.json"), json!({"type": "apiKey"}).to_string()).unwrap();
    std::fs::write(home.join("fake-codex.json"), scenario.to_string()).unwrap();
    let options = CodexAuthOptions { codex_bin: fake_provider(), ..CodexAuthOptions::default() };
    let auth = CodexAuthManager::new(&home, options).await.unwrap();
    Fixture { _temp: temp, home, workspace, auth }
}

impl Fixture {
    fn session(&self, sandbox: &str) -> Arc<CodexAgentSession> {
        let config = CodexSessionConfig {
            codex_bin: fake_provider(),
            model: "gpt-test".to_owned(),
            sandbox: sandbox.to_owned(),
            pricing: Some(TokenPricing {
                input_usd_per_million: Decimal::from(2),
                cached_input_usd_per_million: Decimal::from(1),
                output_usd_per_million: Decimal::from(4),
            }),
        };
        Arc::new(CodexAgentSession::new(config, Arc::new(self.auth.clone())))
    }

    fn request(&self, continuation: Option<&str>, recorder: &Arc<Recorder>) -> TurnRequest {
        TurnRequest {
            settings: Default::default(),
            prompt: "Advance the mission.".to_owned(),
            workspace: self.workspace.clone(),
            continuation_id: continuation.map(str::to_owned),
            runtime_env: BTreeMap::from([
                ("LIMINA_INTERNAL_TOKEN".to_owned(), "capability".to_owned()),
                ("FAKE_LEAK".to_owned(), "sensitive-123".to_owned()),
            ]),
            mcp_servers: Vec::new(),
            observer: recorder.clone(),
        }
    }

    fn log(&self) -> Vec<Value> {
        read_log(&self.home.join("fake-codex.log"))
    }

    /// The `start` entry of the turn's app-server (the auth probes have no sandbox override).
    fn turn_start(&self) -> Value {
        starts(&self.log())
            .into_iter()
            .find(|start| start["args"][1] != "cli_auth_credentials_store=\"file\"")
            .expect("a turn app-server")
    }
}

fn kinds_and_tools(recorder: &Recorder) -> Vec<(String, String)> {
    recorder
        .events()
        .iter()
        .map(|event| {
            (
                event.detail_str("kind").unwrap_or_default().to_owned(),
                event.detail_str("tool").unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_turn_streams_events_and_returns_the_checkpoint() {
    let fixture = fixture(json!({"turn": "complete"})).await;
    let session = fixture.session("workspace-write");
    let recorder = Recorder::new();

    let turn = session.run_turn(fixture.request(None, &recorder)).await.unwrap();

    assert_eq!(turn.continuation_id.as_deref(), Some("thread-new"));
    assert_eq!(turn.turn_id, "turn-1");
    assert_eq!(turn.decision.summary, "Checkpoint complete.");
    assert_eq!(turn.decision.status, "COMPLETE");
    let usage = turn.usage.unwrap();
    // The per-turn `last` breakdown, never the cumulative total.
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.cached_input_tokens),
        (Some(100), Some(20), Some(40))
    );
    assert_eq!((usage.reasoning_output_tokens, usage.total_tokens), (Some(8), Some(120)));
    assert_eq!(usage.cost_microusd, Some(240));
    assert_eq!(
        (usage.usage_source.as_deref(), usage.cost_source.as_deref()),
        (Some("provider"), Some("operator_rate"))
    );
    assert_eq!(recorder.continuations(), ["thread-new"]);
    assert_eq!(
        kinds_and_tools(&recorder),
        [
            ("tool".to_owned(), "command".to_owned()),
            ("tool".to_owned(), "command".to_owned()),
            ("message".to_owned(), String::new()),
        ],
        "other turns' items, malformed lines and the checkpoint answer stay out"
    );
    let events = recorder.events();
    assert_eq!(events[0].detail["input"], "pytest -q tests");
    assert_eq!(
        (&events[1].detail["state"], &events[1].detail["exit_code"]),
        (&json!("ok"), &json!(0))
    );
    assert_eq!(events[2].summary, "## Findings");
    assert!(!serde_json::to_string(&events).unwrap().contains("SECRET"));

    let start = fixture.turn_start();
    assert_eq!(
        start["args"],
        json!([
            "--config",
            "sandbox_workspace_write.network_access=true",
            "app-server",
            "--listen",
            "stdio://"
        ])
    );
    assert_eq!(
        Path::new(start["cwd"].as_str().unwrap()).canonicalize().unwrap(),
        fixture.workspace.canonicalize().unwrap()
    );
    let env = &start["env"];
    assert_eq!(env["CODEX_HOME"], fixture.home.to_string_lossy().as_ref());
    assert_eq!(env["LIMINA_INTERNAL_TOKEN"], "capability");
    assert!(env.get("OPENAI_API_KEY").is_none());
    assert!(env.get("CARGO_MANIFEST_DIR").is_none(), "providers start from an empty environment");

    let log = fixture.log();
    let thread = &requests(&log, "thread/start")[0]["params"];
    assert_eq!(
        thread,
        &json!({"approvalPolicy": "on-request", "approvalsReviewer": "auto_review",
                "cwd": fixture.workspace.to_string_lossy(), "model": "gpt-test",
                "sandbox": "workspace-write"})
    );
    let turn_start = &requests(&log, "turn/start")[0]["params"];
    assert_eq!(turn_start["threadId"], "thread-new");
    assert_eq!(turn_start["input"], json!([{"type": "text", "text": "Advance the mission."}]));
    assert_eq!(turn_start["outputSchema"], turn_output_schema());
    assert!(log.iter().any(|entry| entry.get("eof").is_some()), "the session closed stdin");
}

#[tokio::test]
async fn a_thread_resumes_with_its_continuation() {
    let fixture = fixture(json!({"turn": "complete"})).await;
    let session = fixture.session("read-only");
    let recorder = Recorder::new();

    let turn = session.run_turn(fixture.request(Some("thread-prev"), &recorder)).await.unwrap();

    assert_eq!(turn.continuation_id.as_deref(), Some("thread-prev"));
    assert_eq!(recorder.continuations(), ["thread-prev"]);
    let log = fixture.log();
    assert!(requests(&log, "thread/start").is_empty());
    let resume = &requests(&log, "thread/resume")[0]["params"];
    assert_eq!(
        resume,
        &json!({"threadId": "thread-prev", "cwd": fixture.workspace.to_string_lossy(),
                "model": "gpt-test", "sandbox": "read-only"})
    );
    assert_eq!(fixture.turn_start()["args"], json!(["app-server", "--listen", "stdio://"]));
}

#[tokio::test]
async fn live_steering_reaches_the_active_turn_and_blocks_auth_changes() {
    let fixture = fixture(json!({"turn": "steer"})).await;
    let session = fixture.session("full-access");
    let recorder = Recorder::new();
    assert!(!session.steer("too early").await.unwrap());
    assert!(!session.interrupt().await.unwrap());

    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();
    let busy = fixture.auth.logout().await.unwrap_err();
    assert_eq!(busy.code(), "version_conflict", "auth cannot change during a turn");

    assert!(session.steer("reviewer: Change the strategy now.").await.unwrap());
    let turn = running.await.unwrap().unwrap();

    assert_eq!(turn.decision.summary, "Steering incorporated: reviewer: Change the strategy now.");
    let steer = &requests(&fixture.log(), "turn/steer")[0]["params"];
    assert_eq!(
        steer,
        &json!({"threadId": "thread-new", "expectedTurnId": "turn-1",
                "input": [{"type": "text", "text": "reviewer: Change the strategy now."}]})
    );
    assert_eq!(fixture.turn_start()["args"], json!(["app-server", "--listen", "stdio://"]));
    assert!(!session.steer("after the turn").await.unwrap());
    fixture.auth.logout().await.unwrap();
}

#[tokio::test]
async fn an_interrupted_turn_returns_no_decision() {
    let fixture = fixture(json!({"turn": "interrupt"})).await;
    let session = fixture.session("workspace-write");
    let recorder = Recorder::new();
    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();

    assert!(session.interrupt().await.unwrap());
    let error = running.await.unwrap().unwrap_err();

    assert_eq!(error.message, "Codex completed without the required project decision.");
    assert_eq!(
        (&error.details["continuation_id"], &error.details["turn_id"]),
        (&json!("thread-new"), &json!("turn-1"))
    );
    let interrupt = &requests(&fixture.log(), "turn/interrupt")[0]["params"];
    assert_eq!(interrupt, &json!({"threadId": "thread-new", "turnId": "turn-1"}));
}

#[tokio::test]
async fn failed_turns_become_transport_errors_with_their_reason() {
    let fixture = fixture(json!({"turn": "fail"})).await;
    let recorder = Recorder::new();
    let error = fixture
        .session("workspace-write")
        .run_turn(fixture.request(Some("thread-prev"), &recorder))
        .await
        .unwrap_err();

    assert_eq!(error.code(), "transport_error");
    assert_eq!(
        error.message,
        "The managed Codex turn failed before Limina accepted its checkpoint."
    );
    assert_eq!(error.details["reason"], "Upstream model failure");
    assert_eq!(error.details["continuation_id"], "thread-prev");
    assert_eq!(error.details["retryable"], false);
    let keys: Vec<&str> = error.details.keys().map(String::as_str).collect();
    assert_eq!(keys, ["reason", "continuation_id", "retryable"]);
    let failure =
        recorder.events().into_iter().find(|event| event.detail["method"] == "error").unwrap();
    assert_eq!(
        (&failure.detail["kind"], &failure.detail["state"]),
        (&json!("status"), &json!("failed"))
    );
    assert_eq!(failure.detail["text"], "Stream disconnected.");
}

#[tokio::test]
async fn overloaded_codex_is_retryable() {
    let fixture = fixture(json!({"turn": "overloaded"})).await;
    let error = fixture
        .session("workspace-write")
        .run_turn(fixture.request(None, &Recorder::new()))
        .await
        .unwrap_err();
    assert_eq!(error.details["reason"], "JSON-RPC error -32001: Server overloaded; retry later.");
    assert_eq!(error.details["retryable"], true);
    assert_eq!(error.details["continuation_id"], Value::Null);
    assert!(RetryPolicy::is_retryable(&error));
}

#[tokio::test]
async fn crash_reasons_are_redacted() {
    let fixture = fixture(json!({"turn": "crash"})).await;
    let error = fixture
        .session("workspace-write")
        .run_turn(fixture.request(None, &Recorder::new()))
        .await
        .unwrap_err();
    let reason = error.details["reason"].as_str().unwrap();
    assert!(reason.starts_with("Codex process closed stdout."), "{reason}");
    assert!(reason.contains("crashed on purpose [REDACTED]"), "{reason}");
    assert!(!reason.contains("sensitive-123"));
    assert_eq!(error.details["retryable"], false);
}

#[tokio::test]
async fn approval_requests_get_the_sdk_default_answer() {
    let fixture = fixture(json!({"turn": "approval"})).await;
    let turn = fixture
        .session("workspace-write")
        .run_turn(fixture.request(None, &Recorder::new()))
        .await
        .unwrap();
    assert_eq!(turn.decision.status, "COMPLETE");
    let answer =
        received(&fixture.log()).into_iter().find(|message| message["id"] == "approval-1").unwrap();
    assert_eq!(answer, json!({"id": "approval-1", "result": {"decision": "accept"}}));
}

#[tokio::test]
async fn configuration_and_auth_errors_surface_before_any_turn() {
    let fixture = fixture(json!({})).await;
    let error = fixture
        .session("yolo")
        .run_turn(fixture.request(None, &Recorder::new()))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Unsupported Codex sandbox 'yolo'.");

    let mut missing_workspace = fixture.request(None, &Recorder::new());
    missing_workspace.workspace = fixture.workspace.join("missing");
    let error = fixture.session("workspace-write").run_turn(missing_workspace).await.unwrap_err();
    assert_eq!(
        error.message,
        "The managed Codex turn failed before Limina accepted its checkpoint."
    );
    assert!(
        error.details["reason"].as_str().unwrap().starts_with("Working directory does not exist:"),
        "{:?}",
        error.details
    );

    let no_decision = self::fixture(json!({"turn": "no-decision"})).await;
    let error = no_decision
        .session("workspace-write")
        .run_turn(no_decision.request(None, &Recorder::new()))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Codex completed without the required project decision.");

    let signed_out = tempfile::tempdir().unwrap();
    let options = CodexAuthOptions::chatgpt_only(fake_provider());
    let auth = CodexAuthManager::new(signed_out.path().join("codex"), options).await.unwrap();
    let session = CodexAgentSession::new(
        CodexSessionConfig { codex_bin: fake_provider(), ..CodexSessionConfig::default() },
        Arc::new(auth),
    );
    let error = session.run_turn(fixture.request(None, &Recorder::new())).await.unwrap_err();
    assert_eq!(error.code(), "authentication_required");
    assert_eq!(
        error.message,
        "Codex is not authenticated. Configure OPENAI_API_KEY/CODEX_ACCESS_TOKEN on the server or run \
         `limina runtime codex login`."
    );
}

#[tokio::test]
async fn the_provider_is_killed_when_a_turn_is_dropped_or_the_session_closes() {
    let fixture = fixture(json!({"turn": "hang"})).await;
    let session = fixture.session("workspace-write");

    let recorder = Recorder::new();
    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();
    let pid = fixture.turn_start()["pid"].as_u64().unwrap();
    assert!(process_alive(pid));
    running.abort();
    assert!(eventually(|| !process_alive(pid)).await, "dropping the turn kills Codex");
    assert!(!session.steer("nobody is listening").await.unwrap());

    std::fs::remove_file(fixture.home.join("fake-codex.log")).unwrap();
    let recorder = Recorder::new();
    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();
    let pid = fixture.turn_start()["pid"].as_u64().unwrap();
    session.close().await;
    let error = running.await.unwrap().unwrap_err();
    assert_eq!(
        error.message,
        "The managed Codex turn failed before Limina accepted its checkpoint."
    );
    assert!(eventually(|| !process_alive(pid)).await, "closing the session ends Codex");
    assert_eq!(requests(&fixture.log(), "turn/interrupt").len(), 1);
}

#[tokio::test]
async fn project_model_and_reasoning_override_the_session_on_new_and_resumed_turns() {
    let fixture = fixture(json!({"turn":"complete"})).await;
    let session = fixture.session("workspace-write");
    let recorder = Recorder::new();
    for continuation in [None, Some("thread-new")] {
        let mut request = fixture.request(continuation, &recorder);
        request.settings.model = Some("project-model".into());
        request.settings.reasoning_effort = Some("high".into());
        session.run_turn(request).await.unwrap();
    }
    // Clearing the effort uses the chosen model's default, rather than the resumed override.
    let mut reset = fixture.request(Some("thread-new"), &recorder);
    reset.settings.model = Some("project-model".into());
    session.run_turn(reset).await.unwrap();
    let log = fixture.log();
    for method in ["thread/start", "thread/resume"] {
        for request in requests(&log, method) {
            assert_eq!(request["params"]["model"], "project-model");
        }
    }
    let turns = requests(&log, "turn/start");
    assert_eq!(
        turns.iter().map(|r| r["params"]["effort"].as_str().unwrap()).collect::<Vec<_>>(),
        ["high", "high", "low"]
    );
    assert!(turns.iter().all(|r| r["params"]["model"] == "project-model"));
}

#[tokio::test]
async fn model_discovery_returns_advertised_options_without_starting_a_turn() {
    let fixture = fixture(json!({"turn":"complete"})).await;
    let models =
        limina_engines::models::codex_models(&fake_provider(), &fixture.home).await.unwrap();
    assert_eq!(models[0].id, "gpt-test");
    assert_eq!(models[0].reasoning_efforts, ["medium", "xhigh"]);
    assert!(requests(&fixture.log(), "turn/start").is_empty());
}
