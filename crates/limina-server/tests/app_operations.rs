//! The operations facade: the domain-rule assertions of `tests/test_cloud_ui_readiness.py` and
//! `tests/test_cloud_api.py` (roles, idempotency scoping, steering delivery, lifecycle,
//! resources, preflight, activity and guidance), called the way the HTTP and MCP layers call
//! them. Pure HTTP concerns (status codes of request validation, headers, OpenAPI, the WebSocket
//! protocol) stay with the HTTP port:
//!
//! - `test_cloud_api.py`: `test_authentication_and_public_surface` (probes, `/healthz`,
//!   OpenAPI), `test_failed_authentication_is_rate_limited`, the WebSocket half of
//!   `test_live_attach_can_steer_the_active_managed_turn`, and the routes and statuses of
//!   `test_runtime_authentication_requires_the_distinct_admin_token` and
//!   `test_private_agent_commands_require_a_project_scoped_capability`.
//! - `test_cloud_ui_readiness.py`: the WebSocket half of
//!   `test_browser_ticket_is_single_use_and_supports_read_only_viewers`,
//!   `test_me_describes_the_authenticated_principal`, the OpenAPI and multipart parts of
//!   `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`, `before=0` validation in
//!   `test_events_page_backward_with_an_earlier_history_flag`, and the 401 without credentials in
//!   `test_engine_availability_reports_readiness_without_secret_values`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;

use std::sync::Arc;

use app_support::*;
use limina_core::Principal;
use limina_domain::collaboration::{DraftChanges, MemberChange};
use limina_domain::common::record_event;
use limina_domain::knowledge::{KnowledgeQuery, NewRelation, SourceInput};
use limina_domain::research::NewHypothesis;
use limina_domain::service::Checkpoint;
use limina_engines::RuntimeEvent;
use limina_server::app::{Delivery, NewProject, Upload, scoped_command_id};
use limina_store::args;
use serde_json::{Map, Value, json};

fn completing_agent() -> Arc<ScriptedSession> {
    let mut finished = turn(
        Some("private-continuation"),
        "Evidence checkpoint completed.",
        "COMPLETE",
        "Synthesize evidence.",
        "Present the decision.",
        "None",
    );
    finished.usage = Some(usage(120, 30, 10, 500));
    ScriptedSession::always(Reply::ok(finished).continuation("private-continuation").event(
        RuntimeEvent::new(
            "runtime.codex",
            "limina _agent finding create --thread private-thread",
            detail(json!({
                "method": "item/started",
                "item_type": "CommandExecution",
                "kind": "tool",
                "tool": "command",
                "input": "limina _agent finding create --thread private-thread",
                "item_id": "command-1",
                "phase": "started",
                "state": "running",
                "thread_id": "private-thread",
                "continuation_id": "private-continuation",
            })),
        ),
    ))
}

fn hypothesis(title: &str, statement: &str) -> NewHypothesis {
    NewHypothesis {
        title: title.into(),
        statement: statement.into(),
        mechanism: "Lexical matching".into(),
        generalization: "Across artifacts".into(),
        shortcut_risks: "Vocabulary mismatch".into(),
        test_plan: "Query known terms".into(),
    }
}

async fn activity(test: &TestRuntime, slug: &str) -> Vec<limina_server::app::EventResponse> {
    test.ops().activity(slug, 0, None, 1000, &owner()).await.unwrap().events
}

