//! The live WebSocket protocol (`/v1/projects/{slug}/live`) against a real listener: the
//! handshake and its refusal codes, the snapshot and catch-up, immediate pushes through the event
//! notifier and the poll fallback for other writers, the message types and their errors, pings,
//! and a slow client being dropped instead of buffered for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use app_support::*;
use axum::http::Method;
use futures::StreamExt;
use http_support::*;
use limina_server::http::LiveSettings;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const OWNER: &[(&str, &str)] = &[("authorization", "Bearer owner-token")];

async fn live_api(settings: LiveSettings, session: Arc<dyn limina_engines::AgentSession>) -> Api {
    let test = runtime(session).await;
    let mut config = config(&test);
    config.live = settings;
    let api = Api::with_config(test, ui_tokens(), config).await;
    let body = json!({"slug": "live", "name": "Live", "objective": "Follow along.", "success_criteria": "Events arrive."});
    api.post("/v1/projects", &h(&command("owner-token", None, "x")), &body).await.expect(201);
    api
}

async fn default_api() -> Api {
    live_api(LiveSettings::default(), ScriptedSession::always(Reply::ok(complete()))).await
}

/// No poll fallback within the test's lifetime: every push must come from the notifier.
fn notifier_only() -> LiveSettings {
    LiveSettings { poll_interval: Duration::from_secs(600), ..LiveSettings::default() }
}

async fn owner_socket(server: &Listening, query: &str) -> Socket {
    let url = server.ws_url(&format!("/v1/projects/live/live{query}"));
    connect(&url, &["limina.v1"], OWNER).await.unwrap().0
}

#[tokio::test]
async fn the_handshake_refuses_with_protocol_codes() {
    let api = default_api().await;
    let server = api.listen().await;
    let url = server.ws_url("/v1/projects/live/live");

    let (mut socket, protocol) = connect(&url, &[], OWNER).await.unwrap();
    assert_eq!(protocol, None);
    assert_eq!(
        close_of(&mut socket).await,
        Some((4400, "The limina.v1 subprotocol is required.".into()))
    );

    let (mut socket, _) =
        connect(&url, &["limina.v1", "limina.ticket.a", "limina.ticket.b"], OWNER).await.unwrap();
    assert_eq!(
        close_of(&mut socket).await,
        Some((4400, "Only one live ticket may be supplied.".into()))
    );

    let (mut socket, _) = connect(&url, &["limina.v1"], &[]).await.unwrap();
    assert_eq!(close_of(&mut socket).await, Some((4401, "Authentication required.".into())));

    let (mut socket, _) =
        connect(&url, &["limina.v1"], &[("authorization", "Bearer viewer-token")]).await.unwrap();
    assert_eq!(close_of(&mut socket).await.map(|close| close.0), Some(4403));

    let (mut socket, _) = connect(&url, &["limina.v1", "limina.ticket.forged"], &[]).await.unwrap();
    assert_eq!(close_of(&mut socket).await.map(|close| close.0), Some(4403));

    let missing = server.ws_url("/v1/projects/missing/live");
    let (mut socket, _) = connect(&missing, &["limina.v1"], OWNER).await.unwrap();
    assert_eq!(close_of(&mut socket).await.map(|close| close.0), Some(4401));

    // Plain HTTP finds no route, as with the Python server.
    let plain = api.get("/v1/projects/live/live", OWNER).await;
    plain.expect(404);
    assert_eq!(plain.json(), json!({"detail": "Not Found"}));
    api.close().await;
}

#[tokio::test]
async fn failed_live_attempts_count_against_the_client_limit() {
    let api = default_api().await;
    let server = api.listen().await;
    let url = server.ws_url("/v1/projects/live/live");
    for _ in 0..10 {
        let (mut socket, _) =
            connect(&url, &["limina.v1"], &[("authorization", "Bearer wrong")]).await.unwrap();
        assert_eq!(close_of(&mut socket).await.map(|close| close.0), Some(4401));
    }
    let (mut socket, _) = connect(&url, &["limina.v1"], OWNER).await.unwrap();
    assert_eq!(
        close_of(&mut socket).await,
        Some((4401, "Too many failed authentication attempts.".into()))
    );
    api.close().await;
}

