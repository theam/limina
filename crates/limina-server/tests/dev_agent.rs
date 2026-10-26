//! The scripted engine, driven turn by turn against the internal agent API of a real runtime.
//!
//! `fixtures/dev_scenario.json` is what the original Python scripted engine streamed and
//! requested for every step (a frozen recording); the Rust engine must produce the same events,
//! requests, checkpoints and usage.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod dev_support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dev_support::{DevRuntime, QUIET, drain, numbered, turn_request};
use limina_core::ErrorKind;
use limina_engines::{AgentSession, ChannelObserver, RuntimeEvent, TurnSignal};
use limina_server::dev::{DEMO_SLUG, INTERRUPTED, SCRIPTED_MODEL, STEERING_ACK, ScriptedAgent};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/dev_scenario.json")).unwrap()
}

fn fixture_pace(fixture: &Value) -> Duration {
    Duration::from_secs_f64(fixture["pace"].as_f64().unwrap())
}

/// Key order matters (stored payloads keep it), so values are compared as serialized JSON.
fn same(actual: &Value, expected: &Value) -> bool {
    serde_json::to_string(actual).unwrap() == serde_json::to_string(expected).unwrap()
}

#[track_caller]
fn assert_same(actual: &Value, expected: &Value, what: &str) {
    assert!(
        same(actual, expected),
        "{what} differ\n  actual: {}\nexpected: {}",
        serde_json::to_string_pretty(actual).unwrap(),
        serde_json::to_string_pretty(expected).unwrap()
    );
}

/// Waits for the next event of a running turn.
async fn next_event(signals: &mut mpsc::UnboundedReceiver<TurnSignal>) -> RuntimeEvent {
    match tokio::time::timeout(Duration::from_secs(10), signals.recv()).await {
        Ok(Some(TurnSignal::Event(event))) => event,
        other => panic!("expected an event, got {other:?}"),
    }
}

#[tokio::test]
async fn every_step_streams_and_writes_what_the_python_engine_did() {
    let fixture = fixture();
    let pace = fixture_pace(&fixture);
    let dev = DevRuntime::start(|options| options.pace = pace).await;
    dev.create_project(DEMO_SLUG).await;
    let agent = ScriptedAgent::new(DEMO_SLUG, pace);
    assert_eq!(agent.model().as_deref(), Some(SCRIPTED_MODEL));
    for (step, expected) in fixture["turns"].as_array().unwrap().iter().enumerate() {
        let prompt = expected["prompt"].as_str().unwrap();
        let (turn, events) = dev.play(&agent, DEMO_SLUG, prompt).await;
        let turn = turn.unwrap_or_else(|error| panic!("step {step} failed: {error:?}"));
        assert_same(&numbered(&events), &expected["events"], &format!("step {step} events"));
        let requests = serde_json::to_value(dev.take_requests()).unwrap();
        assert_same(&requests, &expected["requests"], &format!("step {step} requests"));
        let mut checkpoint = json!(turn.decision);
        if step == 2 {
            assert_eq!(checkpoint["mission_update"]["evidence"], json!(["F001", "E002"]));
            assert!(
                checkpoint["mission_update"]["decision"]["recommendation"]
                    .as_str()
                    .unwrap()
                    .contains("0.6")
            );
        }
        if step >= 3 {
            assert_eq!(checkpoint["mission_update"]["evidence"], json!(["F001", "E002", "F002"]));
            assert!(checkpoint["mission_update"]["decision"].is_null());
        }
        checkpoint.as_object_mut().unwrap().remove("mission_update");
        assert_same(&checkpoint, &expected["decision"], &format!("step {step} decision"));
        let usage = turn.usage.unwrap();
        let tokens = expected["usage"]["output_tokens"].as_i64().unwrap();
        assert_eq!(usage.output_tokens, Some(tokens), "step {step}");
        assert_eq!(usage.input_tokens, Some(tokens * 9));
        assert_eq!(usage.cached_input_tokens, Some(tokens * 4));
        assert_eq!(usage.total_tokens, Some(tokens * 10));
        assert_eq!(usage.reasoning_output_tokens, None);
        assert_eq!(usage.cost_microusd, None);
        assert_eq!(usage.usage_source.as_deref(), Some("provider"));
        assert_eq!(turn.continuation_id, None);
        let id = turn.turn_id.strip_prefix("turn-").unwrap();
        assert!(id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()), "{}", turn.turn_id);
    }
    dev.close().await;
}

