//! The project supervisor: ports of the supervisor tests in `tests/test_cloud_runtime.py` and the
//! `DurableRetryTests` of `tests/test_cloud_enterprise_hardening.py`, with the same fake-session
//! approach, plus the lifecycle, shutdown and isolation guarantees of the Rust port.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use app_support::*;
use limina_core::LiminaError;
use limina_domain::Engine;
use limina_domain::project::NewChallenge;
use limina_domain::retry::RetryPolicy;
use limina_domain::service::ChallengeService;
use limina_engines::RuntimeEvent;
use limina_server::app::{Delivery, Guidance};
use limina_store::args;
use serde_json::json;

async fn retrieval(service: &ChallengeService) {
    service
        .create_challenge(
            NewChallenge {
                slug: "retrieval".into(),
                name: "Retrieval quality".into(),
                objective: "Improve retrieval quality.".into(),
                context: "Existing hybrid baseline.".into(),
                success_criteria: "Improve NDCG without a latency regression.".into(),
                ..NewChallenge::default()
            },
            "owner",
            &key(),
        )
        .await
        .unwrap();
}

async fn start(service: &ChallengeService, slug: &str) {
    service.change_project_state(slug, "start", "owner", &key()).await.unwrap();
}

async fn events(service: &ChallengeService, slug: &str) -> Vec<limina_domain::service::EventView> {
    service.events(slug, 0, 1000).await.unwrap()
}

async fn event_types(service: &ChallengeService, slug: &str) -> Vec<String> {
    events(service, slug).await.into_iter().map(|event| event.event_type).collect()
}

#[tokio::test]
async fn an_assessment_cannot_cite_a_note_outside_the_project() {
    use limina_engines::turn::MissionUpdate;
    let mut response = complete();
    response.decision.mission_update = Some(Box::new(MissionUpdate {
        headline: "An unverified claim".into(),
        established: "Claimed result".into(),
        ruled_out: "Nothing yet".into(),
        uncertain: "Evidence is missing".into(),
        evidence: vec!["F999".into()],
        decision: None,
    }));
    let test = runtime(ScriptedSession::always(Reply::ok(response))).await;
    let service = test.runtime.service();
    retrieval(service).await;
    start(service, "retrieval").await;
    test.runtime.supervisor().ensure_running("retrieval").await.unwrap();
    test.runtime.supervisor().join("retrieval").await;
    assert!(service.latest_event("retrieval", "runtime.turn_completed").await.unwrap().is_none());
    assert_eq!(coordinator(service, "retrieval").await.status, "FAILED");
    test.close().await;
}

async fn coordinator(
    service: &ChallengeService,
    slug: &str,
) -> limina_domain::service::CoordinatorView {
    service.get_challenge(slug).await.unwrap().coordinator.unwrap()
}