#[tokio::test]
async fn sessions_start_with_a_snapshot_and_catch_up_after_the_cursor() {
    let api = default_api().await;
    let server = api.listen().await;
    let mut socket = owner_socket(&server, "?after=0").await;
    let snapshot = next_json(&mut socket).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["value"]["project"]["slug"], "live");
    assert_eq!(snapshot["value"]["project"]["role"], "OWNER");
    let created = next_json(&mut socket).await;
    assert_eq!(created["type"], "event");
    assert_eq!(created["value"]["type"], "project.created");
    let cursor = created["value"]["sequence"].as_i64().unwrap();
    drop(socket);

    // Reconnecting after the last event replays nothing; new events still arrive.
    let mut socket = owner_socket(&server, &format!("?after={cursor}")).await;
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    api.put(
        "/v1/projects/live/resources/variables/MODE",
        &h(&command("owner-token", None, "x")),
        &json!({"value": "fast"}),
    )
    .await
    .expect(200);
    let next = next_json(&mut socket).await;
    assert_eq!(next["value"]["type"], "resource.variable_set");
    assert!(next["value"]["sequence"].as_i64().unwrap() > cursor);
    // An unparseable cursor starts from the beginning, as Python's `int()` fallback did.
    let mut socket = owner_socket(&server, "?after=soon").await;
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    assert_eq!(next_json(&mut socket).await["value"]["type"], "project.created");
    api.close().await;
}

#[tokio::test]
async fn writes_in_this_process_are_pushed_immediately() {
    let api = live_api(notifier_only(), ScriptedSession::always(Reply::ok(complete()))).await;
    let server = api.listen().await;
    let mut socket = owner_socket(&server, "?after=1").await;
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    let started = Instant::now();
    api.post(
        "/v1/projects/live/steering",
        &h(&command("owner-token", None, "x")),
        &json!({"body": "Go deeper.", "kind": "ANSWER"}),
    )
    .await
    .expect(202);
    let event = next_json(&mut socket).await;
    assert_eq!(event["value"]["type"], "guidance.received");
    assert_eq!(event["value"]["detail"]["body"], "Go deeper.");
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());

    // Internal agent writes wake live clients too.
    let capability = api.test.runtime.supervisor().issue_capability("live").unwrap();
    let authorization = bearer(&capability);
    let key = limina_core::new_uuid();
    api.post(
        "/internal/v1/projects/live/hypotheses",
        &[("authorization", authorization.as_str()), ("idempotency-key", key.as_str())],
        &json!({"title": "Pushed", "statement": "Agents' writes reach the console at once."}),
    )
    .await
    .expect(201);
    let event = next_json(&mut socket).await;
    assert_eq!(event["value"]["type"], "hypothesis.created");
    api.close().await;
}

#[tokio::test]
async fn writes_from_other_processes_arrive_through_the_poll() {
    let settings =
        LiveSettings { poll_interval: Duration::from_millis(50), ..LiveSettings::default() };
    let api = live_api(settings, ScriptedSession::always(Reply::ok(complete()))).await;
    let server = api.listen().await;
    let mut socket = owner_socket(&server, "?after=1").await;
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    // Written straight to the database: no notification.
    insert_events(&api.test, "live", &[("test.marker", "tester", json!({"marker": 1}))]).await;
    let event = next_json(&mut socket).await;
    assert_eq!(event["value"]["detail"]["marker"], 1);
    // Hidden coordination events advance the cursor without being sent.
    insert_events(
        &api.test,
        "live",
        &[
            ("coordinator.claimed", "limina:worker", json!({})),
            ("test.marker", "tester", json!({"marker": 2})),
        ],
    )
    .await;
    assert_eq!(next_json(&mut socket).await["value"]["detail"]["marker"], 2);
    api.close().await;
}

#[tokio::test]
async fn steering_interrupts_and_actions_answer_with_their_frames() {
    let session = BlockingSession::steerable();
    let api = live_api(LiveSettings::default(), session.clone()).await;
    let server = api.listen().await;
    let mut socket = owner_socket(&server, "").await;
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");

    // Guidance without a running turn is queued.
    send_json(&mut socket, &json!({"body": "Check the baseline first.", "kind": "ANSWER"})).await;
    let queued = next_non_event(&mut socket).await;
    assert_eq!(queued["type"], "delivery");
    assert_eq!(queued["value"], "QUEUED");
    assert_eq!(queued["receipt"]["kind"], "ANSWER");

    send_json(&mut socket, &json!({"type": "action", "action": "START"})).await;
    let state = next_non_event(&mut socket).await;
    assert_eq!(state["type"], "state");
    assert_eq!(state["value"]["slug"], "live");
    session.started(1).await;

    send_json(&mut socket, &json!({"type": "interrupt"})).await;
    let interrupted = next_non_event(&mut socket).await;
    assert_eq!(interrupted["type"], "delivery");
    assert_eq!(interrupted["receipt"]["kind"], "INTERRUPT");
    wait_until("the interrupt to pause the project", || async {
        api.get("/v1/projects/live", OWNER).await.json()["status"] == "PAUSED"
    })
    .await;
    api.close().await;
}