/// Inserts events directly, like the Python tests' `session.add(Event(...))`.
async fn insert_events(test: &TestRuntime, slug: &str, events: &[(&str, &str, Value)]) {
    let challenge = test.runtime.service().get_challenge(slug).await.unwrap();
    let mut tx = test.runtime.db().begin_write().await.unwrap();
    for (event_type, actor, payload) in events {
        record_event(&mut tx, &challenge.id, event_type, actor, &key(), payload.clone(), None)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

// ─── test_cloud_ui_readiness.py ───────────────────────────────────────────────

#[tokio::test]
async fn oidc_principals_and_project_roles_are_enforced_centrally() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    create_project(ops, "ui-ready").await;

    assert_eq!(ops.list_projects(false, None, 100, &owner()).await.unwrap().total, 1);
    assert_eq!(ops.list_projects(false, None, 100, &viewer()).await.unwrap().total, 0);
    let hidden = ops.get_project("ui-ready", &viewer()).await.unwrap_err();
    assert_eq!((hidden.http_status(), hidden.code()), (403, "permission_denied"));

    let change = MemberChange {
        subject: "viewer".into(),
        display_name: "Victor Viewer".into(),
        email: Some("viewer@example.test".into()),
        role: "VIEWER".into(),
    };
    let member = ops.set_member("ui-ready", &change, &owner()).await.unwrap();
    assert_eq!(member.role.as_str(), "VIEWER");
    ops.get_project("ui-ready", &viewer()).await.unwrap();
    let denied = ops.apply_lifecycle("ui-ready", "start", &key(), &viewer()).await.unwrap_err();
    assert_eq!((denied.http_status(), denied.code()), (403, "permission_denied"));
    assert_eq!(denied.details["required_role"], "EDITOR");
    let steer = ops.steer("ui-ready", "Try to mutate.", "STEER", &key(), &viewer()).await;
    assert_eq!(steer.unwrap_err().code(), "permission_denied");

    let orphan = MemberChange {
        subject: "owner".into(),
        display_name: "Olivia Owner".into(),
        email: None,
        role: "EDITOR".into(),
    };
    let orphaned = ops.set_member("ui-ready", &orphan, &owner()).await.unwrap_err();
    assert_eq!(orphaned.http_status(), 422);
    assert!(orphaned.message.contains("retain at least one owner"));

    let events = activity(&test, "ui-ready").await;
    assert_eq!(events[0].actor, "Olivia Owner");
    test.close().await;
}

#[tokio::test]
async fn resource_guidance_is_marked_as_product_generated() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    create_project(ops, "ui-ready").await;
    ops.set_variable(
        "ui-ready",
        "DATASET_URL",
        "https://example.test/eval.jsonl",
        &key(),
        &owner(),
    )
    .await
    .unwrap();
    ops.steer("ui-ready", "Prefer the dev split first.", "STEER", &key(), &owner()).await.unwrap();

    let guidance: Vec<(Value, Option<Value>)> = activity(&test, "ui-ready")
        .await
        .into_iter()
        .filter(|event| event.event_type == "guidance.received")
        .map(|event| (event.detail["body"].clone(), event.detail.get("origin").cloned()))
        .collect();
    assert_eq!(
        guidance,
        [
            (json!("Project variable 'DATASET_URL' was set."), Some(json!("resource"))),
            (json!("Prefer the dev split first."), None),
        ]
    );
    test.close().await;
}

#[tokio::test]
async fn idempotency_keys_are_scoped_to_signed_subjects() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    let shared = "shared-client-key";
    for (principal, slug) in [(owner(), "first"), (namesake(), "second")] {
        let mut project = new_project(slug);
        project.mission = format!("Run {slug} mission.");
        let created = ops.create_project(project, shared, &principal).await.unwrap();
        assert_eq!(created.slug, slug);
    }
    let slugs = |principal: Principal| async move {
        let page = ops.list_projects(false, None, 100, &principal).await.unwrap();
        page.items.into_iter().map(|item| item.slug).collect::<Vec<_>>()
    };
    assert_eq!(slugs(owner()).await, ["first"]);
    assert_eq!(slugs(namesake()).await, ["second"]);
    // A retry with the same key and identity replays the first result.
    let replay = ops.create_project(new_project("first"), shared, &owner()).await.unwrap();
    assert_eq!(replay.slug, "first");
    assert_ne!(scoped_command_id(&owner(), shared), scoped_command_id(&namesake(), shared));
    test.close().await;
}

#[tokio::test]
async fn live_tickets_are_single_use_and_carry_the_callers_role() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    create_project(ops, "ui-ready").await;
    add_member(ops, "ui-ready", &viewer(), "VIEWER").await;

    let grant = ops.issue_live_ticket("ui-ready", &viewer()).await.unwrap();
    let collaboration = test.runtime.collaboration();
    let (principal, role) =
        collaboration.consume_live_ticket("ui-ready", &grant.ticket).await.unwrap();
    assert_eq!((principal.subject.as_str(), role.as_str()), ("viewer", "VIEWER"));
    let reused = collaboration.consume_live_ticket("ui-ready", &grant.ticket).await.unwrap_err();
    assert_eq!(reused.message, "The live attachment ticket is invalid or expired.");

    assert!(ops.issue_live_ticket("ui-ready", &admin()).await.is_err());
    add_member(ops, "ui-ready", &admin(), "VIEWER").await;
    let admin = ops.issue_live_ticket("ui-ready", &admin()).await.unwrap();
    let (principal, role) =
        collaboration.consume_live_ticket("ui-ready", &admin.ticket).await.unwrap();
    assert!(principal.instance_admin);
    assert_eq!(role.as_str(), "VIEWER");
    let local = Principal::local("Operator", true);
    let grant = ops.issue_live_ticket("ui-ready", &local).await.unwrap();
    let (principal, role) =
        collaboration.consume_live_ticket("ui-ready", &grant.ticket).await.unwrap();
    assert!(principal.project_admin);
    assert_eq!(role.as_str(), "OWNER");
    test.close().await;
}

