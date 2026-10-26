//! `tests/test_cloud_ui_readiness.py` over HTTP: roles enforced centrally, product-generated
//! guidance, per-subject idempotency, single-use live tickets with read-only viewers, kickoff,
//! knowledge, sources (multipart upload included), runs and analytics, guidance and checkpoint
//! events, backward event paging, `/v1/me`, the caller's role on every project response, engine
//! readiness without secrets, and the preflight runtime check.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use app_support::*;
use axum::http::Method;
use http_support::*;
use limina_domain::research::NewHypothesis;
use limina_domain::service::Checkpoint;
use limina_engines::RuntimeEvent;
use limina_store::args;
use serde_json::{Value, json};

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

async fn api() -> Api {
    Api::new(runtime(completing_agent()).await, ui_tokens()).await
}

fn a(token: &str) -> Vec<(String, String)> {
    auth(token)
}

fn cmd(token: &str) -> Vec<(String, String)> {
    command(token, None, "spoofed actor")
}

async fn create_project(api: &Api, slug: &str) {
    let body = json!({
        "slug": slug,
        "name": "UI ready",
        "objective": "Make every project surface queryable.",
        "success_criteria": "The typed API supports a collaborative UI.",
    });
    api.post("/v1/projects", &h(&cmd("owner-token")), &body).await.expect(201);
}

async fn add_member(api: &Api, subject: &str, display_name: &str, role: &str) {
    let body = json!({"subject": subject, "display_name": display_name, "role": role});
    api.put("/v1/projects/ui-ready/members", &h(&a("owner-token")), &body).await.expect(200);
}

