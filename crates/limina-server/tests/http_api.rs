//! `tests/test_cloud_api.py` over HTTP: probes and the public surface, authentication rate
//! limits, the admin token, project lifecycle and resources, runtime selection, live steering
//! over the WebSocket, the internal capability API, activity cursors, error shapes and review.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use std::sync::Arc;

use app_support::*;
use axum::http::Method;
use http_support::*;
use serde_json::{Value, json};

const SECRET: &[(&str, &str)] = &[("authorization", "Bearer secret")];
const ADMIN: &[(&str, &str)] = &[("authorization", "Bearer admin-secret")];

/// The Python `setUp`: tokens `secret` and `admin-secret`, a session that blocks until steered.
async fn api() -> (Api, Arc<BlockingSession>) {
    let session = BlockingSession::steerable();
    let api = Api::new(runtime(session.clone()).await, local_tokens()).await;
    (api, session)
}

fn commands(key: Option<&str>) -> Vec<(String, String)> {
    command("secret", key, "api-user")
}

fn cloud_project(slug: &str) -> Value {
    json!({
        "slug": slug,
        "name": "Cloud test",
        "objective": "Prove that Limina owns execution.",
        "context": "API integration test.",
        "success_criteria": "A teammate can steer the managed live turn.",
        "runtime": "codex",
    })
}

async fn create_cloud_project(api: &Api) {
    api.post("/v1/projects", &h(&commands(None)), &cloud_project("cloud-test")).await.expect(201);
}

#[tokio::test]
async fn authentication_and_public_surface() {
    let (api, _) = api().await;
    api.get("/livez", &[]).await.expect(200);
    api.get("/readyz", &[]).await.expect(200);
    api.get("/healthz", &[]).await.expect(401);
    let health = api.get("/healthz", SECRET).await.expect(200).json();
    assert_eq!(health["runtime_owner"], "limina");
    assert_eq!(health["runtimes"], json!(["codex", "claude-code"]));
    assert_eq!(health["auth_mode"], "local");
    assert_eq!(health["interfaces"], json!({"rest": "/v1", "mcp": "/mcp/"}));

    let schema = api.get("/openapi.json", SECRET).await.expect(200).json();
    assert!(schema["components"]["securitySchemes"].get("HTTPBearer").is_some());
    let paths: Vec<&String> = schema["paths"].as_object().unwrap().keys().collect();
    let paths = paths.iter().map(|path| path.as_str()).collect::<Vec<_>>().join(" ");
    for forbidden in ["worker", "thread", "session", "coordinator", "lease"] {
        assert!(!paths.contains(forbidden), "{forbidden}");
    }
    api.close().await;
}

#[tokio::test]
async fn failed_authentication_is_rate_limited() {
    let (api, _) = api().await;
    let invalid = [("authorization", "Bearer wrong")];
    for _ in 0..10 {
        api.get("/healthz", &invalid).await.expect(401);
    }
    let limited = api.get("/healthz", &invalid).await;
    let body = assert_error(&limited, 429, "rate_limited");
    assert!(body["error"]["details"]["retry_after_seconds"].as_u64().unwrap() >= 1);
    let retry_after: u64 = limited.header("retry-after").unwrap().parse().unwrap();
    assert!(retry_after >= 1);
    // Another client address is not limited.
    let mut other =
        Api { app: api.app.clone(), test: api.test, peer: "127.0.0.2:50000".parse().unwrap() };
    other.get("/healthz", &invalid).await.expect(401);
    other.peer = "127.0.0.1:50000".parse().unwrap();
    other.get("/healthz", SECRET).await.expect(429);
    other.close().await;
}

#[tokio::test]
async fn runtime_authentication_requires_the_distinct_admin_token() {
    let (api, _) = api().await;
    let path = "/v1/runtime/engines/codex/auth";
    assert_error(&api.get(path, SECRET).await, 403, "permission_denied");
    let status = api.get(path, ADMIN).await.expect(200).json();
    assert_eq!(status["configured"], false);
    assert_eq!(status["engine"], "codex");
    api.close().await;
}