#[tokio::test]
async fn supervisor_owns_thread_and_consumes_guidance_after_success() {
    let session = ScriptedSession::always(
        Reply::ok(turn(
            Some("thread-123"),
            "Checkpoint complete.",
            "COMPLETE",
            "Verify the result.",
            "Present the finding for review.",
            "None",
        ))
        .continuation("thread-123"),
    );
    let engines = Arc::new(Mutex::new(Vec::new()));
    let test = runtime_with(recording_factory(session.clone(), Arc::clone(&engines)), |_| {}).await;
    test.set_env("UNRELATED_SECRET", "never-copy");
    let service = test.runtime.service();
    retrieval(service).await;
    let message = service
        .send_message(
            "retrieval",
            "STEER",
            "Prioritize the latency guardrail.",
            None,
            "owner",
            &key(),
        )
        .await
        .unwrap();
    service
        .set_variable("retrieval", "SOURCE_URL", "https://example.invalid/source", "owner", &key())
        .await
        .unwrap();
    service
        .set_secret("retrieval", "SOURCE_TOKEN", "scoped-secret", "owner", &key())
        .await
        .unwrap();
    start(service, "retrieval").await;

    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    supervisor.join("retrieval").await;
    supervisor.shutdown().await;

    let status = service.status("retrieval").await.unwrap();
    let state = status.challenge.coordinator.unwrap();
    assert_eq!(state.continuation_id.as_deref(), Some("thread-123"));
    assert_eq!(*engines.lock().unwrap(), vec![Engine::Codex]);
    assert_eq!(state.inbox_cursor, message.sequence);
    assert!(service.inbox("retrieval", 0, true).await.unwrap().is_empty());
    let turns = session.turns();
    assert_eq!(turns.len(), 1);
    let prompt = &turns[0].prompt;
    assert!(prompt.contains("Prioritize the latency guardrail"));
    assert!(prompt.contains("SOURCE_URL"));
    assert!(prompt.contains("SOURCE_TOKEN"));
    assert!(!prompt.contains("scoped-secret"));
    let env = &turns[0].runtime_env;
    assert_eq!(env["SOURCE_URL"], "https://example.invalid/source");
    assert_eq!(env["SOURCE_TOKEN"], "scoped-secret");
    assert!(!env.contains_key("UNRELATED_SECRET"));
    assert!(!env.contains_key("LIMINA_DATABASE_URL"));
    assert_eq!(env["LIMINA_PROJECT"], "retrieval");
    assert_eq!(env["LIMINA_CLI"], std::env::current_exe().unwrap().to_string_lossy());
    assert_eq!(env["LIMINA_CHALLENGE"], "retrieval");
    assert_eq!(env["LIMINA_INTERNAL_URL"], "http://127.0.0.1:7433");
    assert!(env["LIMINA_ACTOR"].starts_with("limina:"));
    // New in the Rust runtime: `limina _agent` resolves to the running build first.
    let directory = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let path: Vec<_> = std::env::split_paths(&env["PATH"]).collect();
    assert_eq!(path, vec![directory, "/usr/bin".into(), "/bin".into()]);
    assert_eq!(turns[0].workspace, test.runtime.workspace_root().join("retrieval"));
    assert!(turns[0].workspace.is_dir());

    let events = events(service, "retrieval").await;
    let sequences: Vec<i32> = events.iter().map(|event| event.sequence).collect();
    let mut sorted = sequences.clone();
    sorted.sort_unstable();
    assert_eq!(sequences, sorted);
    let checkpoint =
        events.iter().rev().find(|event| event.event_type == "coordinator.checkpointed").unwrap();
    assert!(checkpoint.actor.starts_with("limina:"));
    assert_eq!(checkpoint.payload["messages_acknowledged"], 1);
    test.close().await;
}

#[tokio::test]
async fn resource_rotation_restarts_the_turn_with_a_fresh_environment() {
    let session = BlockingSession::new(
        complete(),
        Release::Fail(LiminaError::transport("resource refresh interrupted the stale turn")),
    );
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    let supervisor = test.runtime.supervisor();
    retrieval(service).await;
    start(service, "retrieval").await;

    supervisor.ensure_running("retrieval").await.unwrap();
    session.started(1).await;
    service
        .set_secret("retrieval", "ROTATED_TOKEN", "fresh-secret", "owner", &key())
        .await
        .unwrap();
    let guidance =
        Guidance::new("retrieval", "ANSWER", "Project secret 'ROTATED_TOKEN' was set.", "owner")
            .queued_only();
    supervisor.submit_message(guidance).await.unwrap();
    assert!(supervisor.refresh_resources("retrieval").await);
    session.started(2).await;
    session.release(Release::Complete(turn(
        Some("fresh-continuation"),
        "Used the refreshed resources.",
        "COMPLETE",
        "Verify resource refresh.",
        "Present the result.",
        "None",
    )));
    supervisor.join("retrieval").await;
    supervisor.shutdown().await;

    let turns = session.turns();
    assert!(!turns[0].runtime_env.contains_key("ROTATED_TOKEN"));
    assert_eq!(turns[1].runtime_env["ROTATED_TOKEN"], "fresh-secret");
    assert!(service.inbox("retrieval", 0, true).await.unwrap().is_empty());
    assert!(
        event_types(service, "retrieval").await.contains(&"runtime.resources_refreshed".into())
    );
    test.close().await;
}

