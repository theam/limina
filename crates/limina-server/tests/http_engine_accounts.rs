//! `tests/test_cloud_engine_accounts.py` over HTTP: only owners connect accounts and tokens are
//! never returned, a project token replaces the runtime's credentials and is redacted, bad tokens
//! are refused, and a Codex sign-in belongs to its project (including `DELETE` of the static
//! `claude-code` path, which FastAPI routed to the `{engine}` route).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use app_support::*;
use async_trait::async_trait;
use axum::http::Method;
use http_support::*;
use limina_core::Result;
use limina_engines::{AgentSession, RuntimeEvent, RuntimeTurn, TurnRequest};
use serde_json::{Value, json};

const TOKEN: &str = "sk-ant-oat01-project-subscription-token-value";

/// Records the environment of each turn and echoes the credential to test redaction.
struct CapturingAgent {
    turns: Mutex<Vec<BTreeMap<String, String>>>,
}

#[async_trait]
impl AgentSession for CapturingAgent {
    async fn run_turn(&self, request: TurnRequest) -> Result<RuntimeTurn> {
        let env = request.runtime_env.clone();
        let token = env.get("CLAUDE_CODE_OAUTH_TOKEN").cloned().unwrap_or_default();
        self.turns.lock().unwrap().push(env);
        request
            .observer
            .event(RuntimeEvent::new(
                "runtime.claude-code",
                "echo",
                detail(json!({"kind": "message", "text": format!("token is {token}")})),
            ))
            .await?;
        tokio::task::yield_now().await;
        Ok(turn(None, "Done.", "WAITING", "O", "N", "None"))
    }

    async fn steer(&self, _message: &str) -> Result<bool> {
        Ok(false)
    }

    async fn interrupt(&self) -> Result<bool> {
        Ok(true)
    }

    async fn close(&self) {}

    fn model(&self) -> Option<String> {
        Some("capture".into())
    }
}

/// The Python `setUp`: tokens `owner`, `editor` and `viewer`, a Claude project with an editor
/// and a viewer, and a shared runtime Claude key that a project account must override.
async fn accounts() -> (Api, Arc<CapturingAgent>) {
    let agent = Arc::new(CapturingAgent { turns: Mutex::new(Vec::new()) });
    let test = runtime(agent.clone()).await;
    test.set_env("ANTHROPIC_API_KEY", "runtime-shared-key");
    test.set_env("CLAUDE_CODE_OAUTH_TOKEN", "");
    let principals = HashMap::from([
        ("owner".to_owned(), owner()),
        ("editor".to_owned(), editor()),
        ("viewer".to_owned(), viewer()),
    ]);
    let api = Api::new(test, token_auth(principals)).await;
    let project = json!({
        "slug": "accounts",
        "name": "Accounts",
        "objective": "Run on my own account.",
        "success_criteria": "Turns use the project's account.",
        "runtime": "claude-code",
    });
    api.post("/v1/projects", &h(&command("owner", None, "x")), &project).await.expect(201);
    for (subject, role) in [("editor", "EDITOR"), ("viewer", "VIEWER")] {
        let member = json!({"subject": subject, "display_name": subject, "role": role});
        api.put("/v1/projects/accounts/members", &h(&auth("owner")), &member).await.expect(200);
    }
    (api, agent)
}

async fn listing(api: &Api) -> BTreeMap<String, Value> {
    let response = api.get("/v1/projects/accounts/engine-accounts", &h(&auth("viewer"))).await;
    response
        .expect(200)
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| (item["engine"].as_str().unwrap().to_owned(), item.clone()))
        .collect()
}