#[tokio::test]
async fn kickoff_knowledge_sources_runs_and_analytics_are_queryable() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    let service = test.runtime.service();
    create_project(ops, "ui-ready").await;

    let changes =
        DraftChanges { context: Some("A richer kickoff context.".into()), ..Default::default() };
    let updated = ops.update_project("ui-ready", &changes, &owner()).await.unwrap();
    assert_eq!(updated.context, "A richer kickoff context.");
    assert_eq!(ops.kickoff_templates().len(), 3);
    assert!(ops.preflight("ui-ready", &owner()).await.unwrap().ready);

    let source = SourceInput {
        name: "benchmark".into(),
        source_type: "URL".into(),
        uri: "https://example.test/benchmark".into(),
        media_type: None,
        metadata: detail(json!({"purpose": "evaluation"})),
    };
    ops.set_source("ui-ready", &source, &owner()).await.unwrap();
    for (name, source_type, uri) in [
        ("unsafe", "URL", "https://example.test/data?access_token=secret"),
        ("unsafe-connector", "CONNECTOR", "github://user:token@example.test/repository"),
    ] {
        let source = SourceInput {
            name: name.into(),
            source_type: source_type.into(),
            uri: uri.into(),
            media_type: None,
            metadata: Map::new(),
        };
        let rejected = ops.set_source("ui-ready", &source, &owner()).await.unwrap_err();
        assert_eq!(rejected.http_status(), 422, "{uri}");
    }
    let upload =
        Upload { name: "brief", filename: Some("brief.md"), content_type: Some("text/markdown") };
    let uploaded =
        ops.upload_source("ui-ready", upload, &b"# Brief\nEvidence"[..], &owner()).await.unwrap();
    assert_eq!(uploaded.source_type, "UPLOAD");
    assert_eq!(uploaded.media_type.as_deref(), Some("text/markdown"));
    let relative = uploaded.uri.strip_prefix("workspace://").unwrap();
    assert!(relative.starts_with(".limina/sources/") && relative.ends_with("/brief.md"));
    let stored = test.runtime.workspace_root().join("ui-ready").join(relative);
    assert_eq!(std::fs::read(stored).unwrap(), b"# Brief\nEvidence");
    assert_eq!(uploaded.metadata["filename"], "brief.md");
    assert_eq!(uploaded.metadata["size_bytes"], 16);
    use sha2::Digest as _;
    let expected: String = sha2::Sha256::digest(b"# Brief\nEvidence")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(uploaded.metadata["sha256"], expected);

    let first = service
        .create_hypothesis(
            "ui-ready",
            hypothesis("Hybrid retrieval", "Full-text retrieval narrows the graph."),
            "Limina",
            &key(),
        )
        .await
        .unwrap();
    let second = service
        .create_hypothesis(
            "ui-ready",
            hypothesis("Graph navigation", "Explicit links improve review."),
            "Limina",
            &key(),
        )
        .await
        .unwrap();
    let query = |text: Option<&str>, tag: Option<&str>, cursor: Option<String>, limit: i64| {
        KnowledgeQuery {
            query: text.map(str::to_owned),
            tag: tag.map(str::to_owned),
            cursor,
            limit,
            ..KnowledgeQuery::default()
        }
    };
    let search = ops
        .query_knowledge("ui-ready", &query(Some("retrieval"), None, None, 50), &owner())
        .await
        .unwrap();
    assert_eq!(search.items[0].id, first.id);
    assert_eq!(search.search_backend, "portable-substring");
    assert_eq!(search.items[0].observations, None);
    let wildcard =
        ops.query_knowledge("ui-ready", &query(Some("%"), None, None, 50), &owner()).await.unwrap();
    assert_eq!(wildcard.total, 0);
    let first_page =
        ops.query_knowledge("ui-ready", &query(None, None, None, 1), &owner()).await.unwrap();
    let next = first_page.next_cursor.clone().unwrap();
    let second_page =
        ops.query_knowledge("ui-ready", &query(None, None, Some(next), 1), &owner()).await.unwrap();
    assert_ne!(first_page.items[0].id, second_page.items[0].id);
    let tagged = ops.add_tag("ui-ready", &first.id, "retrieval", &owner()).await.unwrap();
    assert_eq!(tagged.tags, ["retrieval"]);
    let by_tag = ops
        .query_knowledge("ui-ready", &query(None, Some("retrieval"), None, 50), &owner())
        .await
        .unwrap();
    assert_eq!(by_tag.total, 1);
    assert_eq!(by_tag.items[0].tags, ["retrieval"]);

    let relation = NewRelation {
        source_id: first.id.clone(),
        target_id: second.id.clone(),
        relation_type: "SUPPORTS".into(),
        description: String::new(),
    };
    ops.create_relation("ui-ready", &relation, &owner()).await.unwrap();
    let graph = ops.knowledge_graph("ui-ready", &owner()).await.unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges[0].relation_type, "SUPPORTS");

    ops.add_comment("ui-ready", &first.id, "Please test vocabulary drift.", &key(), &owner())
        .await
        .unwrap();
    create_project(ops, "comment-boundary").await;
    let other = service
        .create_hypothesis(
            "comment-boundary",
            hypothesis("Separate tenant artifact", "Comments remain tenant scoped."),
            "Limina",
            &key(),
        )
        .await
        .unwrap();
    let reused = "predictable-comment-key";
    ops.add_comment("ui-ready", &first.id, "Private project comment.", reused, &owner())
        .await
        .unwrap();
    let collision = ops
        .add_comment("comment-boundary", &other.id, "A different comment.", reused, &owner())
        .await
        .unwrap_err();
    assert_eq!(collision.http_status(), 409);
    assert!(!collision.to_body().to_string().contains("Private project comment"));
    let revisions = ops.revisions("ui-ready", &first.id, &owner()).await.unwrap();
    assert_eq!(revisions[0].version, 1);
    let view = detail(json!({"kind": "H", "status": "PROPOSED"}));
    ops.save_view("ui-ready", "Open hypotheses", &view, &owner()).await.unwrap();

    let receipt = ops
        .steer("ui-ready", "Prioritize the strongest evidence.", "STEER", &key(), &owner())
        .await
        .unwrap();
    assert_eq!(receipt.delivery, Delivery::Queued);
    ops.apply_lifecycle("ui-ready", "start", &key(), &owner()).await.unwrap();
    wait_until("the run to complete", || async {
        let runs = ops.runs("ui-ready", None, None, 50, &owner()).await.unwrap();
        runs.items.first().is_some_and(|run| run.status == "COMPLETED")
    })
    .await;
    let run = ops.runs("ui-ready", None, None, 50, &owner()).await.unwrap().items.remove(0);
    assert_eq!(run.usage.input_tokens, Some(120));
    assert_eq!(run.tool_calls, 1);
    let detail = ops.run("ui-ready", &run.id, &owner()).await.unwrap();
    assert!(detail.events.len() >= 3);
    let shown = serde_json::to_string(&detail).unwrap();
    assert!(!shown.contains("private-thread"), "{shown}");
    assert!(!shown.contains("private-continuation"));
    assert!(shown.contains("Updating durable project knowledge"));

    let history = ops.guidance("ui-ready", None, None, 50, &owner()).await.unwrap();
    assert_eq!(history.items[0].body, "Prioritize the strongest evidence.");
    assert_eq!(history.items[0].status, "ACKNOWLEDGED");
    let analytics = ops.analytics("ui-ready", 30, &owner()).await.unwrap();
    assert_eq!(analytics.runs.total, 1);
    assert_eq!(analytics.knowledge.by_kind.get("H"), Some(2));
    test.close().await;
}