#[tokio::test]
async fn project_lifecycle_resources_and_sanitized_status() {
    let (api, session) = api().await;
    let replay_key = key();
    let payload = json!({
        "slug": "cloud-test",
        "name": "Cloud test",
        "objective": "Prove the shared transport.",
        "context": "API integration test.",
        "success_criteria": "Round-trip state through HTTP.",
    });
    let first = api.post("/v1/projects", &h(&commands(Some(&replay_key))), &payload).await;
    let second = api.post("/v1/projects", &h(&commands(Some(&replay_key))), &payload).await;
    first.expect(201);
    assert_eq!(first.json(), second.json());
    assert_eq!(first.json()["status"], "CREATED");
    assert_eq!(first.json()["runtime"], "codex");
    let created_events =
        api.get("/v1/projects/cloud-test/events", SECRET).await.json()["events"].clone();
    assert_eq!(created_events[0]["detail"]["runtime"], "codex");
    assert!(created_events[0]["detail"].get("runtime_engine").is_none());

    let variable = api
        .put(
            "/v1/projects/cloud-test/resources/variables/SOURCE_URL",
            &h(&commands(None)),
            &json!({"value": "https://example.invalid/repository"}),
        )
        .await;
    variable.expect(200);
    assert_eq!(variable.json()["type"], "VARIABLE");
    assert_eq!(variable.json()["value"], "https://example.invalid/repository");
    assert!(variable.json().get("configured").is_none());

    let secret_value = "super-sensitive-token-123";
    let secret = api
        .put(
            "/v1/projects/cloud-test/resources/secrets/SOURCE_TOKEN",
            &h(&commands(None)),
            &json!({"value": secret_value}),
        )
        .await;
    secret.expect(200);
    assert_eq!(secret.json()["type"], "SECRET");
    assert_eq!(secret.json()["configured"], true);
    assert!(secret.json().get("value").is_none());
    assert!(!secret.text().contains(secret_value));

    let resources = api.get("/v1/projects/cloud-test/resources", SECRET).await;
    resources.expect(200);
    assert!(!resources.text().contains(secret_value));
    let review = api.get("/v1/projects/cloud-test/review", SECRET).await;
    let events = api.get("/v1/projects/cloud-test/events", SECRET).await;
    assert!(!review.text().contains(secret_value));
    assert!(!events.text().contains(secret_value));

    api.call(Method::POST, "/v1/projects/cloud-test/actions/start", &h(&commands(None)))
        .await
        .expect(200);
    session.started(1).await;

    let status = api.get("/v1/projects/cloud-test/status", SECRET).await;
    status.expect(200);
    let serialized = status.text().to_lowercase();
    for forbidden in ["thread_id", "continuation_id", "worker_id", "inbox_cursor", "version"] {
        assert!(!serialized.contains(forbidden), "{forbidden}");
    }
    api.close().await;
}

#[tokio::test]
async fn project_creation_selects_a_supported_runtime_engine() {
    let (api, _) = api().await;
    let mut payload = json!({
        "slug": "claude-project",
        "name": "Claude project",
        "objective": "Use the Claude Code engine.",
        "context": "API selection test.",
        "success_criteria": "The engine is persisted.",
        "runtime": "claude-code",
    });
    let created = api.post("/v1/projects", &h(&commands(None)), &payload).await;
    created.expect(201);
    assert_eq!(created.json()["runtime"], "claude-code");

    payload["slug"] = json!("invalid-project");
    payload["runtime"] = json!("other");
    let rejected = api.post("/v1/projects", &h(&commands(None)), &payload).await;
    let body = assert_error(&rejected, 422, "invalid_request");
    assert_eq!(body["error"]["details"]["errors"][0]["loc"], json!(["body", "runtime"]));
    api.close().await;
}

#[tokio::test]
async fn live_attach_can_steer_the_active_managed_turn() {
    let (api, session) = api().await;
    create_cloud_project(&api).await;
    api.call(Method::POST, "/v1/projects/cloud-test/actions/start", &h(&commands(None)))
        .await
        .expect(200);
    session.started(1).await;

    let server = api.listen().await;
    let url = server.ws_url("/v1/projects/cloud-test/live");
    let headers = [("authorization", "Bearer secret"), ("x-limina-actor", "reviewer")];
    let (mut socket, protocol) = connect(&url, &["limina.v1"], &headers).await.unwrap();
    assert_eq!(protocol.as_deref(), Some("limina.v1"));
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    send_json(&mut socket, &json!({"type": "steer", "body": "Prioritize generalization."})).await;
    let delivery = next_non_event(&mut socket).await;
    assert_eq!(delivery["type"], "delivery");
    assert_eq!(delivery["value"], "LIVE");
    assert_eq!(delivery["receipt"]["delivery"], "LIVE");
    drop(socket);
    assert_eq!(*session.steering.lock().unwrap(), ["reviewer: Prioritize generalization."]);

    let response = api
        .post(
            "/v1/projects/cloud-test/steering",
            &h(&commands(None)),
            &json!({"body": "Now test the strongest baseline.", "kind": "STEER"}),
        )
        .await;
    response.expect(202);
    let mut fields = keys(&response.json());
    fields.sort();
    assert_eq!(fields, ["accepted_at", "delivery", "id", "kind", "status"]);
    api.close().await;
}

