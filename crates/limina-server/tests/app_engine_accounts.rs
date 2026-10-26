//! Per-project engine accounts: the four tests of `tests/test_cloud_engine_accounts.py`, at the
//! operations level, with the Python suite's capturing agent and fake Codex.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use app_support::*;
use async_trait::async_trait;
use limina_core::Result;
use limina_domain::Engine;
use limina_engines::{AgentSession, RuntimeEvent, RuntimeTurn, TurnRequest};
use limina_server::app::{NewProject, ProjectEngineAccountResponse};
use serde_json::json;

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

struct Accounts {
    test: TestRuntime,
    agent: Arc<CapturingAgent>,
}

/// The Python `setUp`: a Claude project owned by Olivia with an editor and a viewer, and a
/// shared runtime Claude key that a project account must override.
async fn accounts() -> Accounts {
    let agent = Arc::new(CapturingAgent { turns: Mutex::new(Vec::new()) });
    let test = runtime(agent.clone()).await;
    test.set_env("ANTHROPIC_API_KEY", "runtime-shared-key");
    test.set_env("CLAUDE_CODE_OAUTH_TOKEN", "");
    let project = NewProject {
        slug: "accounts".into(),
        name: "Accounts".into(),
        mission: "Run on my own account.".into(),
        success_criteria: "Turns use the project's account.".into(),
        context: String::new(),
        runtime: Some("claude-code".into()),
        model: None,
        reasoning_effort: None,
    };
    test.ops().create_project(project, &key(), &owner()).await.unwrap();
    add_member(test.ops(), "accounts", &editor(), "EDITOR").await;
    add_member(test.ops(), "accounts", &viewer(), "VIEWER").await;
    Accounts { test, agent }
}

async fn listing(test: &TestRuntime) -> BTreeMap<String, ProjectEngineAccountResponse> {
    test.ops()
        .engine_accounts("accounts", &viewer())
        .await
        .unwrap()
        .into_iter()
        .map(|item| (item.engine.id().to_owned(), item))
        .collect()
}

#[tokio::test]
async fn only_owners_connect_and_tokens_are_never_returned() {
    let Accounts { test, .. } = accounts().await;
    let ops = test.ops();
    let before = listing(&test).await;
    assert_eq!(before["claude-code"].account, None);
    assert_eq!(before["claude-code"].label, "Claude");
    for who in [viewer(), editor()] {
        let denied = ops.connect_claude_account("accounts", TOKEN, &who).await.unwrap_err();
        assert_eq!((denied.http_status(), denied.code()), (403, "permission_denied"));
    }
    let connected = ops.connect_claude_account("accounts", TOKEN, &owner()).await.unwrap();
    let account = connected.account.clone().unwrap();
    assert_eq!(
        (account.method.as_str(), account.label.as_deref(), account.connected_by.as_deref()),
        ("subscription", Some(&*format!("…{}", &TOKEN[TOKEN.len() - 4..])), Some("Olivia Owner"))
    );
    assert!(account.connected_at.is_some());
    let shown = serde_json::to_string(&connected).unwrap()
        + &serde_json::to_string(&ops.engine_accounts("accounts", &viewer()).await.unwrap())
            .unwrap();
    assert!(!shown.contains(TOKEN));
    let preflight = ops.preflight("accounts", &viewer()).await.unwrap();
    let runtime = preflight.checks.iter().find(|check| check.name == "runtime").unwrap();
    assert!(runtime.detail.contains("project's own account"), "{}", runtime.detail);
    assert_eq!(
        runtime.detail,
        "Claude is ready. Runs on the project's own account (…alue), connected by Olivia Owner."
    );
    test.close().await;
}

#[tokio::test]
async fn project_token_replaces_runtime_credentials_and_is_redacted() {
    let Accounts { test, agent } = accounts().await;
    let ops = test.ops();
    ops.connect_claude_account("accounts", TOKEN, &owner()).await.unwrap();
    ops.apply_lifecycle("accounts", "start", &key(), &owner()).await.unwrap();
    wait_until("the project to run a turn", || async { !agent.turns.lock().unwrap().is_empty() })
        .await;
    let env = agent.turns.lock().unwrap()[0].clone();
    assert_eq!(env["CLAUDE_CODE_OAUTH_TOKEN"], TOKEN);
    assert_eq!(env["ANTHROPIC_API_KEY"], "");
    for name in ["ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX"] {
        assert_eq!(env[name], "", "{name} must be withheld");
    }
    wait_until("the turn to checkpoint", || async {
        let status = ops.get_status("accounts", &owner()).await.unwrap();
        status.project.status == "WAITING"
    })
    .await;
    let events = ops.activity("accounts", 0, None, 200, &owner()).await.unwrap();
    let events = serde_json::to_string(&events).unwrap();
    assert!(!events.contains(TOKEN));
    assert!(events.contains("[REDACTED]"));
    test.close().await;
}

#[tokio::test]
async fn api_keys_are_recognised_and_bad_tokens_rejected() {
    let Accounts { test, .. } = accounts().await;
    let accounts = test.runtime.supervisor().engine_accounts();
    let api_key = format!("sk-ant-api03-{}", "k".repeat(30));
    accounts.connect_claude("accounts", &api_key, &owner()).await.unwrap();
    let overlay = accounts.claude_environment("accounts").await.unwrap();
    assert!(overlay.environment["ANTHROPIC_API_KEY"].starts_with("sk-ant-api03-"));
    assert_eq!(overlay.environment["CLAUDE_CODE_OAUTH_TOKEN"], "");
    assert_eq!(overlay.secrets.len(), 1);
    let account = accounts.account("accounts", Engine::ClaudeCode).await.unwrap().unwrap();
    assert_eq!(account.method, "api-key");

    let rejected = test
        .ops()
        .connect_claude_account("accounts", "has spaces in it which is not a token", &owner())
        .await
        .unwrap_err();
    assert_eq!(
        (rejected.http_status(), rejected.message.as_str()),
        (422, "That doesn't look like a Claude token.")
    );
    assert_eq!(
        rejected.details["suggestion"],
        "Run `claude setup-token` and paste the token it prints."
    );
    let short = test.ops().connect_claude_account("accounts", "short", &owner()).await.unwrap_err();
    assert_eq!(short.code(), "invariant_violation");
    test.close().await;
}