#[tokio::test]
async fn guidance_events_carry_their_body_and_a_stable_guidance_id() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    let service = test.runtime.service();
    create_project(ops, "ui-ready").await;
    let receipt = ops
        .steer("ui-ready", "Prefer the stronger baseline.", "STEER", &key(), &owner())
        .await
        .unwrap();
    service.acknowledge_message("ui-ready", &receipt.id, "limina:test", &key()).await.unwrap();
    let legacy = service
        .send_message("ui-ready", "ANSWER", "Use the March snapshot.", None, "Olivia Owner", &key())
        .await
        .unwrap();
    // An event recorded before guidance bodies were part of the payload.
    let stored = service
        .events("ui-ready", 0, 1000)
        .await
        .unwrap()
        .into_iter()
        .find(|event| event.payload["message_id"] == legacy.id.as_str())
        .unwrap();
    let mut payload = stored.payload.as_object().cloned().unwrap();
    payload.remove("body");
    let mut tx = test.runtime.db().begin_write().await.unwrap();
    tx.execute(
        "UPDATE events SET payload = $1 WHERE id = $2",
        args![Value::Object(payload), &stored.id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let events = activity(&test, "ui-ready").await;
    let received: Vec<(Value, Value)> = events
        .iter()
        .filter(|event| event.event_type == "guidance.received")
        .map(|event| (event.detail["guidance_id"].clone(), event.detail["body"].clone()))
        .collect();
    assert_eq!(
        received,
        [
            (json!(receipt.id), json!("Prefer the stronger baseline.")),
            (json!(legacy.id), json!("Use the March snapshot.")),
        ]
    );
    let incorporated: Vec<_> =
        events.iter().filter(|event| event.event_type == "guidance.incorporated").collect();
    assert_eq!(incorporated[0].detail["guidance_id"], json!(receipt.id));
    assert!(!serde_json::to_string(&events).unwrap().contains("message_id"));
    test.close().await;
}

#[tokio::test]
async fn checkpoint_events_explain_what_the_checkpoint_decided() {
    let test = runtime(completing_agent()).await;
    let service = test.runtime.service();
    create_project(test.ops(), "ui-ready").await;
    let coordinator = service.get_challenge("ui-ready").await.unwrap().coordinator.unwrap();
    let checkpoint = Checkpoint {
        current_objective: "Compare the rerankers.".into(),
        next_step: "Run the frozen benchmark.".into(),
        blocker: "Awaiting GPU quota.".into(),
        status: "CREATED".into(),
        worker_id: Some("limina:worker".into()),
        continuation_id: Some("private-continuation".into()),
        inbox_cursor: 0,
        expected_version: coordinator.version,
        ..Checkpoint::default()
    };
    service.checkpoint_coordinator("ui-ready", checkpoint, "limina:worker", &key()).await.unwrap();

    let events = activity(&test, "ui-ready").await;
    let checkpoint =
        events.iter().rev().find(|event| event.event_type == "project.checkpoint").unwrap();
    assert_eq!(checkpoint.actor, "Limina");
    assert_eq!(checkpoint.detail["current_objective"], "Compare the rerankers.");
    assert_eq!(checkpoint.detail["next_step"], "Run the frozen benchmark.");
    assert_eq!(checkpoint.detail["blocker"], "Awaiting GPU quota.");
    for private in ["continuation_id", "worker_id", "version"] {
        assert!(!checkpoint.detail.contains_key(private), "{private}");
    }
    test.close().await;
}

#[tokio::test]
async fn events_page_backward_with_an_earlier_history_flag() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    create_project(ops, "ui-ready").await;
    let mut rows = Vec::new();
    for marker in 0..10 {
        rows.push(("test.marker", "tester", json!({"marker": marker})));
        rows.push(("coordinator.claimed", "limina:worker", json!({})));
    }
    insert_events(&test, "ui-ready", &rows).await;
    let newest = ops.get_status("ui-ready", &owner()).await.unwrap().event_cursor;
    let markers = |page: &limina_server::app::EventPage| -> Vec<Value> {
        page.events.iter().map(|event| event.detail["marker"].clone()).collect()
    };

    let latest = ops.activity("ui-ready", 0, Some(newest + 1), 3, &owner()).await.unwrap();
    assert_eq!(markers(&latest), [json!(7), json!(8), json!(9)]);
    assert!(latest.has_earlier);
    let older =
        ops.activity("ui-ready", 0, Some(latest.events[0].sequence), 3, &owner()).await.unwrap();
    assert_eq!(markers(&older), [json!(4), json!(5), json!(6)]);
    let rest =
        ops.activity("ui-ready", 0, Some(older.events[0].sequence), 100, &owner()).await.unwrap();
    assert_eq!(rest.events[0].event_type, "project.created");
    assert_eq!(rest.events.len(), 5);
    assert!(!rest.has_earlier);

    let forward = ops.activity("ui-ready", 0, None, 2, &owner()).await.unwrap();
    assert!(!forward.has_earlier);
    assert_eq!(forward.cursor, forward.events.last().unwrap().sequence);
    let resumed = ops.activity("ui-ready", forward.cursor, None, 200, &owner()).await.unwrap();
    assert!(resumed.has_earlier);
    assert_eq!(resumed.cursor, newest);
    // An empty backward page keeps a cursor just below `before`.
    let empty = ops.activity("ui-ready", 0, Some(1), 10, &owner()).await.unwrap();
    assert_eq!((empty.events.len(), empty.cursor), (0, 0));
    test.close().await;
}

