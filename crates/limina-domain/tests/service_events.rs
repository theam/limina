//! The event log, guidance inbox and checkpoints: service-level ports of the activity tests in
//! `tests/test_cloud_ui_readiness.py` and `tests/test_cloud_api.py`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod service_support;

use limina_domain::common::record_event;
use limina_domain::service::{ChallengeService, Checkpoint};
use limina_store::{Db, args};
use serde_json::{Map, json};
use service_support::{code, command_id, each_backend, retrieval};

const HIDDEN: &[&str] = &["coordinator.claimed", "coordinator.released"];

async fn challenge_id(service: &ChallengeService) -> String {
    service.get_challenge("retrieval").await.unwrap().id
}

/// Appends `count` marker events, each followed by a hidden coordinator event when `hidden`.
async fn markers(db: &Db, challenge_id: &str, count: usize, hidden: bool) {
    let mut tx = db.begin_write().await.unwrap();
    for marker in 0..count {
        record_event(
            &mut tx,
            challenge_id,
            "test.marker",
            "tester",
            &format!("marker-{marker}"),
            json!({"marker": marker}),
            None,
        )
        .await
        .unwrap();
        if hidden {
            record_event(
                &mut tx,
                challenge_id,
                "coordinator.claimed",
                "limina:worker",
                &format!("claim-{marker}"),
                json!({}),
                None,
            )
            .await
            .unwrap();
        }
    }
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn events_page_backward_with_an_earlier_history_flag() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        markers(&backend.db, &challenge_id(&service).await, 10, true).await;
        let newest = service.status("retrieval").await.unwrap().last_event_sequence;
        let marker = |events: &[limina_domain::service::EventView]| -> Vec<i64> {
            events.iter().map(|event| event.payload["marker"].as_i64().unwrap()).collect()
        };

        let (latest, earlier) =
            service.events_before("retrieval", newest + 1, 0, 3, HIDDEN).await.unwrap();
        assert_eq!(marker(&latest), [7, 8, 9]);
        assert!(earlier);
        let before = i64::from(latest[0].sequence);
        let (older, earlier) =
            service.events_before("retrieval", before, 0, 3, HIDDEN).await.unwrap();
        assert_eq!(marker(&older), [4, 5, 6]);
        assert!(earlier);
        let before = i64::from(older[0].sequence);
        let (rest, earlier) =
            service.events_before("retrieval", before, 0, 100, HIDDEN).await.unwrap();
        assert_eq!(rest[0].event_type, "challenge.created");
        assert_eq!(rest.len(), 5);
        assert!(!earlier);

        let forward = service.events("retrieval", 0, 2).await.unwrap();
        let first = i64::from(forward[0].sequence);
        assert!(!service.has_events_before("retrieval", first, HIDDEN).await.unwrap());
        let cursor = i64::from(forward[1].sequence);
        let resumed = service.events("retrieval", cursor, 200).await.unwrap();
        assert_eq!(i64::from(resumed.last().unwrap().sequence), newest);
        let boundary = i64::from(resumed[0].sequence);
        assert!(service.has_events_before("retrieval", boundary, HIDDEN).await.unwrap());
        let (empty, earlier) = service.events_before("retrieval", 1, 0, 10, HIDDEN).await.unwrap();
        assert!(empty.is_empty() && !earlier);
    })
    .await;
}

#[tokio::test]
async fn recent_events_use_the_newest_events_after_large_histories() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        markers(&backend.db, &challenge_id(&service).await, 1005, false).await;
        let recent = service.recent_events("retrieval", 200).await.unwrap();
        assert_eq!(recent.len(), 200);
        assert_eq!(recent.last().unwrap().payload["marker"], 1004);
        assert!(recent.windows(2).all(|pair| pair[0].sequence < pair[1].sequence));
        assert_eq!(service.recent_events("retrieval", 5000).await.unwrap().len(), 1000);
        assert_eq!(service.events("retrieval", 0, 5000).await.unwrap().len(), 1000);
        assert_eq!(service.recent_events("retrieval", -3).await.unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn activity_cursor_replays_only_new_durable_events() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let first = service.events("retrieval", 0, 200).await.unwrap();
        assert_eq!(
            first.iter().map(|e| e.event_type.as_str()).collect::<Vec<_>>(),
            ["challenge.created"]
        );
        assert_eq!(
            first[0].payload,
            json!({"slug": "retrieval", "name": "Retrieval quality", "runtime_engine": "codex"})
        );
        let cursor = i64::from(first[0].sequence);
        service
            .set_variable("retrieval", "BRIEF_URI", "s3://brief", "owner", &command_id())
            .await
            .unwrap();
        let second = service.events("retrieval", cursor, 200).await.unwrap();
        assert_eq!(
            second.iter().map(|e| e.event_type.as_str()).collect::<Vec<_>>(),
            ["resource.variable_set"]
        );
        assert!(second[0].sequence > first[0].sequence);
        assert_eq!(second[0].payload["name"], "BRIEF_URI");
        assert_eq!(second[0].payload["type"], "VARIABLE");
    })
    .await;
}