#[tokio::test]
async fn the_research_graph_grows_one_step_per_turn() {
    let dev = DevRuntime::start(|_| {}).await;
    dev.create_project(DEMO_SLUG).await;
    let agent = ScriptedAgent::new(DEMO_SLUG, Duration::from_millis(1));
    let service = dev.runtime.service();

    // 1. The baseline, then H001 and H002.
    let (turn, _) = dev.play(&agent, DEMO_SLUG, QUIET).await;
    assert_eq!(turn.unwrap().decision.status, "RUNNING");
    let statuses = dev.statuses(DEMO_SLUG).await;
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses["H001"], "PROPOSED");
    assert_eq!(statuses["H002"], "PROPOSED");

    // 2. E001 is designed, claimed, observed and completed; F001 confirms H001.
    let (turn, _) = dev.play(&agent, DEMO_SLUG, QUIET).await;
    assert_eq!(turn.unwrap().decision.summary, "Confirmed H001 with E001 and published F001.");
    let statuses = dev.statuses(DEMO_SLUG).await;
    assert_eq!(statuses["H001"], "CONFIRMED");
    assert_eq!(statuses["E001"], "COMPLETED");
    assert_eq!(statuses["F001"], "PUBLISHED");
    let experiment = service.get_artifact(DEMO_SLUG, "E001").await.unwrap();
    assert_eq!(experiment.artifact.hypothesis_id.as_deref(), Some("H001"));
    assert_eq!(experiment.observations.unwrap().len(), 2);
    assert!(experiment.lease.is_none(), "completing E001 releases its lease");
    let finding = service.get_artifact(DEMO_SLUG, "F001").await.unwrap().artifact;
    assert_eq!(finding.experiment_id.as_deref(), Some("E001"));
    assert_eq!(finding.payload["impact"], "HIGH");
    let events = service.events(DEMO_SLUG, 0, 200).await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|event| event.event_type.as_str()).collect();
    for kind in ["experiment.created", "experiment.claimed", "experiment.observed"] {
        assert!(kinds.contains(&kind), "{kind} missing from {kinds:?}");
    }

    // 3. The threshold needs a human decision: E002 is designed and the turn asks.
    let (turn, _) = dev.play(&agent, DEMO_SLUG, QUIET).await;
    let decision = turn.unwrap().decision;
    assert_eq!(decision.status, "WAITING");
    assert!(decision.blocker.starts_with("How much answer rate can we give up?"));
    assert_eq!(dev.statuses(DEMO_SLUG).await["E002"], "DESIGNED");

    // 4. The answer runs E002; F002 confirms H002.
    let (turn, _) = dev.play(&agent, DEMO_SLUG, QUIET).await;
    let decision = turn.unwrap().decision;
    assert_eq!((decision.status.as_str(), decision.blocker.as_str()), ("WAITING", "None"));
    let statuses = dev.statuses(DEMO_SLUG).await;
    assert_eq!(statuses["H002"], "CONFIRMED");
    assert_eq!(statuses["E002"], "COMPLETED");
    assert_eq!(statuses["F002"], "PUBLISHED");

    // 5. Afterwards it only reports, and picks up guidance when the prompt has some.
    let guided = "# Turn\n\n## New human guidance\n\n- [STEER] Olivia: Next?\n";
    let (turn, events) = dev.play(&agent, DEMO_SLUG, guided).await;
    assert_eq!(turn.unwrap().decision.next_step, "Wait for the next direction");
    assert_eq!(events[0].detail_str("text"), Some("Picking up your latest guidance."));
    assert_eq!(dev.statuses(DEMO_SLUG).await.len(), 6);
    dev.close().await;
}

#[tokio::test]
async fn steering_is_acknowledged_at_the_next_step() {
    let fixture = fixture();
    let dev = DevRuntime::start(|_| {}).await;
    dev.create_project(DEMO_SLUG).await;
    // Steps take tens of milliseconds, so both notes land in the same pause.
    let agent = Arc::new(ScriptedAgent::new(DEMO_SLUG, Duration::from_millis(100)));
    assert!(!agent.steer("Nobody is listening").await.unwrap(), "no turn is active");
    let (observer, mut signals) = ChannelObserver::new();
    let request = turn_request(QUIET, dev.turn_env(DEMO_SLUG), observer, &dev.data());
    let running = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run_turn(request).await }
    });
    let first = next_event(&mut signals).await;
    assert_eq!(first.detail_str("kind"), Some("message"));
    let started = next_event(&mut signals).await;
    assert_eq!(started.detail_str("phase"), Some("started"));
    assert!(agent.steer("Olivia Owner: Prefer recall: keep latency flat").await.unwrap());
    assert!(agent.steer("Sam without a colon").await.unwrap());
    // The acknowledgement comes when the command's pause ends, before it completes.
    let acknowledged = next_event(&mut signals).await;
    assert_same(&numbered(std::slice::from_ref(&acknowledged)), &fixture["steering"], "steering");
    assert!(acknowledged.detail_str("text").unwrap().starts_with(STEERING_ACK));
    let completed = next_event(&mut signals).await;
    assert_eq!(completed.detail_str("phase"), Some("completed"));
    assert_eq!(completed.detail["item_id"], started.detail["item_id"]);
    let turn = running.await.unwrap().unwrap();
    assert_eq!(turn.decision.status, "RUNNING");
    let acks = drain(&mut signals)
        .iter()
        .filter(|event| event.detail_str("text").is_some_and(|text| text.starts_with(STEERING_ACK)))
        .count();
    assert_eq!(acks, 0, "each note is acknowledged once");
    // The acknowledgement counts toward the turn's tokens like any message.
    assert!(turn.usage.unwrap().output_tokens.unwrap() > 2018);
    assert!(!agent.steer("Too late").await.unwrap());
    dev.close().await;
}