#[tokio::test]
async fn project_responses_include_the_callers_role() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    let mut project = new_project("ui-ready");
    project.mission = "Expose roles.".into();
    let created = ops.create_project(project, &key(), &owner()).await.unwrap();
    assert_eq!(created.role.unwrap().as_str(), "OWNER");
    add_member(ops, "ui-ready", &viewer(), "VIEWER").await;
    add_member(ops, "ui-ready", &editor(), "EDITOR").await;

    let role = |principal: Principal| async move {
        ops.get_project("ui-ready", &principal).await.unwrap().role.unwrap().as_str()
    };
    assert_eq!(role(viewer()).await, "VIEWER");
    assert_eq!(role(editor()).await, "EDITOR");
    assert!(ops.get_project("ui-ready", &admin()).await.is_err());
    add_member(ops, "ui-ready", &admin(), "VIEWER").await;
    assert_eq!(role(admin()).await, "VIEWER");
    let listed = ops.list_projects(false, None, 100, &viewer()).await.unwrap();
    assert_eq!(
        listed.items.iter().map(|item| item.role.unwrap().as_str()).collect::<Vec<_>>(),
        ["VIEWER"]
    );
    let status = ops.get_status("ui-ready", &editor()).await.unwrap();
    assert_eq!(status.project.role.unwrap().as_str(), "EDITOR");
    let changes =
        DraftChanges { context: Some("Roles drive UI affordances.".into()), ..Default::default() };
    assert_eq!(
        ops.update_project("ui-ready", &changes, &owner()).await.unwrap().role.unwrap().as_str(),
        "OWNER"
    );
    let stopped = ops.apply_lifecycle("ui-ready", "stop", &key(), &editor()).await.unwrap();
    assert_eq!(stopped.role.unwrap().as_str(), "EDITOR");
    let archive = ops.apply_lifecycle("ui-ready", "archive", &key(), &editor()).await.unwrap_err();
    assert_eq!(archive.details["required_role"], "OWNER");
    let archived = ops.apply_lifecycle("ui-ready", "archive", &key(), &owner()).await.unwrap();
    assert_eq!(archived.status, "ARCHIVED");
    assert_eq!(ops.list_projects(false, None, 100, &owner()).await.unwrap().total, 0);
    assert_eq!(ops.list_projects(true, None, 100, &owner()).await.unwrap().total, 1);
    test.close().await;
}