#[tokio::test]
async fn invalid_messages_end_the_session_with_an_error() {
    let api = default_api().await;
    let server = api.listen().await;
    for (message, code, text) in [
        (
            json!({"type": "dance"}),
            "invariant_violation",
            "Live messages must be steer, interrupt, or action.",
        ),
        (
            json!({"type": "steer", "body": "   "}),
            "invariant_violation",
            "Steering message cannot be empty.",
        ),
        (
            json!({"type": "action", "action": "explode"}),
            "invariant_violation",
            "Project action must be start, pause, resume, stop, or archive.",
        ),
        (json!(["not", "an", "object"]), "invalid_request", "Live messages must be JSON objects."),
    ] {
        let mut socket = owner_socket(&server, "").await;
        assert_eq!(next_json(&mut socket).await["type"], "snapshot");
        send_json(&mut socket, &message).await;
        let error = next_non_event(&mut socket).await;
        assert_eq!(
            error,
            json!({"type": "error", "value": {"code": code, "message": text}}),
            "{message}"
        );
        assert_eq!(close_of(&mut socket).await.map(|close| close.0), Some(1000));
    }
    api.close().await;
}

/// Reads frames for `duration`; returns the close frame if the server closed meanwhile.
async fn read_for(socket: &mut Socket, duration: Duration) -> Option<Option<(u16, String)>> {
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        match tokio::time::timeout_at(deadline, socket.next()).await {
            Err(_) => return None,
            Ok(None) | Ok(Some(Err(_))) => return Some(None),
            Ok(Some(Ok(Message::Close(frame)))) => {
                return Some(frame.map(|frame| (u16::from(frame.code), frame.reason.to_string())));
            }
            Ok(Some(Ok(_))) => {}
        }
    }
}

#[tokio::test]
async fn unanswered_pings_close_the_session() {
    let settings = LiveSettings {
        ping_interval: Duration::from_millis(50),
        pong_timeout: Duration::from_millis(100),
        ..LiveSettings::default()
    };
    let api = live_api(settings, ScriptedSession::always(Reply::ok(complete()))).await;
    let server = api.listen().await;
    // A client that keeps reading answers the pings and stays connected.
    let mut responsive = owner_socket(&server, "").await;
    assert_eq!(read_for(&mut responsive, Duration::from_millis(500)).await, None);
    // One that stops reading never answers and is closed.
    let mut silent = owner_socket(&server, "").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(close_of(&mut silent).await, Some((1011, "keepalive ping timeout".into())));
    api.close().await;
}

#[tokio::test]
async fn slow_clients_are_dropped_instead_of_buffered() {
    let settings =
        LiveSettings { send_timeout: Duration::from_millis(200), ..LiveSettings::default() };
    let api = live_api(settings, ScriptedSession::always(Reply::ok(complete()))).await;
    let filler = "x".repeat(64 * 1024);
    let rows: Vec<(&str, &str, Value)> = (0..400)
        .map(|marker| ("test.marker", "tester", json!({"marker": marker, "filler": filler})))
        .collect();
    insert_events(&api.test, "live", &rows).await;
    let server = api.listen().await;
    let mut socket = owner_socket(&server, "").await;
    // Do not read while the server has far more than the socket buffers hold to send.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mut received = 0;
    loop {
        match tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("the stream hangs")
        {
            Some(Ok(Message::Text(_))) => received += 1,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
            Some(Ok(_)) => {}
        }
    }
    // The snapshot plus every event would be 402 frames; the server gave up long before.
    assert!(received < 402, "{received} frames were delivered");
    api.close().await;
}

#[tokio::test]
async fn binary_frames_and_pings_from_clients_are_accepted() {
    let api = default_api().await;
    let server = api.listen().await;
    let mut socket = owner_socket(&server, "").await;
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    futures::SinkExt::send(&mut socket, Message::Ping(vec![1, 2].into())).await.unwrap();
    let steer = json!({"type": "steer", "body": "From a binary frame."}).to_string();
    futures::SinkExt::send(&mut socket, Message::Binary(steer.into_bytes().into())).await.unwrap();
    assert_eq!(next_non_event(&mut socket).await["type"], "delivery");
    let guidance = api.get("/v1/projects/live/guidance", OWNER).await.json();
    assert_eq!(guidance["items"][0]["body"], "From a binary frame.");
    let _ = api.call(Method::GET, "/livez", &[]).await;
    api.close().await;
}
