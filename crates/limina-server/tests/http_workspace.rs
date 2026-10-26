#![allow(clippy::unwrap_used)]
mod app_support;
mod http_support;
use app_support::*;
use http_support::*;
use serde_json::json;
const OWNER: &[(&str, &str)] = &[("authorization", "Bearer owner-token")];
const VIEWER: &[(&str, &str)] = &[("authorization", "Bearer viewer-token")];

#[tokio::test]
async fn assessment_is_durable_independent_of_history_and_requires_membership() {
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, ui_tokens()).await;
    create_project(api.test.ops(), "workspace").await;
    add_member(api.test.ops(), "workspace", &viewer(), "VIEWER").await;
    let path = "/v1/projects/workspace/workspace";
    assert!(api.get(path, OWNER).await.expect(200).json().is_null());
    let event = api.test.runtime.service().record_runtime_event("workspace", "runtime.turn_completed", json!({
        "summary": "No measurement yet", "mission_update": {"headline": "Evaluation is still untrusted", "evidence": []}
    }).as_object().unwrap().clone(), "agent", &key()).await.unwrap();
    for _ in 0..35 {
        api.test
            .runtime
            .service()
            .record_runtime_event(
                "workspace",
                "runtime.codex",
                json!({"summary":"tool"}).as_object().unwrap().clone(),
                "agent",
                &key(),
            )
            .await
            .unwrap();
    }
    api.get(path, &[]).await.expect(401);
    api.get(path, &[("authorization", "Bearer admin-token")]).await.expect(403);
    let read = api.get(path, VIEWER).await.expect(200).json();
    assert_eq!(read["sequence"], event.sequence);
    assert_eq!(read["mission_update"]["headline"], "Evaluation is still untrusted");
    assert!(
        api.get("/v1/projects/workspace/metrics", VIEWER)
            .await
            .expect(200)
            .json()
            .as_array()
            .unwrap()
            .is_empty()
    );
    api.close().await;
}