#[tokio::test]
async fn engine_availability_reports_readiness_without_secret_values() {
    let test = runtime(completing_agent()).await;
    test.sign_in_runtime_codex();
    test.set_env("ANTHROPIC_API_KEY", "sk-ant-top-secret-value");
    test.set_env("CLAUDE_CODE_OAUTH_TOKEN", "oauth-top-secret-value");
    let engines = test.ops().engine_availability().await;
    assert!(!serde_json::to_string(&engines).unwrap().contains("top-secret"));
    let codex = engines.iter().find(|item| item.engine == "codex").unwrap();
    assert_eq!(codex.label, "Codex");
    assert!(codex.installed && codex.credentials);
    assert_eq!(codex.detail, "Signed in with ChatGPT (olivia@example.test)");
    assert_eq!(codex.model.as_deref(), Some("gpt-5.4"));
    let claude = engines.iter().find(|item| item.engine == "claude-code").unwrap();
    assert_eq!(claude.label, "Claude");
    assert!(claude.credentials);
    assert!(claude.detail.contains("ANTHROPIC_API_KEY"));

    test.remove_env("ANTHROPIC_API_KEY");
    let engines = test.ops().engine_availability().await;
    let claude = engines.iter().find(|item| item.engine == "claude-code").unwrap();
    assert!(claude.credentials);
    assert!(claude.detail.contains("CLAUDE_CODE_OAUTH_TOKEN"));
    assert!(!serde_json::to_string(&engines).unwrap().contains("top-secret"));
    test.close().await;
}

#[tokio::test]
async fn preflight_runtime_check_requires_engine_credentials() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    create_project(ops, "ui-ready").await;
    let runtime_check = || async {
        let preflight = ops.preflight("ui-ready", &owner()).await.unwrap();
        assert!(preflight.ready);
        preflight.checks.into_iter().find(|check| check.name == "runtime").unwrap()
    };
    let status = |check: &limina_domain::collaboration::PreflightCheck| {
        serde_json::to_value(check.status).unwrap().as_str().unwrap().to_owned()
    };

    let missing = runtime_check().await;
    assert_eq!(status(&missing), "WARN");
    assert!(missing.detail.contains("Sign in with ChatGPT"));

    test.sign_in_runtime_codex();
    let ready = runtime_check().await;
    assert_eq!(status(&ready), "PASS");
    assert!(ready.detail.contains("Signed in with ChatGPT"));

    // A runtime login in progress owns the credential store.
    test.codex.hold_logins();
    let login = ops.runtime_codex_login("chatgpt", &key(), &admin()).await.unwrap();
    let limina_server::app::RuntimeCodexLogin::Device(attempt) = login else {
        panic!("a ChatGPT login starts a device login")
    };
    let updating = runtime_check().await;
    assert_eq!(status(&updating), "WARN");
    assert!(updating.detail.contains("being updated"));
    let cancelled = ops.runtime_codex_cancel_login(&attempt.login_id, &admin()).await.unwrap();
    assert_eq!(cancelled.status, "CANCELLED");

    let changes = DraftChanges { runtime: Some("claude-code".into()), ..Default::default() };
    ops.update_project("ui-ready", &changes, &owner()).await.unwrap();
    let unconfigured = runtime_check().await;
    assert_eq!(status(&unconfigured), "WARN");
    assert!(unconfigured.detail.contains("Connect an account"));
    test.close().await;
}

// ─── test_cloud_api.py ────────────────────────────────────────────────────────

fn cloud_project(slug: &str) -> NewProject {
    NewProject {
        slug: slug.into(),
        name: "Cloud test".into(),
        mission: "Prove that Limina owns execution.".into(),
        success_criteria: "A teammate can steer the managed live turn.".into(),
        context: "API integration test.".into(),
        runtime: Some("codex".into()),
        model: None,
        reasoning_effort: None,
    }
}