#[tokio::test]
async fn only_owners_connect_and_tokens_are_never_returned() {
    let (api, _) = accounts().await;
    let before = listing(&api).await;
    assert!(before["claude-code"]["account"].is_null());
    assert_eq!(before["claude-code"]["label"], "Claude");
    let path = "/v1/projects/accounts/engine-accounts/claude-code";
    for token in ["viewer", "editor"] {
        assert_error(
            &api.put(path, &h(&auth(token)), &json!({"token": TOKEN})).await,
            403,
            "permission_denied",
        );
    }
    let connected = api.put(path, &h(&auth("owner")), &json!({"token": TOKEN})).await;
    connected.expect(200);
    let account = connected.json()["account"].clone();
    assert_eq!(
        (&account["method"], &account["label"], &account["connected_by"]),
        (
            &json!("subscription"),
            &json!(format!("…{}", &TOKEN[TOKEN.len() - 4..])),
            &json!("Olivia Owner")
        )
    );
    let listing =
        api.get("/v1/projects/accounts/engine-accounts", &h(&auth("viewer"))).await.text();
    assert!(!(connected.text() + &listing).contains(TOKEN));
    let preflight = api.get("/v1/projects/accounts/preflight", &h(&auth("viewer"))).await.json();
    let runtime = preflight["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "runtime")
        .unwrap()
        .clone();
    assert!(runtime["detail"].as_str().unwrap().contains("project's own account"));
    api.close().await;
}

#[tokio::test]
async fn project_token_replaces_runtime_credentials_and_is_redacted() {
    let (api, agent) = accounts().await;
    let path = "/v1/projects/accounts/engine-accounts/claude-code";
    api.put(path, &h(&auth("owner")), &json!({"token": TOKEN})).await.expect(200);
    api.call(Method::POST, "/v1/projects/accounts/actions/start", &h(&command("owner", None, "x")))
        .await
        .expect(200);
    wait_until("the project to run a turn", || async { !agent.turns.lock().unwrap().is_empty() })
        .await;
    let env = agent.turns.lock().unwrap()[0].clone();
    assert_eq!(env["CLAUDE_CODE_OAUTH_TOKEN"], TOKEN);
    assert_eq!(env["ANTHROPIC_API_KEY"], "");
    wait_until("the turn to checkpoint", || async {
        api.get("/v1/projects/accounts/status", &h(&auth("owner"))).await.json()["project"]["status"] == "WAITING"
    })
    .await;
    let events = api.get("/v1/projects/accounts/events", &h(&auth("owner"))).await.text();
    assert!(!events.contains(TOKEN));
    assert!(events.contains("[REDACTED]"));
    api.close().await;
}

#[tokio::test]
async fn api_keys_are_recognised_and_bad_tokens_rejected() {
    let (api, _) = accounts().await;
    let path = "/v1/projects/accounts/engine-accounts/claude-code";
    let api_key = format!("sk-ant-api03-{}", "k".repeat(30));
    let connected = api.put(path, &h(&auth("owner")), &json!({"token": api_key})).await;
    assert_eq!(connected.expect(200).json()["account"]["method"], "api-key");
    let accounts = api.test.runtime.supervisor().engine_accounts();
    let overlay = accounts.claude_environment("accounts").await.unwrap();
    assert!(overlay.environment["ANTHROPIC_API_KEY"].starts_with("sk-ant-api03-"));
    assert_eq!(overlay.environment["CLAUDE_CODE_OAUTH_TOKEN"], "");
    assert_eq!(overlay.secrets.len(), 1);

    let rejected = api
        .put(path, &h(&auth("owner")), &json!({"token": "has spaces in it which is not a token"}))
        .await;
    let body = assert_error(&rejected, 422, "invariant_violation");
    assert_eq!(body["error"]["message"], "That doesn't look like a Claude token.");
    let short = api.put(path, &h(&auth("owner")), &json!({"token": "short"})).await;
    let body = assert_error(&short, 422, "invalid_request");
    assert_eq!(body["error"]["details"]["errors"][0]["type"], "too_short");
    // Disconnecting through the static path reaches the `{engine}` route.
    let removed = api.delete(path, &h(&auth("owner"))).await;
    assert!(removed.expect(200).json()["account"].is_null());
    api.close().await;
}

#[tokio::test]
async fn codex_sign_in_is_scoped_to_the_project() {
    let (api, _) = accounts().await;
    let supervisor = api.test.runtime.supervisor();
    assert!(supervisor.codex_auth_for("accounts").await.unwrap().same_as(supervisor.codex_auth()));
    let started = api
        .call(
            Method::POST,
            "/v1/projects/accounts/engine-accounts/codex/login",
            &h(&command("owner", None, "x")),
        )
        .await;
    started.expect(200);
    assert_eq!(started.json()["user_code"], "WXYZ-1234");
    let mut status = json!("PENDING");
    for _ in 0..500 {
        status = api
            .get("/v1/projects/accounts/engine-accounts/codex/login/login-1", &h(&auth("owner")))
            .await
            .json()["status"]
            .clone();
        if status != "PENDING" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(status, "SUCCEEDED");
    let account = listing(&api).await["codex"]["account"].clone();
    assert_eq!(account["label"], "olivia@example.test · pro");
    let project_manager = supervisor.codex_auth_for("accounts").await.unwrap();
    assert!(!project_manager.same_as(supervisor.codex_auth()));
    assert_eq!(project_manager.home().file_name().unwrap(), "accounts");

    let removed =
        api.delete("/v1/projects/accounts/engine-accounts/codex", &h(&auth("owner"))).await;
    assert!(removed.expect(200).json()["account"].is_null());
    assert!(!project_manager.home().join("auth.json").exists());
    assert!(supervisor.codex_auth_for("accounts").await.unwrap().same_as(supervisor.codex_auth()));
    let unknown =
        api.delete("/v1/projects/accounts/engine-accounts/gemini", &h(&auth("owner"))).await;
    assert_error(&unknown, 422, "invariant_violation");
    api.close().await;
}