#[tokio::test]
async fn published_files_are_scoped_bounded_and_downloaded_without_execution() {
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, ui_tokens()).await;
    create_project(api.test.ops(), "files").await;
    add_member(api.test.ops(), "files", &viewer(), "VIEWER").await;
    let dir = api.test.root.join("workspaces/files/outputs");
    tokio::fs::create_dir_all(&dir).await.unwrap();
    for i in 0..28 {
        tokio::fs::write(dir.join(format!("report-{i:02}.md")), "# Published\n\nEvidence [[F001]]")
            .await
            .unwrap();
    }
    tokio::fs::write(dir.join(".env"), "hidden").await.unwrap();
    tokio::fs::write(dir.parent().unwrap().join("secret.txt"), "not published").await.unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../secret.txt", dir.join("linked.txt")).unwrap();
    let prefix = "/v1/projects/files/outputs";
    api.get(prefix, &[]).await.expect(401);
    api.get(prefix, &[("authorization", "Bearer admin-token")]).await.expect(403);
    let page = api.get(prefix, VIEWER).await.expect(200).json();
    assert_eq!(page["items"].as_array().unwrap().len(), 25);
    assert_eq!(page["next_offset"], 25);
    assert_eq!(
        api.get(&format!("{prefix}?offset=25"), OWNER).await.expect(200).json()["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    api.get(&format!("{prefix}/.env"), OWNER).await.expect(404);
    api.get(&format!("{prefix}/linked.txt"), OWNER).await.expect(404);
    let preview = api.get(&format!("{prefix}/report-00.md"), VIEWER).await.expect(200).json();
    assert_eq!(preview["text"], "# Published\n\nEvidence [[F001]]");
    let downloaded = api.get(&format!("{prefix}/report-00.md/download"), VIEWER).await;
    let download = downloaded.expect(200);
    assert_eq!(download.headers["content-type"], "application/octet-stream");
    assert_eq!(download.headers["content-disposition"], "attachment");
    assert_eq!(download.body.as_ref(), b"# Published\n\nEvidence [[F001]]");
    tokio::fs::write(dir.join("large.txt"), "é".repeat(70000)).await.unwrap();
    let preview = api.get(&format!("{prefix}/large.txt"), OWNER).await.expect(200).json();
    assert_eq!(preview["truncated"], true);
    assert_eq!(preview["text"].as_str().unwrap().len(), 128 * 1024);
    api.close().await;
}

#[tokio::test]
async fn historical_pages_are_bounded_and_do_not_change_the_current_note() {
    use limina_domain::research::{HypothesisDecision, NewHypothesis};
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, ui_tokens()).await;
    create_project(api.test.ops(), "history").await;
    let service = api.test.runtime.service();
    let mut note = service
        .create_hypothesis(
            "history",
            NewHypothesis {
                title: "A revisable claim".into(),
                statement: "Original claim".into(),
                ..Default::default()
            },
            "agent",
            &key(),
        )
        .await
        .unwrap();
    for version in 1..32 {
        note = service
            .decide_hypothesis(
                "history",
                &note.id,
                HypothesisDecision {
                    status: "TESTING".into(),
                    conclusion: format!("Evidence assessment {version}"),
                    expected_version: note.version,
                },
                "agent",
                &key(),
            )
            .await
            .unwrap();
    }
    let path = "/v1/projects/history/knowledge/H001/history";
    let newest = api.get(path, OWNER).await.expect(200).json();
    assert_eq!(newest["items"].as_array().unwrap().len(), 25);
    let before = newest["next_before"].as_i64().unwrap();
    let older = api.get(&format!("{path}?before={before}"), OWNER).await.expect(200).json();
    assert_eq!(older["items"].as_array().unwrap().len(), 7);
    assert!(older["next_before"].is_null());
    assert_eq!(
        older["items"].as_array().unwrap().last().unwrap()["content"]["statement"],
        "Original claim"
    );
    assert_eq!(
        service.get_artifact("history", "H001").await.unwrap().artifact.version,
        note.version
    );
    api.close().await;
}

#[tokio::test]
async fn approving_a_recommendation_records_guidance_without_inventing_measurements() {
    let reply = turn(
        None,
        "Awaiting a mission decision",
        "WAITING",
        "Validate",
        "Keep the guardrail",
        "Choose the budget",
    );
    let api = Api::new(runtime(ScriptedSession::always(Reply::ok(reply))).await, ui_tokens()).await;
    create_project(api.test.ops(), "decision").await;
    let headers = &[("authorization", "Bearer owner-token"), ("idempotency-key", "define-target")];
    api.put(
        "/v1/projects/decision/metrics/coverage",
        headers,
        &json!({
            "metric":{"name":"Coverage", "direction":"at_least", "target":90}, "expected_version":0
        }),
    )
    .await
    .expect(200);
    api.post(
        "/v1/projects/decision/actions/start",
        &[("authorization", "Bearer owner-token"), ("idempotency-key", "start")],
        &json!({}),
    )
    .await
    .expect(200);
    wait_until("initial checkpoint", || async {
        api.test
            .runtime
            .service()
            .latest_event("decision", "runtime.turn_completed")
            .await
            .unwrap()
            .is_some()
    })
    .await;
    let before = api.get("/v1/projects/decision/metrics", OWNER).await.expect(200).json();
    let receipt = api.post("/v1/projects/decision/steering", &[("authorization", "Bearer owner-token"), ("idempotency-key", "approve")],
        &json!({"kind":"APPROVAL", "body":"Keep the agreed coverage guardrail. This is direction, not a measurement."})).await.expect(202).json();
    assert_eq!(receipt["kind"], "APPROVAL");
    let after = api.get("/v1/projects/decision/metrics", OWNER).await.expect(200).json();
    assert_eq!(before, after);
    assert!(after[0]["current"].is_null());
    assert!(after[0]["evidence_id"].is_null());
    api.close().await;
}