#[tokio::test]
async fn project_lifecycle_resources_and_sanitized_status() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let ops = test.ops();
    let replay_key = key();
    let first =
        ops.create_project(cloud_project("cloud-test"), &replay_key, &owner()).await.unwrap();
    let second =
        ops.create_project(cloud_project("cloud-test"), &replay_key, &owner()).await.unwrap();
    assert_eq!(serde_json::to_value(&first).unwrap(), serde_json::to_value(&second).unwrap());
    assert_eq!((first.status.as_str(), first.runtime.as_str()), ("CREATED", "codex"));
    let created = &activity(&test, "cloud-test").await[0];
    assert_eq!(created.event_type, "project.created");
    assert_eq!(created.detail["runtime"], "codex");
    assert!(!created.detail.contains_key("runtime_engine"));

    let variable = ops
        .set_variable(
            "cloud-test",
            "SOURCE_URL",
            "https://example.invalid/repository",
            &key(),
            &owner(),
        )
        .await
        .unwrap();
    assert_eq!(variable.resource_type, "VARIABLE");
    assert_eq!(variable.value.as_deref(), Some("https://example.invalid/repository"));
    let secret_value = "super-sensitive-token-123";
    let secret =
        ops.set_secret("cloud-test", "SOURCE_TOKEN", secret_value, &key(), &owner()).await.unwrap();
    let secret_json = serde_json::to_value(&secret).unwrap();
    assert_eq!(secret_json["type"], "SECRET");
    assert_eq!(secret_json["configured"], true);
    assert!(secret_json.get("value").is_none());
    assert!(!secret_json.to_string().contains(secret_value));
    let resources =
        serde_json::to_string(&ops.list_resources("cloud-test", &owner()).await.unwrap()).unwrap();
    let review =
        serde_json::to_string(&ops.review("cloud-test", None, 50, &owner()).await.unwrap())
            .unwrap();
    let events = serde_json::to_string(&activity(&test, "cloud-test").await).unwrap();
    for shown in [resources, review, events] {
        assert!(!shown.contains(secret_value));
    }
    // Only owners manage secrets.
    add_member(ops, "cloud-test", &editor(), "EDITOR").await;
    let denied = ops.set_secret("cloud-test", "OTHER", "x", &key(), &editor()).await.unwrap_err();
    assert_eq!(denied.code(), "permission_denied");

    ops.apply_lifecycle("cloud-test", "start", &key(), &owner()).await.unwrap();
    session.started(1).await;
    let status = serde_json::to_string(&ops.get_status("cloud-test", &owner()).await.unwrap())
        .unwrap()
        .to_lowercase();
    for forbidden in ["thread_id", "continuation_id", "worker_id", "inbox_cursor", "version"] {
        assert!(!status.contains(forbidden), "{forbidden} in {status}");
    }
    test.close().await;
}

#[tokio::test]
async fn project_creation_selects_a_supported_runtime_engine() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    let mut project = cloud_project("claude-project");
    project.runtime = Some("claude-code".into());
    assert_eq!(
        ops.create_project(project.clone(), &key(), &owner()).await.unwrap().runtime,
        "claude-code"
    );
    project.slug = "invalid-project".into();
    project.runtime = Some("other".into());
    let rejected = ops.create_project(project, &key(), &owner()).await.unwrap_err();
    assert_eq!((rejected.http_status(), rejected.code()), (422, "invariant_violation"));
    test.close().await;
}

#[tokio::test]
async fn steering_reaches_the_active_managed_turn_live() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let ops = test.ops();
    ops.create_project(cloud_project("cloud-test"), &key(), &owner()).await.unwrap();
    ops.apply_lifecycle("cloud-test", "start", &key(), &owner()).await.unwrap();
    session.started(1).await;

    let reviewer = Principal::new("reviewer", "reviewer");
    add_member(ops, "cloud-test", &reviewer, "EDITOR").await;
    let receipt = ops
        .steer("cloud-test", "Prioritize generalization.", "STEER", &key(), &reviewer)
        .await
        .unwrap();
    assert_eq!(receipt.delivery, Delivery::Live);
    assert_eq!(*session.steering.lock().unwrap(), ["reviewer: Prioritize generalization."]);
    let shown = serde_json::to_value(&receipt).unwrap();
    let keys: Vec<&str> = shown.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["id", "delivery", "kind", "accepted_at", "status"]);

    test.runtime.supervisor().join("cloud-test").await;
    let queued = ops
        .steer("cloud-test", "Now test the strongest baseline.", "STEER", &key(), &owner())
        .await
        .unwrap();
    assert_eq!(queued.delivery, Delivery::Queued);
    assert_eq!(queued.status, "PENDING");
    test.close().await;
}

#[tokio::test]
async fn interrupt_steering_pauses_a_running_project() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let ops = test.ops();
    ops.create_project(cloud_project("cloud-test"), &key(), &owner()).await.unwrap();
    ops.apply_lifecycle("cloud-test", "start", &key(), &owner()).await.unwrap();
    session.started(1).await;

    let receipt = ops
        .steer("cloud-test", "Pause and await human direction.", "interrupt", &key(), &owner())
        .await
        .unwrap();
    assert_eq!((receipt.delivery, receipt.kind.as_str()), (Delivery::Live, "INTERRUPT"));
    test.runtime.supervisor().join("cloud-test").await;
    assert_eq!(ops.get_project("cloud-test", &owner()).await.unwrap().status, "PAUSED");
    test.close().await;
}

