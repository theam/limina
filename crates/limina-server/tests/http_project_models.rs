//! Project defaults travel from onboarding/kickoff through persistence into real turn requests.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod app_support;
mod http_support;
use app_support::*;
use http_support::*;
use serde_json::json;

const OWNER: &[(&str, &str)] =
    &[("authorization", "Bearer owner-token"), ("idempotency-key", "model-test-create")];

#[tokio::test]
async fn onboarding_and_kickoff_defaults_reach_each_engine_and_future_turns() {
    for engine in ["codex", "claude-code"] {
        let session = ScriptedSession::always(Reply::ok(turn(
            Some("continued"),
            "Waiting",
            "WAITING",
            "Mission",
            "Ask",
            "Need input",
        )));
        let api = Api::new(runtime(session.clone()).await, ui_tokens()).await;
        let created = api.post("/v1/projects", OWNER, &json!({"slug":"defaults", "name":"Defaults", "objective":"Mission", "success_criteria":"Success", "runtime":engine, "model":"onboarding-model", "reasoning_effort":"high"})).await.expect(201).json();
        assert_eq!(created["model"], "onboarding-model");
        assert_eq!(created["reasoning_effort"], "high");
        add_member(api.test.ops(), "defaults", &editor(), "EDITOR").await;
        api.patch(
            "/v1/projects/defaults",
            &[("authorization", "Bearer editor-token")],
            &json!({"model":"forbidden"}),
        )
        .await
        .expect(403);
        let kickoff = api
            .patch(
                "/v1/projects/defaults",
                OWNER,
                &json!({"model":"kickoff-model", "reasoning_effort":"max"}),
            )
            .await
            .expect(200)
            .json();
        assert_eq!(kickoff["model"], "kickoff-model");
        let reloaded = api.get("/v1/projects/defaults", OWNER).await.expect(200).json();
        assert_eq!(reloaded["reasoning_effort"], "max");
        api.test.ops().apply_lifecycle("defaults", "start", &key(), &owner()).await.unwrap();
        wait_until("first turn", || async {
            api.test.ops().get_project("defaults", &owner()).await.unwrap().status == "WAITING"
        })
        .await;
        assert_eq!(session.turns()[0].settings.model.as_deref(), Some("kickoff-model"));
        assert_eq!(session.turns()[0].settings.reasoning_effort.as_deref(), Some("max"));
        api.test.ops().apply_lifecycle("defaults", "pause", &key(), &owner()).await.unwrap();
        api.patch(
            "/v1/projects/defaults",
            OWNER,
            &json!({"model":"next-model", "reasoning_effort":"low"}),
        )
        .await
        .expect(200);
        api.test.ops().apply_lifecycle("defaults", "resume", &key(), &owner()).await.unwrap();
        wait_until("second turn", || async {
            session.turns().len() == 2
                && api.test.ops().get_project("defaults", &owner()).await.unwrap().status
                    == "WAITING"
        })
        .await;
        assert_eq!(session.turns()[1].settings.model.as_deref(), Some("next-model"));
        assert_eq!(session.turns()[1].settings.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(session.turns()[1].continuation_id.as_deref(), Some("continued"));
        let runs = api.get("/v1/projects/defaults/runs", OWNER).await.expect(200).json();
        assert_eq!(runs["items"][0]["model"], "next-model");
        api.close().await;
    }
}

#[tokio::test]
async fn invalid_provider_choices_are_rejected_and_catalog_is_authenticated() {
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, ui_tokens()).await;
    api.get("/v1/runtime/engines/claude-code/models", &[]).await.expect(401);
    let models = api.get("/v1/runtime/engines/claude-code/models", OWNER).await.expect(200).json();
    assert_eq!(models["models"][0]["reasoning_efforts"], json!(["low", "future"]));
    assert!(models.get("reasoning_efforts").is_none());
    api.post("/v1/projects", OWNER, &json!({"slug":"bad", "name":"Bad", "objective":"Mission", "success_criteria":"Success", "runtime":"claude-code", "reasoning_effort":"not a mode"})).await.expect(422);
    api.post("/v1/projects", OWNER, &json!({"slug":"bad", "name":"Bad", "objective":"Mission", "success_criteria":"Success", "model":"--bad"})).await.expect(422);
    api.close().await;
}

#[tokio::test]
async fn editing_defaults_during_a_turn_does_not_interrupt_it_or_change_its_snapshot() {
    let session = BlockingSession::steerable();
    let test = runtime(session.clone()).await;
    let mut project = new_project("live-defaults");
    project.model = Some("first-model".into());
    project.reasoning_effort = Some("high".into());
    test.ops().create_project(project, &key(), &owner()).await.unwrap();
    test.ops().apply_lifecycle("live-defaults", "start", &key(), &owner()).await.unwrap();
    session.started(1).await;
    let changes = limina_domain::collaboration::DraftChanges {
        model: Some("next-model".into()),
        reasoning_effort: Some("low".into()),
        ..Default::default()
    };
    test.ops().update_project("live-defaults", &changes, &owner()).await.unwrap();
    assert_eq!(session.turns()[0].settings.model.as_deref(), Some("first-model"));
    assert_eq!(session.interrupts.load(std::sync::atomic::Ordering::SeqCst), 0);
    session.release(Release::Complete(complete()));
    wait_until("current turn finishes", || async {
        test.ops().get_project("live-defaults", &owner()).await.unwrap().status == "COMPLETE"
    })
    .await;
    assert_eq!(
        test.ops().get_project("live-defaults", &owner()).await.unwrap().model.as_deref(),
        Some("next-model")
    );
    test.close().await;
}

#[tokio::test]
async fn catalog_refresh_is_live_and_project_credentials_require_membership() {
    let session = ScriptedSession::always(Reply::ok(complete()));
    let scopes = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = scopes.clone();
    let agent = session.clone();
    let factory: limina_engines::AgentFactory = std::sync::Arc::new(move |slug, _| {
        seen.lock().unwrap().push(slug.to_owned());
        Ok(agent.clone())
    });
    let api = Api::new(runtime_with(factory, |_| {}).await, ui_tokens()).await;
    create_project(api.test.ops(), "private-models").await;
    api.test
        .ops()
        .connect_claude_account("private-models", "test-private-model-token-12345", &owner())
        .await
        .unwrap();
    let path = "/v1/runtime/engines/claude-code/models?project=private-models";
    api.get(path, &[("authorization", "Bearer admin-token")]).await.expect(403);
    assert!(session.catalog_calls.lock().unwrap().is_empty());
    for _ in 0..2 {
        api.get(path, OWNER).await.expect(200);
    }
    api.get("/v1/runtime/engines/claude-code/models", OWNER).await.expect(200);
    let calls = session.catalog_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3, "refresh must query the provider again");
    assert_eq!(
        calls[0].get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str),
        Some("test-private-model-token-12345")
    );
    assert!(calls[2].is_empty(), "onboarding cannot reuse another project's credentials");
    assert!(session.turns().is_empty());
    assert_eq!(*scopes.lock().unwrap(), ["private-models", "private-models", "__model_catalog"]);
    api.close().await;
}