#[tokio::test]
async fn refreshing_without_an_active_turn_only_wakes_the_project() {
    let session = ScriptedSession::always(Reply::ok(complete()));
    let test = runtime(session).await;
    retrieval(test.runtime.service()).await;
    assert!(!test.runtime.supervisor().refresh_resources("retrieval").await);
    test.close().await;
}

#[tokio::test]
async fn live_steering_reaches_the_active_managed_turn() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    let supervisor = test.runtime.supervisor();
    retrieval(service).await;
    start(service, "retrieval").await;

    supervisor.ensure_running("retrieval").await.unwrap();
    session.started(1).await;
    let delivery = supervisor
        .submit_message(Guidance::new("retrieval", "STEER", "Change the strategy now.", "reviewer"))
        .await
        .unwrap();
    supervisor.join("retrieval").await;
    supervisor.shutdown().await;

    assert_eq!(delivery.delivery, Delivery::Live);
    assert_eq!(*session.steering.lock().unwrap(), vec!["reviewer: Change the strategy now."]);
    assert!(service.inbox("retrieval", 0, true).await.unwrap().is_empty());
    let state = coordinator(service, "retrieval").await;
    assert_eq!(state.inbox_cursor, delivery.message.sequence);
    test.close().await;
}

#[tokio::test]
async fn runtime_events_and_decisions_redact_project_secret_values() {
    let session = ScriptedSession::always(
        Reply::ok(turn(
            Some("thread"),
            "Found sensitive-123",
            "COMPLETE",
            "Rotate sensitive-123",
            "Ask for access",
            "sensitive-123 expired",
        ))
        .event(RuntimeEvent::new(
            "runtime.codex",
            "Used sensitive-123",
            detail(json!({"nested": ["sensitive-123", {"value": "prefix-sensitive-123"}]})),
        )),
    );
    let test = runtime(session).await;
    let service = test.runtime.service();
    retrieval(service).await;
    service.set_secret("retrieval", "API_TOKEN", "sensitive-123", "owner", &key()).await.unwrap();
    start(service, "retrieval").await;

    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    supervisor.join("retrieval").await;

    let stored = serde_json::to_string(&events(service, "retrieval").await).unwrap();
    assert!(!stored.contains("sensitive-123"), "{stored}");
    assert!(stored.contains("[REDACTED]"));
    let transcript = events(service, "retrieval")
        .await
        .into_iter()
        .find(|event| event.event_type == "runtime.codex")
        .unwrap();
    assert_eq!(transcript.payload["summary"], "Used [REDACTED]");
    assert_eq!(transcript.payload["nested"], json!(["[REDACTED]", {"value": "prefix-[REDACTED]"}]));
    let state = coordinator(service, "retrieval").await;
    assert_eq!(state.current_objective, "Rotate [REDACTED]");
    assert_eq!(state.blocker, "[REDACTED] expired");
    let runs = test.runtime.collaboration().runs("retrieval", None, None, 50).await.unwrap();
    assert_eq!(runs.items[0].summary, "Found [REDACTED]");
    test.close().await;
}

#[tokio::test]
async fn failed_turns_redact_secrets_from_their_error() {
    let session = ScriptedSession::always(Reply::err(LiminaError::transport(
        "The provider echoed sensitive-123 back.",
    )));
    let test = runtime(session).await;
    let service = test.runtime.service();
    retrieval(service).await;
    service.set_secret("retrieval", "API_TOKEN", "sensitive-123", "owner", &key()).await.unwrap();
    start(service, "retrieval").await;

    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    supervisor.join("retrieval").await;

    let failed = events(service, "retrieval")
        .await
        .into_iter()
        .find(|event| event.event_type == "runtime.turn_failed")
        .unwrap();
    assert_eq!(failed.payload["summary"], "The managed turn failed before checkpointing.");
    assert_eq!(failed.payload["error"], "The provider echoed [REDACTED] back.");
    let run =
        &test.runtime.collaboration().runs("retrieval", None, None, 50).await.unwrap().items[0];
    assert_eq!(run.status, "FAILED");
    let error = run.error.as_ref().unwrap();
    assert_eq!(error.code.as_deref(), Some("transport_error"));
    assert_eq!(error.message.as_deref(), Some("The provider echoed [REDACTED] back."));
    test.close().await;
}