#[tokio::test]
async fn guidance_events_carry_their_body_and_legacy_bodies_resolve() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let steer = service
            .send_message(
                "retrieval",
                "STEER",
                "Prefer the stronger baseline.",
                None,
                "owner",
                &command_id(),
            )
            .await
            .unwrap();
        let acknowledged = service
            .acknowledge_message("retrieval", &steer.id, "limina:test", &command_id())
            .await
            .unwrap();
        assert_eq!(acknowledged.status, "ACKNOWLEDGED");
        assert!(acknowledged.acknowledged_at.is_some());
        let legacy = service
            .send_message(
                "retrieval",
                "answer",
                "Use the March snapshot.",
                None,
                "owner",
                &command_id(),
            )
            .await
            .unwrap();
        assert_eq!(legacy.kind, "ANSWER");
        // Events recorded before bodies were part of the payload.
        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute(
            "UPDATE events SET payload = $1 WHERE event_type = 'inbox.message_sent' \
             AND command_id = (SELECT command_id FROM inbox_messages WHERE id = $2)",
            args![
                json!({"message_id": legacy.id, "kind": "ANSWER", "sequence": legacy.sequence}),
                &legacy.id
            ],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let events = service.events("retrieval", 0, 200).await.unwrap();
        let sent: Vec<_> = events.iter().filter(|e| e.event_type == "inbox.message_sent").collect();
        assert_eq!(sent[0].payload["body"], "Prefer the stronger baseline.");
        assert!(sent[1].payload.get("body").is_none());
        let acked: Vec<_> =
            events.iter().filter(|e| e.event_type == "inbox.message_acknowledged").collect();
        assert_eq!(acked[0].payload, json!({"message_id": steer.id, "sequence": steer.sequence}));

        let bodies = service
            .message_bodies("retrieval", [legacy.id.as_str(), "unknown", legacy.id.as_str()])
            .await
            .unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[&legacy.id], "Use the March snapshot.");
        assert!(service.message_bodies("missing", Vec::<String>::new()).await.unwrap().is_empty());
        assert_eq!(code(service.message_bodies("missing", ["x"]).await), "not_found");

        let pending = service.inbox("retrieval", 0, true).await.unwrap();
        assert_eq!(pending.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), [legacy.id.as_str()]);
        assert_eq!(
            service.inbox("retrieval", i64::from(steer.sequence), false).await.unwrap().len(),
            1
        );
        assert_eq!(
            code(service.acknowledge_message("retrieval", "missing", "x", &command_id()).await),
            "not_found"
        );
    })
    .await;
}

#[tokio::test]
async fn checkpoints_record_the_decision_and_consume_guidance() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let message = service
            .send_message("retrieval", "STEER", "Hurry.", Some("resource"), "owner", &command_id())
            .await
            .unwrap();
        let sent = service.events("retrieval", 1, 200).await.unwrap();
        assert_eq!(sent[0].payload["origin"], "resource");
        let wake_at = limina_core::now() + chrono::Duration::minutes(5);
        let coordinator = service
            .checkpoint_coordinator(
                "retrieval",
                Checkpoint {
                    current_objective: "Compare the rerankers.".into(),
                    next_step: "Run the frozen benchmark.".into(),
                    blocker: "Awaiting GPU quota.".into(),
                    status: "waiting".into(),
                    worker_id: Some("limina:worker".into()),
                    continuation_id: Some("private-continuation".into()),
                    inbox_cursor: message.sequence,
                    expected_version: 1,
                    acknowledge_message_ids: vec![message.id.clone()],
                    wake_at: Some(wake_at),
                },
                "limina:worker",
                &command_id(),
            )
            .await
            .unwrap();
        assert_eq!(coordinator.status, "WAITING");
        assert_eq!(coordinator.version, 2);
        assert_eq!(coordinator.wake_at, Some(wake_at));
        assert_eq!(coordinator.heartbeat_at, Some(coordinator.updated_at));
        assert!(service.inbox("retrieval", 0, true).await.unwrap().is_empty());
        let checkpoint = service.recent_events("retrieval", 1).await.unwrap().remove(0);
        assert_eq!(checkpoint.event_type, "coordinator.checkpointed");
        assert_eq!(checkpoint.actor, "limina:worker");
        assert_eq!(
            checkpoint.payload,
            json!({
                "status": "WAITING",
                "current_objective": "Compare the rerankers.",
                "next_step": "Run the frozen benchmark.",
                "blocker": "Awaiting GPU quota.",
                "version": 2,
                "worker_id": "limina:worker",
                "continuation_id": "private-continuation",
                "inbox_cursor": message.sequence,
                "messages_acknowledged": 1,
                "wake_at": limina_core::iso(&wake_at),
            })
        );
        let stored = service.get_challenge("retrieval").await.unwrap().coordinator.unwrap();
        assert_eq!(stored, coordinator, "the result is what was stored");
    })
    .await;
}

#[tokio::test]
async fn runtime_events_are_internal_and_typed_by_their_command() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let rejected = service
            .record_runtime_event(
                "retrieval",
                "inbox.message_sent",
                Map::new(),
                "limina:w",
                &command_id(),
            )
            .await
            .unwrap_err();
        assert_eq!(rejected.code(), "invariant_violation");
        assert_eq!(rejected.details["event_type"], "inbox.message_sent");
        let id = command_id();
        let payload = json!({"kind": "tool", "summary": "Using Bash"});
        let event = service
            .record_runtime_event(
                "retrieval",
                "runtime.claude-code",
                payload.as_object().cloned().unwrap(),
                "limina:worker",
                &id,
            )
            .await
            .unwrap();
        assert_eq!(event.event_type, "runtime.claude-code");
        assert_eq!(event.payload, payload);
        assert_eq!(event.command_id, id);
        let other_type = service
            .record_runtime_event("retrieval", "runtime.codex", Map::new(), "limina:worker", &id)
            .await
            .unwrap_err();
        assert_eq!(other_type.details["original_command_type"], "runtime.claude-code");
        assert_eq!(
            code(
                service
                    .record_runtime_event(
                        "missing",
                        "runtime.codex",
                        Map::new(),
                        "w",
                        &command_id()
                    )
                    .await
            ),
            "not_found"
        );
    })
    .await;
}