#[tokio::test]
async fn codex_sign_in_is_scoped_to_the_project() {
    let Accounts { test, .. } = accounts().await;
    let ops = test.ops();
    let supervisor = test.runtime.supervisor();
    assert!(supervisor.codex_auth_for("accounts").await.unwrap().same_as(supervisor.codex_auth()));

    let started = ops.start_codex_account_login("accounts", &key(), &owner()).await.unwrap();
    assert_eq!(started.user_code.as_deref(), Some("WXYZ-1234"));
    let mut status = started.status;
    for _ in 0..500 {
        status = ops.codex_account_login("accounts", "login-1", &owner()).await.unwrap().status;
        if status != "PENDING" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(status, "SUCCEEDED");
    let account = listing(&test).await["codex"].account.clone().unwrap();
    assert_eq!(account.label.as_deref(), Some("olivia@example.test · pro"));
    assert_eq!(account.connected_by.as_deref(), Some("Olivia Owner"));
    let project_manager = supervisor.codex_auth_for("accounts").await.unwrap();
    assert!(!project_manager.same_as(supervisor.codex_auth()));
    assert_eq!(project_manager.home().file_name().unwrap(), "accounts");
    assert_ne!(project_manager.home(), supervisor.codex_auth().home());

    let removed = ops.disconnect_engine_account("accounts", "codex", &owner()).await.unwrap();
    assert_eq!(removed.account, None);
    assert!(!project_manager.home().join("auth.json").exists());
    assert!(supervisor.codex_auth_for("accounts").await.unwrap().same_as(supervisor.codex_auth()));
    let viewer_denied =
        ops.disconnect_engine_account("accounts", "codex", &viewer()).await.unwrap_err();
    assert_eq!(viewer_denied.code(), "permission_denied");
    let unknown = ops.disconnect_engine_account("accounts", "gemini", &owner()).await.unwrap_err();
    assert_eq!(unknown.message, "Runtime engine must be one of: claude-code, codex.");
    test.close().await;
}

#[tokio::test]
async fn forgetting_a_project_removes_its_codex_home() {
    let Accounts { test, .. } = accounts().await;
    let accounts = test.runtime.supervisor().engine_accounts();
    accounts.start_codex_login("accounts", "command-1").await.unwrap();
    let home = accounts.codex_home("accounts");
    wait_until("the sign-in to land", || {
        let file = home.join("auth.json");
        async move { file.exists() }
    })
    .await;
    accounts.forget_project("accounts").await;
    assert!(!home.exists());
    test.close().await;
}

#[tokio::test]
async fn reconnecting_replaces_the_stored_token() {
    let Accounts { test, .. } = accounts().await;
    replace_and_disconnect(&test).await;
    test.close().await;
}

/// The same flow on PostgreSQL when `LIMINA_TEST_POSTGRES_URL` points at a database the tests
/// may wipe (see `limina-store/tests/backends.rs`).
#[tokio::test]
async fn reconnecting_replaces_the_stored_token_on_postgres() {
    let Ok(url) = std::env::var("LIMINA_TEST_POSTGRES_URL") else { return };
    let db = limina_store::Db::connect(&url).await.unwrap();
    let mut tx = db.begin_write().await.unwrap();
    tx.execute_script("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    tx.commit().await.unwrap();
    db.upgrade().await.unwrap();
    db.close().await;
    let agent = Arc::new(CapturingAgent { turns: Mutex::new(Vec::new()) });
    let test = runtime_with(factory(agent), |config| config.database_url = url.clone()).await;
    let mut project = limina_server::app::NewProject {
        slug: "accounts".into(),
        name: "Accounts".into(),
        mission: "Run on my own account.".into(),
        success_criteria: "Turns use the project's account.".into(),
        ..Default::default()
    };
    project.runtime = Some("claude-code".into());
    test.ops().create_project(project, &key(), &owner()).await.unwrap();
    replace_and_disconnect(&test).await;
    test.close().await;
}

async fn replace_and_disconnect(test: &TestRuntime) {
    let ops = test.ops();
    let first = ops.connect_claude_account("accounts", TOKEN, &owner()).await.unwrap();
    let api_key = format!("sk-ant-api03-{}", "z".repeat(30));
    let second = ops.connect_claude_account("Accounts", &api_key, &owner()).await.unwrap();
    let (first, second) = (first.account.unwrap(), second.account.unwrap());
    assert_eq!((first.method.as_str(), second.method.as_str()), ("subscription", "api-key"));
    assert_eq!(second.label.as_deref(), Some("…zzzz"));
    let overlay = test.runtime.supervisor().engine_accounts().claude_environment("accounts").await;
    assert_eq!(overlay.unwrap().environment["ANTHROPIC_API_KEY"], api_key);
    let removed = ops.disconnect_engine_account("accounts", "claude-code", &owner()).await.unwrap();
    assert_eq!(removed.account, None);
    let overlay = test.runtime.supervisor().engine_accounts().claude_environment("accounts").await;
    assert!(overlay.unwrap().environment.is_empty());
}