#[tokio::test]
async fn run_record_start_failure_releases_capability_and_coordinator() {
    let session = ScriptedSession::always(Reply::ok(complete()));
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    service
        .create_challenge(
            NewChallenge {
                slug: "startup-failure".into(),
                name: "Startup failure".into(),
                objective: "Release partial runtime state.".into(),
                success_criteria: "No heartbeat or capability survives.".into(),
                ..NewChallenge::default()
            },
            "owner",
            &key(),
        )
        .await
        .unwrap();
    start(service, "startup-failure").await;
    // The run record cannot be written: Python's `start_run` mock raising "database unavailable".
    let mut tx = test.runtime.db().begin_write().await.unwrap();
    tx.execute(
        "CREATE TRIGGER refuse_runs BEFORE INSERT ON runtime_runs \
         BEGIN SELECT RAISE(ABORT, 'database unavailable'); END",
        args![],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("startup-failure").await.unwrap();
    supervisor.join("startup-failure").await;

    let state = coordinator(service, "startup-failure").await;
    assert_eq!(state.worker_id, None);
    assert_eq!(supervisor.issued_capabilities(), 0);
    assert_eq!(supervisor.active_turns(), 0);
    assert!(session.turns().is_empty());
    // The coordinator lease was released: another worker can take it.
    service.claim_coordinator("startup-failure", 60, "limina:other", &key()).await.unwrap();
    // The loop recorded the failure, like any other.
    assert_eq!(state.status, "FAILED");
    assert!(event_types(service, "startup-failure").await.contains(&"runtime.failed".into()));
    test.close().await;
}

#[tokio::test]
async fn retry_attempts_have_distinct_runs_and_clear_wake_state() {
    let session = ScriptedSession::new(vec![
        Reply::err(retryable("provider overloaded")),
        Reply::ok(turn(Some("continuation"), "Recovered.", "COMPLETE", "Done", "Review", "None")),
    ]);
    let test = runtime_with(factory(session.clone()), |config| {
        config.retry_policy = RetryPolicy { delays_seconds: vec![0.0] };
    })
    .await;
    let service = test.runtime.service();
    retrieval(service).await;
    start(service, "retrieval").await;

    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    wait_until("the retried turn to complete", || async {
        coordinator(service, "retrieval").await.status == "COMPLETE"
    })
    .await;
    supervisor.join("retrieval").await;

    let state = coordinator(service, "retrieval").await;
    assert_eq!(state.status, "COMPLETE");
    assert_eq!(state.wake_at, None);
    let runs = test.runtime.collaboration().runs("retrieval", None, None, 50).await.unwrap();
    let statuses: Vec<&str> = runs.items.iter().map(|run| run.status.as_str()).collect();
    assert_eq!(statuses, ["COMPLETED", "FAILED"]);
    let retries: Vec<i32> = runs.items.iter().map(|run| run.retry_count).collect();
    assert_eq!(retries, [1, 0]);
    let scheduled = events(service, "retrieval")
        .await
        .into_iter()
        .find(|event| event.event_type == "runtime.retry_scheduled")
        .unwrap();
    assert_eq!(scheduled.payload["retry_count"], 1);
    assert_eq!(scheduled.payload["error"], "provider overloaded");
    assert_eq!(scheduled.payload["retryable"], true);
    assert!(scheduled.payload["wake_at"].as_str().unwrap().ends_with("+00:00"));
    assert!(scheduled.payload.get("run_id").is_none());
    test.close().await;
}

#[tokio::test]
async fn exhausted_retries_fail_the_project_and_wait_for_intervention() {
    let session = ScriptedSession::always(Reply::err(
        retryable("provider overloaded").with("reason", "server_overloaded"),
    ));
    let test = runtime_with(factory(session.clone()), |config| {
        config.retry_policy = RetryPolicy { delays_seconds: vec![0.0] };
    })
    .await;
    let service = test.runtime.service();
    retrieval(service).await;
    start(service, "retrieval").await;

    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    supervisor.join("retrieval").await;

    let state = coordinator(service, "retrieval").await;
    assert_eq!(state.status, "FAILED");
    assert_eq!(state.next_step, "Resolve the runtime failure and resume the project.");
    assert_eq!(state.blocker, "server_overloaded");
    assert_eq!(state.worker_id, None);
    let runs = test.runtime.collaboration().runs("retrieval", None, None, 50).await.unwrap();
    let retries: Vec<(String, i32)> =
        runs.items.iter().map(|run| (run.status.clone(), run.retry_count)).collect();
    assert_eq!(retries, [("FAILED".into(), 1), ("FAILED".into(), 0)]);
    let failed = events(service, "retrieval")
        .await
        .into_iter()
        .find(|event| event.event_type == "runtime.failed")
        .unwrap();
    assert_eq!(
        failed.payload,
        json!({
            "summary": "The managed runtime failed and will wait for intervention.",
            "retryable": true,
            "reason": "server_overloaded",
            "error": "provider overloaded",
        })
    );
    test.close().await;
}

#[tokio::test]
async fn non_retryable_failures_fail_at_once() {
    let session = ScriptedSession::always(Reply::err(LiminaError::transport("Codex refused.")));
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    retrieval(service).await;
    start(service, "retrieval").await;
    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    supervisor.join("retrieval").await;

    assert_eq!(session.turns().len(), 1);
    let state = coordinator(service, "retrieval").await;
    assert_eq!((state.status.as_str(), state.blocker.as_str()), ("FAILED", "Codex refused."));
    let types = event_types(service, "retrieval").await;
    assert!(types.contains(&"runtime.turn_failed".into()));
    assert!(!types.contains(&"runtime.retry_scheduled".into()));
    test.close().await;
}

#[tokio::test]
async fn pausing_interrupts_the_active_turn_and_ends_the_loop() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    let supervisor = test.runtime.supervisor();
    retrieval(service).await;
    start(service, "retrieval").await;
    supervisor.ensure_running("retrieval").await.unwrap();
    session.started(1).await;

    let delivery = supervisor
        .interrupt("retrieval", "Olivia Owner", "Pause and await human direction.", Some("stop-1"))
        .await
        .unwrap();
    supervisor.join("retrieval").await;

    assert_eq!(delivery.delivery, Delivery::Live);
    assert_eq!(delivery.message.kind, "INTERRUPT");
    let state = coordinator(service, "retrieval").await;
    assert_eq!(state.status, "PAUSED");
    let runs = test.runtime.collaboration().runs("retrieval", None, None, 50).await.unwrap();
    let run = &runs.items[0];
    assert_eq!(run.status, "INTERRUPTED");
    assert_eq!(run.summary, "The managed turn was interrupted by a lifecycle change.");
    assert_eq!(run.error, None);
    let types = event_types(service, "retrieval").await;
    assert!(types.contains(&"runtime.turn_interrupted".into()));
    assert!(types.contains(&"runtime.interrupted".into()));
    assert!(types.contains(&"project.paused".into()));
    // The pause is idempotent on `<command_id>:pause`.
    supervisor.interrupt("retrieval", "Olivia Owner", "Again.", Some("stop-1")).await.unwrap();
    test.close().await;
}

