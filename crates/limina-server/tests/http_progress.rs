#![allow(clippy::unwrap_used)]
mod app_support;
mod http_support;
use app_support::*;
use http_support::*;
use limina_domain::research::NewHypothesis;
use serde_json::json;
const OWNER: &[(&str, &str)] =
    &[("authorization", "Bearer owner-token"), ("idempotency-key", "kpi-create")];
const EDITOR: &[(&str, &str)] =
    &[("authorization", "Bearer editor-token"), ("idempotency-key", "kpi-editor")];
const VIEWER: &[(&str, &str)] =
    &[("authorization", "Bearer viewer-token"), ("idempotency-key", "kpi-viewer")];
#[tokio::test]
async fn metrics_require_membership_and_editor_access_and_preserve_missing_measurements() {
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, ui_tokens()).await;
    create_project(api.test.ops(), "tracked").await;
    add_member(api.test.ops(), "tracked", &editor(), "EDITOR").await;
    add_member(api.test.ops(), "tracked", &viewer(), "VIEWER").await;
    let path = "/v1/projects/tracked/metrics/precision";
    let input = json!({"metric":{"name":"Precision","unit":"%","direction":"at_least","target":95,"baseline":70},"expected_version":0});
    api.get("/v1/projects/tracked/metrics", &[]).await.expect(401);
    api.get("/v1/projects/tracked/metrics", &[("authorization", "Bearer admin-token")])
        .await
        .expect(403);
    api.put(path, VIEWER, &input).await.expect(403);
    let created = api.put(path, EDITOR, &input).await.expect(200).json();
    assert_eq!(created["current"], json!(null));
    assert_eq!(created["observed_at"], json!(null));
    assert_eq!(created["version"], 1);
    assert_eq!(created, api.put(path, EDITOR, &input).await.expect(200).json());
    let all = api.get("/v1/projects/tracked/metrics", VIEWER).await.expect(200).json();
    assert_eq!(all, json!([created]));
    let measured = json!({"metric":{"name":"Precision","unit":"%","direction":"at_least","target":95,"current":90},"expected_version":1});
    api.put(path, OWNER, &measured).await.expect(422);
    let stale = json!({"metric":{"name":"Precision","direction":"at_least","target":100},"expected_version":0});
    api.put(path, OWNER, &stale).await.expect(409);
    api.close().await;
}

#[tokio::test]
async fn focused_document_and_identifier_search_share_project_authorization() {
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, ui_tokens()).await;
    create_project(api.test.ops(), "notes").await;
    let note = api
        .test
        .runtime
        .service()
        .create_hypothesis(
            "notes",
            NewHypothesis {
                title: "A long title that remains readable".into(),
                statement: "The full note body includes a conclusive final sentence.".into(),
                ..Default::default()
            },
            "researcher",
            &key(),
        )
        .await
        .unwrap();
    let path = format!("/v1/projects/notes/knowledge/{}/document", note.id);
    api.get(&path, &[("authorization", "Bearer admin-token")]).await.expect(403);
    let focused = api.get(&path, OWNER).await.expect(200).json();
    let files = focused["files"].as_object().unwrap();
    assert_eq!(files.len(), 1);
    assert!(files.values().next().unwrap().as_str().unwrap().contains("conclusive final sentence"));
    let matches =
        api.get("/v1/projects/notes/knowledge?query=h001&limit=1", OWNER).await.expect(200).json();
    assert_eq!(matches["total"], 1);
    assert_eq!(matches["items"][0]["id"], note.id);
    let children = api
        .get("/v1/projects/notes/knowledge?parent=H001&limit=25", OWNER)
        .await
        .expect(200)
        .json();
    assert_eq!(children["total"], 0);
    let mission =
        api.get("/v1/projects/notes/knowledge/CHALLENGE/document", OWNER).await.expect(200).json();
    assert_eq!(mission["files"].as_object().unwrap().len(), 1);
    api.close().await;
}