#[tokio::test]
async fn private_agent_commands_require_a_project_scoped_capability() {
    let (api, _) = api().await;
    create_cloud_project(&api).await;
    let capability = api.test.runtime.supervisor().issue_capability("cloud-test").unwrap();
    let authorization = bearer(&capability);
    let command_id = key();
    let headers =
        [("authorization", authorization.as_str()), ("idempotency-key", command_id.as_str())];
    let created = api
        .post(
            "/internal/v1/projects/cloud-test/hypotheses",
            &headers,
            &json!({"title": "Scoped write", "statement": "The capability permits only its managed project."}),
        )
        .await;
    created.expect(201);
    assert_eq!(created.json()["id"], "H001");
    // A replay with the same key returns the same artifact.
    let replayed = api
        .post(
            "/internal/v1/projects/cloud-test/hypotheses",
            &headers,
            &json!({"title": "Scoped write", "statement": "The capability permits only its managed project."}),
        )
        .await;
    assert_eq!(replayed.json(), created.json());

    let denied = api
        .get("/internal/v1/projects/cloud-test/status", &[("authorization", "Bearer wrong")])
        .await;
    assert_error(&denied, 401, "authentication_required");
    // The capability is scoped to its project.
    api.post("/v1/projects", &h(&commands(None)), &cloud_project("other-project"))
        .await
        .expect(201);
    let other = api.get("/internal/v1/projects/other-project/status", &headers[..1]).await;
    assert_error(&other, 401, "authentication_required");
    let status =
        api.get("/internal/v1/projects/cloud-test/status", &headers[..1]).await.expect(200).json();
    assert_eq!(status["counts"]["H"]["PROPOSED"], 1);
    api.close().await;
}

#[tokio::test]
async fn activity_cursor_replays_only_new_durable_events() {
    let (api, _) = api().await;
    create_cloud_project(&api).await;
    let first = api.get("/v1/projects/cloud-test/events?after=0", SECRET).await.json();
    let types = |page: &Value| -> Vec<Value> {
        page["events"].as_array().unwrap().iter().map(|event| event["type"].clone()).collect()
    };
    assert_eq!(types(&first), [json!("project.created")]);

    api.put(
        "/v1/projects/cloud-test/resources/variables/BRIEF_URI",
        &h(&commands(None)),
        &json!({"value": "s3://brief"}),
    )
    .await
    .expect(200);
    let second = api
        .get(&format!("/v1/projects/cloud-test/events?after={}", first["cursor"]), SECRET)
        .await
        .json();
    assert_eq!(types(&second), [json!("resource.variable_set"), json!("guidance.received")]);
    assert!(second["cursor"].as_i64().unwrap() > first["cursor"].as_i64().unwrap());
    api.close().await;
}

#[tokio::test]
async fn domain_errors_have_stable_machine_readable_shape() {
    let (api, _) = api().await;
    let response = api.get("/v1/projects/missing/status", SECRET).await;
    let body = assert_error(&response, 404, "not_found");
    assert!(body["error"]["message"].as_str().unwrap().contains("does not exist"));
    assert_eq!(keys(&body["error"]), ["code", "message", "details"]);
    api.close().await;
}

#[tokio::test]
async fn review_recent_activity_uses_the_newest_events_after_large_histories() {
    let (api, _) = api().await;
    create_cloud_project(&api).await;
    let rows: Vec<(&str, &str, Value)> =
        (0..1005).map(|marker| ("test.marker", "tester", json!({"marker": marker}))).collect();
    insert_events(&api.test, "cloud-test", &rows).await;
    let review = api.get("/v1/projects/cloud-test/review", SECRET).await;
    review.expect(200);
    let recent = review.json()["recent_activity"].clone();
    assert_eq!(recent.as_array().unwrap().len(), 50);
    assert_eq!(recent[49]["detail"]["marker"], 1004);
    api.close().await;
}