#[tokio::test]
async fn shutdown_cancels_active_turns_and_closes_sessions() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    let supervisor = test.runtime.supervisor();
    retrieval(service).await;
    start(service, "retrieval").await;
    supervisor.ensure_running("retrieval").await.unwrap();
    session.started(1).await;
    assert_eq!(supervisor.active_turns(), 1);
    assert_eq!(supervisor.issued_capabilities(), 1);

    supervisor.shutdown().await;

    assert!(session.closed.load(Ordering::SeqCst));
    assert_eq!(supervisor.active_turns(), 0);
    assert_eq!(supervisor.issued_capabilities(), 0);
    assert!(!supervisor.is_running("retrieval"));
    let runs = test.runtime.collaboration().runs("retrieval", None, None, 50).await.unwrap();
    assert_eq!(
        (runs.items[0].status.as_str(), runs.items[0].summary.as_str()),
        ("INTERRUPTED", "Runtime task was cancelled.")
    );
    // The lease was released and the project stays RUNNING for the next start.
    service.claim_coordinator("retrieval", 60, "limina:next", &key()).await.unwrap();
    assert_eq!(coordinator(service, "retrieval").await.status, "RUNNING");
    let error = supervisor.ensure_running("retrieval").await.unwrap_err();
    assert_eq!(error.message, "Project supervisor is closed.");
    test.close().await;
}