#[tokio::test]
async fn private_agent_commands_require_a_project_scoped_capability() {
    let test = runtime(completing_agent()).await;
    test.ops().create_project(cloud_project("cloud-test"), &key(), &owner()).await.unwrap();
    let capability = test.runtime.supervisor().issue_capability("cloud-test").unwrap();
    let actor = test
        .runtime
        .internal_actor(Some(&format!("Bearer {capability}")), "cloud-test", None)
        .unwrap();
    let new = NewHypothesis {
        title: "Scoped write".into(),
        statement: "The capability permits only its managed project.".into(),
        ..NewHypothesis::default()
    };
    let created =
        test.runtime.service().create_hypothesis("cloud-test", new, &actor, &key()).await.unwrap();
    assert_eq!(created.id, "H001");

    let denied = test.runtime.internal_actor(Some("Bearer wrong"), "cloud-test", None).unwrap_err();
    assert_eq!(denied.http_status(), 401);
    let missing = test.runtime.internal_actor(None, "cloud-test", None).unwrap_err();
    assert_eq!(missing.message, "An internal project capability is required.");
    test.close().await;
}

#[tokio::test]
async fn activity_cursor_replays_only_new_durable_events() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    ops.create_project(cloud_project("cloud-test"), &key(), &owner()).await.unwrap();
    let first = ops.activity("cloud-test", 0, None, 200, &owner()).await.unwrap();
    let types = |page: &limina_server::app::EventPage| -> Vec<String> {
        page.events.iter().map(|event| event.event_type.clone()).collect()
    };
    assert_eq!(types(&first), ["project.created"]);

    ops.set_variable("cloud-test", "BRIEF_URI", "s3://brief", &key(), &owner()).await.unwrap();
    let second = ops.activity("cloud-test", first.cursor, None, 200, &owner()).await.unwrap();
    assert_eq!(types(&second), ["resource.variable_set", "guidance.received"]);
    assert!(second.cursor > first.cursor);
    test.close().await;
}

#[tokio::test]
async fn domain_errors_have_stable_machine_readable_shape() {
    let test = runtime(completing_agent()).await;
    let error = test.ops().get_status("missing", &owner()).await.unwrap_err();
    assert_eq!(error.http_status(), 404);
    let body = error.to_body();
    assert_eq!(body["error"]["code"], "not_found");
    assert!(body["error"]["message"].as_str().unwrap().contains("does not exist"));
    test.close().await;
}

#[tokio::test]
async fn review_recent_activity_uses_the_newest_events_after_large_histories() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    ops.create_project(cloud_project("cloud-test"), &key(), &owner()).await.unwrap();
    let rows: Vec<(&str, &str, Value)> =
        (0..1005).map(|marker| ("test.marker", "tester", json!({"marker": marker}))).collect();
    insert_events(&test, "cloud-test", &rows).await;
    let review = ops.review("cloud-test", None, 50, &owner()).await.unwrap();
    assert_eq!(review.recent_activity.len(), 50);
    assert_eq!(review.recent_activity.last().unwrap().detail["marker"], 1004);
    test.close().await;
}

#[tokio::test]
async fn runtime_codex_administration_requires_an_instance_admin() {
    let test = runtime(completing_agent()).await;
    let ops = test.ops();
    let denied = ops.runtime_codex_status(&owner()).await.unwrap_err();
    assert_eq!(
        (denied.http_status(), denied.message.as_str()),
        (403, "Instance administrator access is required.")
    );
    let status = ops.runtime_codex_status(&admin()).await.unwrap();
    assert!(!status.configured);
    assert_eq!(ops.runtime_codex_logout(&owner()).await.unwrap_err().code(), "permission_denied");
    let logged_out = ops.runtime_codex_logout(&admin()).await.unwrap();
    assert!(!logged_out.configured);
    let missing = ops.runtime_codex_login_attempt("nope", &admin()).await.unwrap_err();
    assert_eq!(missing.message, "The Codex login attempt does not exist.");
    test.close().await;
}

#[tokio::test]
async fn uploads_over_the_limit_are_refused_and_removed() {
    let test =
        runtime_with(factory(completing_agent()), |config| config.upload_limit_bytes = 4).await;
    let ops = test.ops();
    create_project(ops, "ui-ready").await;
    let upload = Upload { name: "big", filename: Some("../big file.bin"), content_type: None };
    let error =
        ops.upload_source("ui-ready", upload, &b"too large"[..], &owner()).await.unwrap_err();
    assert_eq!(error.message, "Uploaded sources must be at most 4 bytes.");
    let sources = test.runtime.workspace_root().join("ui-ready/.limina/sources");
    let kept: Vec<_> = walk(&sources).into_iter().filter(|path| path.is_file()).collect();
    assert!(kept.is_empty(), "{kept:?}");
    assert!(ops.sources("ui-ready", &owner()).await.unwrap().is_empty());
    let viewer_upload = Upload { name: "x", filename: None, content_type: None };
    add_member(ops, "ui-ready", &viewer(), "VIEWER").await;
    let denied =
        ops.upload_source("ui-ready", viewer_upload, &b""[..], &viewer()).await.unwrap_err();
    assert_eq!(denied.code(), "permission_denied");
    test.close().await;
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(walk(&path));
            }
            found.push(path);
        }
    }
    found
}