async fn events(api: &Api, query: &str) -> Value {
    let path = format!("/v1/projects/ui-ready/events{query}");
    api.get(&path, &h(&a("owner-token"))).await.expect(200).json()
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

#[tokio::test]
async fn oidc_principals_and_project_roles_are_enforced_centrally() {
    let api = api().await;
    create_project(&api, "ui-ready").await;
    let owner_projects = api.get("/v1/projects", &h(&a("owner-token"))).await.json();
    let viewer_projects = api.get("/v1/projects", &h(&a("viewer-token"))).await.json();
    assert_eq!(owner_projects["total"], 1);
    assert_eq!(viewer_projects["total"], 0);
    api.get("/v1/projects/ui-ready", &h(&a("viewer-token"))).await.expect(403);

    let member = json!({
        "subject": "viewer",
        "display_name": "Victor Viewer",
        "email": "viewer@example.test",
        "role": "VIEWER",
    });
    api.put("/v1/projects/ui-ready/members", &h(&a("owner-token")), &member).await.expect(200);
    api.get("/v1/projects/ui-ready", &h(&a("viewer-token"))).await.expect(200);
    let denied = api
        .call(Method::POST, "/v1/projects/ui-ready/actions/start", &h(&cmd("viewer-token")))
        .await;
    assert_error(&denied, 403, "permission_denied");

    let orphan = json!({"subject": "owner", "display_name": "Olivia Owner", "role": "EDITOR"});
    let orphan = api.put("/v1/projects/ui-ready/members", &h(&a("owner-token")), &orphan).await;
    orphan.expect(422);
    assert!(orphan.text().contains("retain at least one owner"));

    let events = events(&api, "").await["events"].clone();
    assert_eq!(events[0]["actor"], "Olivia Owner");
    assert!(!events.to_string().contains("spoofed actor"));
    api.close().await;
}

#[tokio::test]
async fn resource_guidance_is_marked_as_product_generated() {
    let api = api().await;
    create_project(&api, "ui-ready").await;
    api.put(
        "/v1/projects/ui-ready/resources/variables/DATASET_URL",
        &h(&cmd("owner-token")),
        &json!({"value": "https://example.test/eval.jsonl"}),
    )
    .await
    .expect(200);
    api.post(
        "/v1/projects/ui-ready/steering",
        &h(&cmd("owner-token")),
        &json!({"body": "Prefer the dev split first.", "kind": "STEER"}),
    )
    .await
    .expect(202);
    let events = events(&api, "").await["events"].clone();
    let guidance: Vec<(Value, Value)> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "guidance.received")
        .map(|event| {
            (
                event["detail"]["body"].clone(),
                event["detail"].get("origin").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    assert_eq!(
        guidance,
        [
            (json!("Project variable 'DATASET_URL' was set."), json!("resource")),
            (json!("Prefer the dev split first."), Value::Null),
        ]
    );
    api.close().await;
}

#[tokio::test]
async fn idempotency_keys_are_scoped_to_signed_subjects() {
    let api = api().await;
    let shared = "shared-client-key";
    for (token, slug) in [("owner-token", "first"), ("same-name-token", "second")] {
        let body = json!({
            "slug": slug,
            "name": slug.to_uppercase(),
            "objective": format!("Run {slug} mission."),
            "success_criteria": "Produce durable evidence.",
        });
        let response =
            api.post("/v1/projects", &h(&command(token, Some(shared), "x")), &body).await;
        response.expect(201);
        assert_eq!(response.json()["slug"], slug);
    }
    let slugs = |token: &str| {
        let api = &api;
        let token = token.to_owned();
        async move {
            api.get("/v1/projects", &h(&a(&token))).await.json()["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["slug"].clone())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(slugs("owner-token").await, [json!("first")]);
    assert_eq!(slugs("same-name-token").await, [json!("second")]);
    api.close().await;
}

#[tokio::test]
async fn browser_ticket_is_single_use_and_supports_read_only_viewers() {
    let api = api().await;
    create_project(&api, "ui-ready").await;
    add_member(&api, "viewer", "Victor Viewer", "VIEWER").await;
    let ticket =
        api.call(Method::POST, "/v1/projects/ui-ready/live-ticket", &h(&a("viewer-token"))).await;
    ticket.expect(200);
    let value = ticket.json()["ticket"].as_str().unwrap().to_owned();
    let server = api.listen().await;
    let url = server.ws_url("/v1/projects/ui-ready/live");
    let protocols = ["limina.v1".to_owned(), format!("limina.ticket.{value}")];
    let protocols: Vec<&str> = protocols.iter().map(String::as_str).collect();

    let (mut socket, protocol) = connect(&url, &protocols, &[]).await.unwrap();
    assert_eq!(protocol.as_deref(), Some("limina.v1"));
    let snapshot = next_json(&mut socket).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["value"]["project"]["role"], "VIEWER");
    send_json(&mut socket, &json!({"type": "steer", "body": "Try to mutate."})).await;
    let error = next_non_event(&mut socket).await;
    assert_eq!(error["type"], "error");
    assert!(error["value"]["message"].as_str().unwrap().contains("Viewers"));
    assert_eq!(close_of(&mut socket).await.map(|close| close.0), Some(1000));

    // The ticket was used up (an authorization error, as in Python: 4403).
    let (mut reused, _) = connect(&url, &protocols, &[]).await.unwrap();
    let close = close_of(&mut reused).await.unwrap();
    assert_eq!(close, (4403, "The live attachment ticket is invalid or expired.".to_owned()));

    api.call(Method::POST, "/v1/projects/ui-ready/live-ticket", &h(&a("admin-token")))
        .await
        .expect(403);
    add_member(&api, "instance-admin", "Alice Admin", "EDITOR").await;
    let admin_ticket = api
        .call(Method::POST, "/v1/projects/ui-ready/live-ticket", &h(&a("admin-token")))
        .await
        .json()["ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    let admin_protocols = ["limina.v1".to_owned(), format!("limina.ticket.{admin_ticket}")];
    let admin_protocols: Vec<&str> = admin_protocols.iter().map(String::as_str).collect();
    let (mut socket, _) = connect(&url, &admin_protocols, &[]).await.unwrap();
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    send_json(&mut socket, &json!({"type": "steer", "body": "Review the current strategy."})).await;
    let delivery = next_non_event(&mut socket).await;
    assert_eq!(delivery["type"], "delivery");
    api.close().await;
}

/// A `multipart/form-data` body with a `name` field and one file.
fn upload_form(name: &str, filename: &str, content: &[u8], media_type: &str) -> (String, Vec<u8>) {
    let boundary = "upload-boundary-7f3a";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
         Content-Type: {media_type}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[tokio::test]
async fn kickoff_knowledge_sources_runs_and_analytics_are_queryable() {
    let api = api().await;
    let owner_auth = a("owner-token");
    let o = h(&owner_auth);
    create_project(&api, "ui-ready").await;
    let updated = api
        .patch("/v1/projects/ui-ready", &o, &json!({"context": "A richer kickoff context."}))
        .await;
    updated.expect(200);
    assert_eq!(updated.json()["context"], "A richer kickoff context.");
    api.get("/v1/project-templates", &o).await.expect(200);
    let preflight = api.get("/v1/projects/ui-ready/preflight", &o).await.json();
    assert_eq!(preflight["ready"], true);

    let source = json!({
        "name": "benchmark",
        "type": "URL",
        "uri": "https://example.test/benchmark",
        "metadata": {"purpose": "evaluation"},
    });
    api.put("/v1/projects/ui-ready/sources", &o, &source).await.expect(200);
    let credential = json!({"name": "unsafe", "type": "URL", "uri": "https://example.test/data?access_token=secret"});
    api.put("/v1/projects/ui-ready/sources", &o, &credential).await.expect(422);
    let connector = json!({"name": "unsafe-connector", "type": "CONNECTOR", "uri": "github://user:token@example.test/repository"});
    api.put("/v1/projects/ui-ready/sources", &o, &connector).await.expect(422);
    let (content_type, form) =
        upload_form("brief", "brief.md", b"# Brief\nEvidence", "text/markdown");
    let upload = api
        .raw(Method::POST, "/v1/projects/ui-ready/sources/upload", &o, &content_type, form)
        .await;
    upload.expect(201);
    let uploaded = upload.json();
    assert_eq!(uploaded["type"], "UPLOAD");
    assert_eq!(uploaded["media_type"], "text/markdown");
    assert_eq!(uploaded["metadata"]["filename"], "brief.md");
    assert_eq!(uploaded["metadata"]["size_bytes"], 16);
    let stored = uploaded["uri"].as_str().unwrap().strip_prefix("workspace://").unwrap().to_owned();
    let on_disk = api.test.runtime.workspace_root().join("ui-ready").join(&stored);
    assert_eq!(std::fs::read(on_disk).unwrap(), b"# Brief\nEvidence");

    let service = api.test.runtime.service();
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
    let search = api.get("/v1/projects/ui-ready/knowledge?query=retrieval", &o).await;
    search.expect(200);
    assert_eq!(search.json()["items"][0]["id"], json!(first.id));
    assert_eq!(search.json()["search_backend"], "portable-substring");
    let wildcard = api.get("/v1/projects/ui-ready/knowledge?query=%25", &o).await.json();
    assert_eq!(wildcard["total"], 0);
    let first_page = api.get("/v1/projects/ui-ready/knowledge?limit=1", &o).await.json();
    assert!(!first_page["next_cursor"].is_null());
    let second_page = api
        .get(
            &format!(
                "/v1/projects/ui-ready/knowledge?limit=1&cursor={}",
                first_page["next_cursor"].as_str().unwrap()
            ),
            &o,
        )
        .await
        .json();
    assert_ne!(first_page["items"][0]["id"], second_page["items"][0]["id"]);
    let tagged = api
        .call(
            Method::PUT,
            &format!("/v1/projects/ui-ready/knowledge/{}/tags/retrieval", first.id),
            &o,
        )
        .await;
    tagged.expect(200);
    assert_eq!(tagged.json()["tags"], json!(["retrieval"]));
    let tag_search = api.get("/v1/projects/ui-ready/knowledge?tag=retrieval", &o).await.json();
    assert_eq!(tag_search["total"], 1);
    assert_eq!(tag_search["items"][0]["tags"], json!(["retrieval"]));

    let relation = json!({"source_id": first.id, "target_id": second.id, "type": "SUPPORTS"});
    api.post("/v1/projects/ui-ready/knowledge/relations", &o, &relation).await.expect(201);
    let graph = api.get("/v1/projects/ui-ready/knowledge/graph", &o).await.json();
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"][0]["type"], "SUPPORTS");

    let comment_path = format!("/v1/projects/ui-ready/knowledge/{}/comments", first.id);
    api.post(
        &comment_path,
        &h(&cmd("owner-token")),
        &json!({"body": "Please test vocabulary drift."}),
    )
    .await
    .expect(201);
    create_project(&api, "comment-boundary").await;
    let other = service
        .create_hypothesis(
            "comment-boundary",
            NewHypothesis {
                title: "Separate tenant artifact".into(),
                statement: "Comments remain tenant scoped.".into(),
                mechanism: "Project-scoped foreign keys".into(),
                generalization: "Across every project".into(),
                shortcut_risks: "Reused client keys".into(),
                test_plan: "Reuse one key across two projects.".into(),
            },
            "Limina",
            &key(),
        )
        .await
        .unwrap();
    let reused = command("owner-token", Some("predictable-comment-key"), "x");
    api.post(&comment_path, &h(&reused), &json!({"body": "Private project comment."}))
        .await
        .expect(201);
    let collision = api
        .post(
            &format!("/v1/projects/comment-boundary/knowledge/{}/comments", other.id),
            &h(&reused),
            &json!({"body": "A different comment."}),
        )
        .await;
    collision.expect(409);
    assert!(!collision.text().contains("Private project comment"));
    let revisions =
        api.get(&format!("/v1/projects/ui-ready/knowledge/{}/revisions", first.id), &o).await;
    revisions.expect(200);
    assert_eq!(revisions.json()[0]["version"], 1);
    let view = json!({"name": "Open hypotheses", "query": {"kind": "H", "status": "PROPOSED"}});
    api.put("/v1/projects/ui-ready/knowledge/views", &o, &view).await.expect(200);

    let guidance = api
        .post(
            "/v1/projects/ui-ready/steering",
            &h(&cmd("owner-token")),
            &json!({"body": "Prioritize the strongest evidence.", "kind": "STEER"}),
        )
        .await;
    guidance.expect(202);
    api.call(Method::POST, "/v1/projects/ui-ready/actions/start", &h(&cmd("owner-token")))
        .await
        .expect(200);

    let deadline = Instant::now() + Duration::from_secs(5);
    let runs = loop {
        let runs = api.get("/v1/projects/ui-ready/runs", &o).await.json();
        if runs["items"][0]["status"] == "COMPLETED" || Instant::now() > deadline {
            break runs;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let run = &runs["items"][0];
    assert_eq!(run["status"], "COMPLETED");
    assert_eq!(run["usage"]["input_tokens"], 120);
    assert_eq!(run["tool_calls"], 1);
    let detail =
        api.get(&format!("/v1/projects/ui-ready/runs/{}", run["id"].as_str().unwrap()), &o).await;
    detail.expect(200);
    assert!(detail.json()["events"].as_array().unwrap().len() >= 3);
    assert!(!detail.text().contains("private-thread"));
    assert!(!detail.text().contains("private-continuation"));
    assert!(detail.text().contains("Updating durable project knowledge"));

    let history = api.get("/v1/projects/ui-ready/guidance", &o).await.json();
    assert_eq!(history["items"][0]["body"], "Prioritize the strongest evidence.");
    assert_eq!(history["items"][0]["status"], "ACKNOWLEDGED");
    let analytics = api.get("/v1/projects/ui-ready/analytics", &o).await;
    analytics.expect(200);
    assert_eq!(analytics.json()["runs"]["total"], 1);
    assert_eq!(analytics.json()["knowledge"]["by_kind"]["H"], 2);

    let schema = api.get("/openapi.json", &[]).await.json();
    let knowledge = &schema["paths"]["/v1/projects/{slug}/knowledge"]["get"]["responses"]["200"]["content"]
        ["application/json"]["schema"];
    assert_eq!(knowledge, &json!({"$ref": "#/components/schemas/KnowledgePage"}));
    let upload =
        &schema["paths"]["/v1/projects/{slug}/sources/upload"]["post"]["requestBody"]["content"];
    assert!(upload.get("multipart/form-data").is_some());
    api.close().await;
}

#[tokio::test]
async fn guidance_events_carry_their_body_and_a_stable_guidance_id() {
    let api = api().await;
    create_project(&api, "ui-ready").await;
    let receipt = api
        .post(
            "/v1/projects/ui-ready/steering",
            &h(&cmd("owner-token")),
            &json!({"body": "Prefer the stronger baseline.", "kind": "STEER"}),
        )
        .await
        .json();
    let receipt_id = receipt["id"].as_str().unwrap();
    let service = api.test.runtime.service();
    service.acknowledge_message("ui-ready", receipt_id, "limina:test", &key()).await.unwrap();
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
    let mut tx = api.test.runtime.db().begin_write().await.unwrap();
    tx.execute(
        "UPDATE events SET payload = $1 WHERE id = $2",
        args![Value::Object(payload), &stored.id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let events = events(&api, "").await["events"].clone();
    let of_type = |kind: &str| -> Vec<Value> {
        events.as_array().unwrap().iter().filter(|event| event["type"] == kind).cloned().collect()
    };
    let received: Vec<(Value, Value)> = of_type("guidance.received")
        .iter()
        .map(|event| (event["detail"]["guidance_id"].clone(), event["detail"]["body"].clone()))
        .collect();
    assert_eq!(
        received,
        [
            (json!(receipt_id), json!("Prefer the stronger baseline.")),
            (json!(legacy.id), json!("Use the March snapshot.")),
        ]
    );
    assert_eq!(of_type("guidance.incorporated")[0]["detail"]["guidance_id"], json!(receipt_id));
    assert!(!events.to_string().contains("message_id"));
    api.close().await;
}

#[tokio::test]
async fn checkpoint_events_explain_what_the_checkpoint_decided() {
    let api = api().await;
    create_project(&api, "ui-ready").await;
    let service = api.test.runtime.service();
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
    let events = events(&api, "").await["events"].clone();
    let checkpoint = events
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|event| event["type"] == "project.checkpoint")
        .unwrap()
        .clone();
    assert_eq!(checkpoint["actor"], "Limina");
    assert_eq!(checkpoint["detail"]["current_objective"], "Compare the rerankers.");
    assert_eq!(checkpoint["detail"]["next_step"], "Run the frozen benchmark.");
    assert_eq!(checkpoint["detail"]["blocker"], "Awaiting GPU quota.");
    for private in ["continuation_id", "worker_id", "version"] {
        assert!(checkpoint["detail"].get(private).is_none(), "{private}");
    }
    api.close().await;
}

#[tokio::test]
async fn events_page_backward_with_an_earlier_history_flag() {
    let api = api().await;
    create_project(&api, "ui-ready").await;
    let mut rows = Vec::new();
    for marker in 0..10 {
        rows.push(("test.marker", "tester", json!({"marker": marker})));
        rows.push(("coordinator.claimed", "limina:worker", json!({})));
    }
    insert_events(&api.test, "ui-ready", &rows).await;
    let newest =
        api.get("/v1/projects/ui-ready/status", &h(&a("owner-token"))).await.json()["event_cursor"]
            .as_i64()
            .unwrap();
    let markers = |page: &Value| -> Vec<Value> {
        page["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["detail"]["marker"].clone())
            .collect()
    };
    let latest = events(&api, &format!("?before={}&limit=3", newest + 1)).await;
    assert_eq!(markers(&latest), [json!(7), json!(8), json!(9)]);
    assert_eq!(latest["has_earlier"], true);
    let older = events(&api, &format!("?before={}&limit=3", latest["events"][0]["sequence"])).await;
    assert_eq!(markers(&older), [json!(4), json!(5), json!(6)]);
    let rest = events(&api, &format!("?before={}&limit=100", older["events"][0]["sequence"])).await;
    assert_eq!(rest["events"][0]["type"], "project.created");
    assert_eq!(rest["events"].as_array().unwrap().len(), 5);
    assert_eq!(rest["has_earlier"], false);

    let forward = events(&api, "?after=0&limit=2").await;
    assert_eq!(forward["has_earlier"], false);
    assert_eq!(forward["cursor"], forward["events"][1]["sequence"]);
    let resumed = events(&api, &format!("?after={}", forward["cursor"])).await;
    assert_eq!(resumed["has_earlier"], true);
    assert_eq!(resumed["cursor"], newest);
    let invalid = api.get("/v1/projects/ui-ready/events?before=0", &h(&a("owner-token"))).await;
    assert_error(&invalid, 422, "invalid_request");
    // The console's first page asks for everything before the largest safe integer.
    let latest = events(&api, "?before=9007199254740991&limit=300").await;
    assert_eq!(latest["events"].as_array().unwrap().len(), 11);
    api.close().await;
}

#[tokio::test]
async fn me_describes_the_authenticated_principal() {
    let api = api().await;
    let owner = api.get("/v1/me", &h(&a("owner-token"))).await;
    owner.expect(200);
    assert_eq!(
        owner.json(),
        json!({
            "subject": "owner",
            "display_name": "Olivia Owner",
            "email": "owner@example.test",
            "instance_admin": false,
            "auth_mode": "oidc",
        })
    );
    assert_eq!(api.get("/v1/me", &h(&a("admin-token"))).await.json()["instance_admin"], true);
    api.get("/v1/me", &[]).await.expect(401);
    api.close().await;
}

#[tokio::test]
async fn project_responses_include_the_callers_role() {
    let api = api().await;
    let created = api
        .post(
            "/v1/projects",
            &h(&cmd("owner-token")),
            &json!({
                "slug": "ui-ready",
                "name": "UI ready",
                "objective": "Expose roles.",
                "success_criteria": "The UI hides controls a caller cannot use.",
            }),
        )
        .await;
    assert_eq!(created.json()["role"], "OWNER");
    add_member(&api, "viewer", "Victor Viewer", "VIEWER").await;
    add_member(&api, "editor", "Erin Editor", "EDITOR").await;
    let role = |token: &'static str| {
        let api = &api;
        async move {
            let response = api.get("/v1/projects/ui-ready", &h(&a(token))).await;
            response.expect(200);
            response.json()["role"].clone()
        }
    };
    assert_eq!(role("viewer-token").await, "VIEWER");
    assert_eq!(role("editor-token").await, "EDITOR");
    api.get("/v1/projects/ui-ready", &h(&a("admin-token"))).await.expect(403);
    add_member(&api, "instance-admin", "Alice Admin", "VIEWER").await;
    assert_eq!(role("admin-token").await, "VIEWER");
    let listed = api.get("/v1/projects", &h(&a("viewer-token"))).await.json();
    assert_eq!(listed["items"][0]["role"], "VIEWER");
    let status = api.get("/v1/projects/ui-ready/status", &h(&a("editor-token"))).await.json();
    assert_eq!(status["project"]["role"], "EDITOR");
    let updated = api
        .patch(
            "/v1/projects/ui-ready",
            &h(&a("owner-token")),
            &json!({"context": "Roles drive UI affordances."}),
        )
        .await;
    assert_eq!(updated.json()["role"], "OWNER");
    let stopped = api
        .call(Method::POST, "/v1/projects/ui-ready/actions/stop", &h(&cmd("editor-token")))
        .await;
    stopped.expect(200);
    assert_eq!(stopped.json()["role"], "EDITOR");
    api.close().await;
}

#[tokio::test]
async fn engine_availability_reports_readiness_without_secret_values() {
    let api = api().await;
    api.test.sign_in_runtime_codex();
    api.test.set_env("ANTHROPIC_API_KEY", "sk-ant-top-secret-value");
    api.test.set_env("CLAUDE_CODE_OAUTH_TOKEN", "oauth-top-secret-value");
    let response = api.get("/v1/runtime/engines", &h(&a("viewer-token"))).await;
    response.expect(200);
    assert!(!response.text().contains("top-secret"));
    let engines = response.json();
    let engine = |id: &str| {
        engines.as_array().unwrap().iter().find(|item| item["engine"] == id).unwrap().clone()
    };
    assert_eq!(engine("codex")["label"], "Codex");
    assert_eq!(engine("codex")["installed"], true);
    assert_eq!(engine("codex")["credentials"], true);
    assert_eq!(engine("codex")["detail"], "Signed in with ChatGPT (olivia@example.test)");
    assert!(engine("codex")["model"].as_str().is_some_and(|model| !model.is_empty()));
    assert_eq!(engine("claude-code")["label"], "Claude");
    assert_eq!(engine("claude-code")["credentials"], true);
    assert!(engine("claude-code")["detail"].as_str().unwrap().contains("ANTHROPIC_API_KEY"));

    api.test.remove_env("ANTHROPIC_API_KEY");
    let oauth_only = api.get("/v1/runtime/engines", &h(&a("owner-token"))).await;
    let claude = oauth_only
        .json()
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["engine"] == "claude-code")
        .unwrap()
        .clone();
    assert_eq!(claude["credentials"], true);
    assert!(claude["detail"].as_str().unwrap().contains("CLAUDE_CODE_OAUTH_TOKEN"));
    assert!(!oauth_only.text().contains("top-secret"));
    api.get("/v1/runtime/engines", &[]).await.expect(401);
    api.close().await;
}

#[tokio::test]
async fn preflight_runtime_check_requires_engine_credentials() {
    let api = api().await;
    create_project(&api, "ui-ready").await;
    let runtime_check = || {
        let api = &api;
        async move {
            let preflight =
                api.get("/v1/projects/ui-ready/preflight", &h(&a("owner-token"))).await.json();
            assert_eq!(preflight["ready"], true);
            preflight["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|check| check["name"] == "runtime")
                .unwrap()
                .clone()
        }
    };
    let missing = runtime_check().await;
    assert_eq!(missing["status"], "WARN");
    assert!(missing["detail"].as_str().unwrap().contains("Sign in with ChatGPT"));

    api.test.sign_in_runtime_codex();
    let ready = runtime_check().await;
    assert_eq!(ready["status"], "PASS");
    assert!(ready["detail"].as_str().unwrap().contains("Signed in with ChatGPT"));

    // A runtime login in progress owns the credential store.
    api.test.codex.hold_logins();
    let admin_command = command("admin-token", None, "x");
    let login = api
        .post(
            "/v1/runtime/engines/codex/auth/login",
            &h(&admin_command),
            &json!({"method": "chatgpt"}),
        )
        .await;
    login.expect(200);
    assert_eq!(login.json()["status"], "PENDING");
    let updating = runtime_check().await;
    assert_eq!(updating["status"], "WARN");
    assert!(updating["detail"].as_str().unwrap().contains("being updated"));
    let login_path = format!(
        "/v1/runtime/engines/codex/auth/login/{}",
        login.json()["login_id"].as_str().unwrap()
    );
    let cancelled = api.delete(&login_path, &h(&a("admin-token"))).await;
    assert_eq!(cancelled.expect(200).json()["status"], "CANCELLED");

    api.patch("/v1/projects/ui-ready", &h(&a("owner-token")), &json!({"runtime": "claude-code"}))
        .await
        .expect(200);
    let unconfigured = runtime_check().await;
    assert_eq!(unconfigured["status"], "WARN");
    assert!(unconfigured["detail"].as_str().unwrap().contains("Connect an account"));
    api.close().await;
}