#[tokio::test]
async fn a_panicking_session_fails_its_own_project_only() {
    let healthy = ScriptedSession::always(Reply::ok(complete()));
    let broken = ScriptedSession::always(Reply::panics("provider bug"));
    let sessions = (healthy.clone(), broken.clone());
    let factory: limina_engines::AgentFactory = Arc::new(move |slug: &str, _engine| {
        let session: Arc<dyn limina_engines::AgentSession> =
            if slug == "broken" { sessions.1.clone() } else { sessions.0.clone() };
        Ok(session)
    });
    let test = runtime_with(factory, |_| {}).await;
    let ops = test.ops();
    for slug in ["broken", "healthy"] {
        create_project(ops, slug).await;
        ops.apply_lifecycle(slug, "start", &key(), &owner()).await.unwrap();
    }
    let supervisor = test.runtime.supervisor();
    supervisor.join("broken").await;
    supervisor.join("healthy").await;

    let service = test.runtime.service();
    assert_eq!(coordinator(service, "healthy").await.status, "COMPLETE");
    assert_eq!(coordinator(service, "broken").await.status, "FAILED");
    let runs = test.runtime.collaboration().runs("broken", None, None, 50).await.unwrap();
    let error = runs.items[0].error.as_ref().unwrap();
    assert_eq!(error.code.as_deref(), Some("runtime_error"));
    assert_eq!(error.message.as_deref(), Some("provider bug"));
    let failed = events(service, "broken")
        .await
        .into_iter()
        .find(|event| event.event_type == "runtime.failed")
        .unwrap();
    assert_eq!(failed.payload["error"], "The managed runtime raised an unexpected internal error.");
    assert_eq!(supervisor.active_turns(), 0);
    test.close().await;
}

#[tokio::test]
async fn internal_capabilities_last_for_one_turn() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    let supervisor = test.runtime.supervisor();
    retrieval(service).await;
    start(service, "retrieval").await;
    supervisor.ensure_running("retrieval").await.unwrap();
    session.started(1).await;

    let env = session.turns()[0].runtime_env.clone();
    let token = env["LIMINA_INTERNAL_TOKEN"].clone();
    assert_eq!(token.len(), 43);
    let bearer = format!("Bearer {token}");
    let actor = test.runtime.internal_actor(Some(&bearer), "retrieval", Some("lane-1")).unwrap();
    assert_eq!(actor, format!("{}:lane-1", supervisor.runtime_id()));
    assert_eq!(actor, format!("{}:lane-1", env["LIMINA_ACTOR"]));
    let other = test.runtime.internal_actor(Some(&bearer), "other", None).unwrap_err();
    assert_eq!(
        (other.code(), other.message.as_str()),
        ("authentication_required", "The internal project capability is invalid or expired.")
    );

    session.release(Release::Complete(complete()));
    supervisor.join("retrieval").await;
    let expired = test.runtime.internal_actor(Some(&bearer), "retrieval", None).unwrap_err();
    assert_eq!(expired.http_status(), 401);
    test.close().await;
}