#[tokio::test]
async fn an_interrupt_ends_the_turn_at_once_with_a_transport_error() {
    let fixture = fixture();
    let dev = DevRuntime::start(|_| {}).await;
    dev.create_project(DEMO_SLUG).await;
    let agent = Arc::new(ScriptedAgent::new(DEMO_SLUG, Duration::from_millis(200)));
    assert!(!agent.interrupt().await.unwrap(), "nothing to interrupt");
    let (observer, mut signals) = ChannelObserver::new();
    let request = turn_request(QUIET, dev.turn_env(DEMO_SLUG), observer, &dev.data());
    let running = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run_turn(request).await }
    });
    next_event(&mut signals).await;
    let interrupted_at = Instant::now();
    assert!(agent.interrupt().await.unwrap());
    let error = running.await.unwrap().unwrap_err();
    assert!(interrupted_at.elapsed() < Duration::from_millis(150), "the pause ends at once");
    assert_eq!(error.kind, ErrorKind::Transport);
    assert_eq!(error.code(), fixture["interrupted"]["code"]);
    assert_eq!(error.message, fixture["interrupted"]["message"]);
    assert_eq!(error.message, INTERRUPTED);
    assert!(!agent.interrupt().await.unwrap());
    assert!(dev.statuses(DEMO_SLUG).await.is_empty(), "nothing was written");

    // The next turn starts over; closing the session interrupts like an interrupt does.
    let quick = ScriptedAgent::new(DEMO_SLUG, Duration::from_millis(1));
    quick.close().await;
    let (turn, _) = dev.play(&quick, DEMO_SLUG, QUIET).await;
    assert_eq!(turn.unwrap().decision.status, "RUNNING");
    assert_eq!(dev.statuses(DEMO_SLUG).await.len(), 2);
    dev.close().await;
}

#[tokio::test]
async fn the_first_request_waits_for_the_internal_api_to_listen() {
    let dev = DevRuntime::start(|_| {}).await;
    dev.create_project(DEMO_SLUG).await;
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut env = dev.turn_env(DEMO_SLUG);
    env.insert("LIMINA_INTERNAL_URL".into(), format!("http://127.0.0.1:{port}/"));
    let agent = ScriptedAgent::new(DEMO_SLUG, Duration::from_millis(1));
    let (observer, _signals) = ChannelObserver::new();
    let request = turn_request(QUIET, env, observer, &dev.data());
    let late = async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        dev.serve(TcpListener::bind(("127.0.0.1", port)).await.unwrap());
    };
    let (turn, ()) = tokio::join!(agent.run_turn(request), late);
    assert_eq!(turn.unwrap().decision.status, "RUNNING");
    assert_eq!(dev.statuses(DEMO_SLUG).await.len(), 2);
    dev.close().await;
}

#[tokio::test]
async fn internal_api_failures_fail_the_turn_with_the_reason() {
    let dev = DevRuntime::start(|_| {}).await;
    dev.create_project(DEMO_SLUG).await;
    let agent = ScriptedAgent::new(DEMO_SLUG, Duration::from_millis(1));

    let mut env = dev.turn_env(DEMO_SLUG);
    env.insert("LIMINA_INTERNAL_TOKEN".into(), "not-a-capability".into());
    let (observer, _signals) = ChannelObserver::new();
    let error = agent.run_turn(turn_request(QUIET, env, observer, &dev.data())).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Transport);
    assert_eq!(error.message, "The internal agent API rejected GET artifacts with HTTP 401.");
    assert_eq!(error.details["status"], 401);
    assert!(error.details["reason"].as_str().is_some_and(|reason| !reason.is_empty()));

    let (observer, _signals) = ChannelObserver::new();
    let error = agent.run_turn(turn_request(QUIET, BTreeMap::new(), observer, &dev.data())).await;
    let error = error.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Transport);
    assert_eq!(error.details["missing"], "LIMINA_INTERNAL_URL");

    // A project the capability does not cover is refused too.
    let other = ScriptedAgent::new("another-project", Duration::from_millis(1));
    let (observer, _signals) = ChannelObserver::new();
    let request = turn_request(QUIET, dev.turn_env(DEMO_SLUG), observer, &dev.data());
    let error = other.run_turn(request).await.unwrap_err();
    assert_eq!(error.details["status"], 401);
    dev.close().await;
}
