//! `tests/test_cloud_mcp.py`: MCP shares the authenticated public boundary, the projects,
//! resources and audit history of REST, and exposes read-only project knowledge as resources.
//!
//! The Python tests compared MCP with REST routes; here the REST side is the operations those
//! routes return (the HTTP layer serves them unchanged). `/healthz` advertising `/mcp/` belongs to
//! the HTTP layer's tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod mcp_support;

use app_support::*;
use mcp_support::*;
use rmcp::model::ResourceContents;
use serde_json::json;

async fn server() -> McpServer {
    serve(runtime(ScriptedSession::always(Reply::ok(complete()))).await, token_auth()).await
}

#[tokio::test]
async fn mcp_uses_the_same_authenticated_public_boundary() {
    let server = server().await;
    let list = rpc(1, "tools/list", json!({}));

    let anonymous: Vec<_> =
        mcp_headers().into_iter().filter(|(name, _)| *name != "authorization").collect();
    let unauthorized = post(&server.url(), &anonymous, list.clone()).await;
    assert_eq!(unauthorized.status, 401);
    assert_eq!(unauthorized.json()["error"]["code"], "authentication_required");

    let mut rebinding = mcp_headers();
    rebinding.push(("host", "attacker.invalid".into()));
    let rebinding = post(&server.url(), &rebinding, list).await;
    assert_eq!(rebinding.status, 421);

    let client = server.client(Some("maya")).await;
    let listed = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = listed.iter().map(|tool| tool.name.as_ref()).collect();
    for name in ["limina_create_project", "limina_steer_project", "limina_review_project"] {
        assert!(names.contains(&name), "{name} is missing");
    }
    assert!(!names.contains(&"limina_set_project_secret"));
    let create = listed.iter().find(|tool| tool.name == "limina_create_project").unwrap();
    let properties = &create.input_schema["properties"];
    assert_eq!(properties["actor"]["maxLength"], 200);
    assert_eq!(properties["idempotency_key"]["maxLength"], 55);
    let serialized = serde_json::to_string(&listed).unwrap().to_lowercase();
    for forbidden in ["thread_id", "session_id", "worker_id", "subagent"] {
        assert!(!serialized.contains(forbidden), "tools mention {forbidden}");
    }
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn mcp_and_rest_share_projects_resources_and_audit_history() {
    let server = server().await;
    let client = server.client(Some("maya")).await;
    let maya = local("maya");
    let ops = server.test.ops();

    let arguments = json!({
        "slug": "mcp-project",
        "name": "MCP project",
        "mission": "Prove the shared application contract.",
        "success_criteria": "MCP and REST observe one durable project.",
        "runtime": "claude-code",
        "idempotency_key": "create-mcp-project",
    });
    let first = structured(&client, "limina_create_project", arguments.clone()).await;
    let replay = structured(&client, "limina_create_project", arguments).await;
    assert_eq!(first, replay);
    assert_eq!(first["runtime"], "claude-code");

    let project = ops.get_project("mcp-project", &maya).await.unwrap();
    assert_eq!(serde_json::to_value(&project).unwrap(), first);

    let interrupt = json!({
        "project": "mcp-project",
        "message": "Pause before execution and wait for the revised benchmark.",
        "kind": "INTERRUPT",
        "idempotency_key": "interrupt-before-start",
    });
    let interrupted = structured(&client, "limina_steer_project", interrupt.clone()).await;
    let interrupt_replay = structured(&client, "limina_steer_project", interrupt).await;
    assert_eq!(interrupted, interrupt_replay);

    let variable = structured(
        &client,
        "limina_set_project_variable",
        json!({
            "project": "mcp-project",
            "name": "EVAL_SET_URI",
            "value": "s3://research/eval-v3.parquet",
            "idempotency_key": "set-eval-uri",
        }),
    )
    .await;
    assert_eq!(variable["type"], "VARIABLE");

    let resources = ops.list_resources("mcp-project", &maya).await.unwrap();
    assert_eq!(serde_json::to_value(&resources).unwrap(), json!([variable]));
    let preflight =
        structured(&client, "limina_preflight_project", json!({"project": "mcp-project"})).await;
    assert_eq!(preflight["ready"], true);
    let checks: Vec<&str> =
        preflight["checks"].as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert!(checks.contains(&"runtime"));
    // The same readiness the REST route reports.
    let rest = ops.preflight("mcp-project", &maya).await.unwrap();
    assert_eq!(serde_json::to_value(&rest).unwrap(), preflight);

    let events = ops.activity("mcp-project", 0, None, 200, &maya).await.unwrap().events;
    assert_eq!(events[0].actor, "maya");
    let count = |kind: &str| events.iter().filter(|event| event.event_type == kind).count();
    assert_eq!(count("project.created"), 1);
    assert_eq!(count("guidance.received"), 2);
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn mcp_resources_expose_read_only_project_knowledge() {
    let server = server().await;
    let client = server.client(Some("maya")).await;
    structured(
        &client,
        "limina_create_project",
        json!({
            "slug": "reviewable",
            "name": "Reviewable",
            "mission": "Make durable work reviewable.",
            "success_criteria": "The MCP resource returns the project review.",
            "actor": "reviewer",
        }),
    )
    .await;
    let review = read(&client, "limina://projects/reviewable/review").await.unwrap();
    let ResourceContents::TextResourceContents { mime_type, text, .. } = &review.contents[0] else {
        panic!("not text: {review:?}");
    };
    assert_eq!(mime_type.as_deref(), Some("application/json"));
    assert!(text.contains("\"mission\": \"Make durable work reviewable.\""), "{text}");

    let status =
        structured(&client, "limina_get_project_status", json!({"project": "reviewable"})).await;
    assert_eq!(status["project"]["slug"], "reviewable");
    assert_eq!(status["knowledge"], json!({}));
    // `actor` named the teammate the static-token caller acted for.
    let events = server.test.ops().activity("reviewable", 0, None, 10, &local("maya")).await;
    assert_eq!(events.unwrap().events[0].actor, "reviewer");
    client.cancel().await.unwrap();
    server.close().await;
}