#[tokio::test]
async fn waiting_projects_wake_when_guidance_arrives() {
    let session = ScriptedSession::new(vec![
        Reply::ok(turn(Some("t"), "Need input.", "WAITING", "Decide", "Ask", "Which split?")),
        Reply::ok(complete()),
    ]);
    let test = runtime_with(factory(session.clone()), |config| {
        // Only the wake-up can start the second turn quickly.
        config.poll_interval = std::time::Duration::from_secs(30);
    })
    .await;
    let service = test.runtime.service();
    let supervisor = test.runtime.supervisor();
    retrieval(service).await;
    start(service, "retrieval").await;
    let mut changes = test.runtime.notifier().subscribe("retrieval");
    supervisor.ensure_running("retrieval").await.unwrap();
    wait_until("the project to wait", || async {
        coordinator(service, "retrieval").await.status == "WAITING"
    })
    .await;
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();

    supervisor
        .submit_message(Guidance::new("retrieval", "ANSWER", "Use the dev split.", "owner"))
        .await
        .unwrap();
    supervisor.join("retrieval").await;

    assert_eq!(session.turns().len(), 2);
    assert!(session.turns()[1].prompt.contains("[ANSWER] owner: Use the dev split."));
    assert_eq!(session.turns()[1].continuation_id.as_deref(), Some("t"));
    assert_eq!(coordinator(service, "retrieval").await.status, "COMPLETE");
    assert!(changes.has_changed().unwrap());
    test.close().await;
}

#[tokio::test]
async fn recover_resumes_projects_left_running() {
    let session = ScriptedSession::always(Reply::ok(complete()));
    let test = runtime(session.clone()).await;
    let ops = test.ops();
    create_project(ops, "resumed").await;
    create_project(ops, "draft").await;
    test.runtime.service().change_project_state("resumed", "start", "owner", &key()).await.unwrap();

    test.runtime.start().await.unwrap();
    let supervisor = test.runtime.supervisor();
    supervisor.join("resumed").await;

    assert!(!supervisor.is_running("draft"));
    assert_eq!(session.turns().len(), 1);
    assert_eq!(coordinator(test.runtime.service(), "resumed").await.status, "COMPLETE");
    test.close().await;
}

#[tokio::test]
async fn unsafe_legacy_resources_are_blocked_and_reported() {
    let session = ScriptedSession::always(Reply::ok(complete()));
    let test = runtime(session.clone()).await;
    let service = test.runtime.service();
    retrieval(service).await;
    // A row written before the reserved-name policy existed.
    let challenge = service.get_challenge("retrieval").await.unwrap();
    let mut tx = test.runtime.db().begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO project_resources (id, challenge_id, name, resource_type, value, \
         secret_ciphertext, status, created_by, created_at, updated_at) \
         VALUES ($1, $2, 'NODE_OPTIONS', 'VARIABLE', '--require evil', NULL, 'ACTIVE', 'owner', \
         $3, $3)",
        args![key(), &challenge.id, limina_core::now()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    start(service, "retrieval").await;
    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    supervisor.join("retrieval").await;

    assert!(!session.turns()[0].runtime_env.contains_key("NODE_OPTIONS"));
    let blocked = events(service, "retrieval")
        .await
        .into_iter()
        .find(|event| event.event_type == "runtime.resources_blocked")
        .unwrap();
    assert_eq!(blocked.payload["names"], json!(["NODE_OPTIONS"]));
    assert!(blocked.payload["run_id"].is_string());
    test.close().await;
}

#[tokio::test]
async fn runs_count_tool_calls_and_record_usage() {
    let tool = |phase: &str| {
        RuntimeEvent::new(
            "runtime.codex",
            "Using command",
            detail(json!({"kind": "tool", "tool": "command", "phase": phase, "item_id": "c1"})),
        )
    };
    let mut finished = complete();
    finished.usage = Some(usage(120, 30, 10, 500));
    finished.turn_id = "turn-9".into();
    let session = ScriptedSession::always(
        Reply::ok(finished).event(tool("started")).event(tool("completed")),
    );
    let test = runtime(session).await;
    let service = test.runtime.service();
    retrieval(service).await;
    start(service, "retrieval").await;
    let supervisor = test.runtime.supervisor();
    supervisor.ensure_running("retrieval").await.unwrap();
    supervisor.join("retrieval").await;

    let collaboration = test.runtime.collaboration();
    let run = collaboration.runs("retrieval", None, None, 50).await.unwrap().items.remove(0);
    assert_eq!(run.status, "COMPLETED");
    assert_eq!(run.tool_calls, 1);
    assert_eq!(run.model.as_deref(), Some("test-model"));
    assert_eq!(run.usage.input_tokens, Some(120));
    assert_eq!(run.usage.cost_microusd, Some(500));
    let detail = collaboration.run("retrieval", &run.id).await.unwrap();
    let types: Vec<&str> = detail.events.iter().map(|event| event.event_type.as_str()).collect();
    assert_eq!(
        types,
        ["runtime.turn_started", "runtime.codex", "runtime.codex", "runtime.turn_completed"]
    );
    assert!(detail.events.iter().all(|event| event.payload["run_id"] == run.id.as_str()));
    test.close().await;
}

/// The lease is renewed every third of its TTL (at least every 10 s), so this test takes ~10 s.
#[tokio::test]
async fn a_lost_lease_interrupts_the_turn_and_refuses_its_checkpoint() {
    let session = BlockingSession::new(
        complete(),
        Release::Complete(turn(Some("thread-live"), "Late.", "COMPLETE", "Stale", "Stale", "None")),
    );
    let test = runtime_with(factory(session.clone()), |config| config.lease_ttl_seconds = 30).await;
    let service = test.runtime.service();
    let supervisor = test.runtime.supervisor();
    retrieval(service).await;
    start(service, "retrieval").await;
    supervisor.ensure_running("retrieval").await.unwrap();
    session.started(1).await;
    // Another worker takes the coordinator over before the next renewal.
    let challenge = service.get_challenge("retrieval").await.unwrap();
    let mut tx = test.runtime.db().begin_write().await.unwrap();
    tx.execute(
        "UPDATE work_leases SET owner = 'limina:thief' WHERE challenge_id = $1 \
         AND scope = 'coordinator'",
        args![&challenge.id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let collaboration = test.runtime.collaboration();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let runs = collaboration.runs("retrieval", None, None, 50).await.unwrap();
        if runs.items.first().is_some_and(|run| run.status == "FAILED") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the lost lease never failed the run");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    supervisor.shutdown().await;

    assert!(session.interrupts.load(Ordering::SeqCst) >= 1);
    let lost = events(service, "retrieval")
        .await
        .into_iter()
        .find(|event| event.event_type == "runtime.lease_lost")
        .unwrap();
    assert_eq!(lost.payload["error"], "coordinator is already claimed by limina:thief.");
    assert!(lost.payload["run_id"].is_string());
    let runs = collaboration.runs("retrieval", None, None, 50).await.unwrap().items;
    let error = runs.last().unwrap().error.clone().unwrap();
    assert_eq!(error.code.as_deref(), Some("lease_conflict"));
    assert_eq!(
        error.message.as_deref(),
        Some("The project runtime lost ownership while the managed turn was active.")
    );
    // The stale decision was never checkpointed, and no other turn could start.
    let state = coordinator(service, "retrieval").await;
    assert_eq!(state.status, "RUNNING");
    assert_eq!(state.current_objective, "Improve retrieval quality.");
    assert_eq!(runs.len(), 1);
    test.close().await;
}
