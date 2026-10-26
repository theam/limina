//! Codex authentication: ports of the `codex_auth` parts of
//! `tests/test_cloud_enterprise_hardening.py` (with an in-process fake, like the Python
//! `codex_factory`), the Codex login parts of `tests/test_cloud_engine_accounts.py`, and the
//! same flows end to end over the real `app-server` protocol against the fake provider.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use limina_domain::environment::{SAFE_CODEX_ENV, SAFE_PROCESS_ENV};
use limina_engines::ProviderError;
use limina_engines::codex::auth::{
    CodexAccount, CodexAuthClient, CodexAuthConnector, DeviceCode, LoginCompletion,
};
use limina_engines::{
    CodexAuthManager, CodexAuthOptions, CodexAuthRouter, DeviceLoginAttempt, ProjectCodexAccounts,
};
use serde_json::{Value, json};
use support::{fake_provider, read_log, received, requests, starts};
use tokio::sync::watch;

// The in-process fake (Python's `FakeCodex` / `FakeDeviceHandle`) ------------------------------

#[derive(Default)]
struct FakeState {
    account_type: Option<String>,
    received_key: Option<String>,
    environments: Vec<BTreeMap<String, String>>,
    cancelled: bool,
}

struct FakeConnector {
    state: Arc<Mutex<FakeState>>,
    closed: watch::Sender<bool>,
}

impl FakeConnector {
    fn new(account_type: Option<&str>) -> Arc<Self> {
        let state =
            FakeState { account_type: account_type.map(str::to_owned), ..FakeState::default() };
        Arc::new(Self { state: Arc::new(Mutex::new(state)), closed: watch::Sender::new(false) })
    }
}

struct FakeClient {
    state: Arc<Mutex<FakeState>>,
    environment: BTreeMap<String, String>,
    closed: watch::Sender<bool>,
}

#[async_trait]
impl CodexAuthConnector for FakeConnector {
    async fn connect(
        &self,
        environment: BTreeMap<String, String>,
    ) -> Result<Arc<dyn CodexAuthClient>, ProviderError> {
        self.state.lock().unwrap().environments.push(environment.clone());
        Ok(Arc::new(FakeClient {
            state: Arc::clone(&self.state),
            environment,
            closed: self.closed.clone(),
        }))
    }
}

#[async_trait]
impl CodexAuthClient for FakeClient {
    async fn account(&self) -> Result<Option<CodexAccount>, ProviderError> {
        let state = self.state.lock().unwrap();
        Ok(state.account_type.clone().map(|account_type| CodexAccount {
            account_type,
            email: None,
            plan_type: None,
        }))
    }

    async fn login_api_key(&self, api_key: &str) -> Result<(), ProviderError> {
        let mut state = self.state.lock().unwrap();
        state.received_key = Some(api_key.to_owned());
        state.account_type = Some("apiKey".to_owned());
        let home = Path::new(&self.environment["CODEX_HOME"]);
        std::fs::write(home.join("auth.json"), "{}\n").unwrap();
        Ok(())
    }

    async fn login_chatgpt_device_code(&self) -> Result<DeviceCode, ProviderError> {
        self.closed.send_replace(false);
        Ok(DeviceCode {
            login_id: "device-login".into(),
            verification_url: "https://example.invalid/device".into(),
            user_code: "ABCD-EFGH".into(),
        })
    }

    async fn wait_login(&self, _login_id: &str) -> Result<LoginCompletion, ProviderError> {
        let mut closed = self.closed.subscribe();
        let _ =
            tokio::time::timeout(Duration::from_secs(2), closed.wait_for(|closed| *closed)).await;
        Ok(LoginCompletion { success: false, error: None })
    }

    async fn cancel_login(&self, _login_id: &str) -> Result<(), ProviderError> {
        self.state.lock().unwrap().cancelled = true;
        Ok(())
    }

    async fn logout(&self) -> Result<(), ProviderError> {
        self.state.lock().unwrap().account_type = None;
        Ok(())
    }

    async fn close(&self) {
        self.closed.send_replace(true);
    }
}

async fn manager(
    home: &Path,
    mode: &str,
    api_key: Option<&str>,
    connector: Arc<FakeConnector>,
) -> CodexAuthManager {
    let options = CodexAuthOptions {
        mode: mode.to_owned(),
        api_key: api_key.map(str::to_owned),
        connector: Some(connector),
        ..CodexAuthOptions::default()
    };
    CodexAuthManager::new(home, options).await.unwrap()
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn api_key_auth_is_materialized_without_child_environment_exposure() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("fresh").join("codex");
    let fake = FakeConnector::new(None);
    let manager = manager(&home, "api-key", Some("server-only-key"), Arc::clone(&fake)).await;

    let status = manager.ensure_ready().await.unwrap();

    assert!(status.configured);
    assert_eq!(status.active_method.as_deref(), Some("api-key"));
    assert_eq!(status.source, "server_api_key");
    let state = fake.state.lock().unwrap();
    assert_eq!(state.received_key.as_deref(), Some("server-only-key"));
    assert!(!state.environments.is_empty());
    for environment in &state.environments {
        // The child starts from an empty environment: only the safe names and the pinned home.
        for name in environment.keys() {
            assert!(
                SAFE_PROCESS_ENV.contains(&name.as_str())
                    || SAFE_CODEX_ENV.contains(&name.as_str()),
                "{name} leaked into the Codex auth process"
            );
        }
        assert!(!environment.contains_key("OPENAI_API_KEY"));
        assert_eq!(environment["CODEX_HOME"], home.to_string_lossy());
    }
    #[cfg(unix)]
    {
        assert_eq!(mode_of(&home), 0o700);
        assert_eq!(mode_of(&home.join("auth.json")), 0o600);
    }
}

#[tokio::test]
async fn auth_mutation_is_rejected_while_a_turn_is_active() {
    let temp = tempfile::tempdir().unwrap();
    let manager =
        manager(&temp.path().join("codex"), "chatgpt", None, FakeConnector::new(Some("chatgpt")))
            .await;
    let turn = manager.turn().await.unwrap();
    assert_eq!(turn.status().active_method.as_deref(), Some("chatgpt"));

    let error = manager.logout().await.unwrap_err();
    assert_eq!(error.code(), "version_conflict");
    assert_eq!(error.message, "Codex authentication cannot change while a managed turn is active.");
    assert_eq!(error.details["active_turns"], 1);
    assert_eq!(manager.start_device_login("c").await.unwrap_err().code(), "version_conflict");
    assert_eq!(
        manager.login_from_environment("api-key").await.unwrap_err().code(),
        "version_conflict"
    );

    drop(turn);
    assert_eq!(manager.active_turns(), 0);
    manager.logout().await.unwrap();
}

#[tokio::test]
async fn materialized_api_key_turns_share_the_auth_read_gate() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(
        &temp.path().join("codex"),
        "api-key",
        Some("server-only-key"),
        FakeConnector::new(None),
    )
    .await;
    manager.ensure_ready().await.unwrap();

    let first = manager.turn().await.unwrap();
    let second =
        tokio::time::timeout(Duration::from_secs(1), manager.turn()).await.unwrap().unwrap();
    assert_eq!(manager.active_turns(), 2);
    assert!(manager.status().await.configured);
    drop((first, second));
    assert_eq!(manager.active_turns(), 0);
}

#[tokio::test]
async fn chatgpt_turns_are_serialized() {
    let temp = tempfile::tempdir().unwrap();
    let manager =
        manager(&temp.path().join("codex"), "chatgpt", None, FakeConnector::new(Some("chatgpt")))
            .await;
    let first = manager.turn().await.unwrap();
    let blocked = tokio::time::timeout(Duration::from_millis(100), manager.turn()).await;
    assert!(blocked.is_err(), "a second ChatGPT turn waits for the first");
    drop(first);
    let second = tokio::time::timeout(Duration::from_secs(1), manager.turn()).await.unwrap();
    assert!(second.is_ok());
}

#[tokio::test]
async fn device_login_cancellation_releases_the_auth_writer() {
    let temp = tempfile::tempdir().unwrap();
    let fake = FakeConnector::new(None);
    let manager = manager(&temp.path().join("codex"), "chatgpt", None, Arc::clone(&fake)).await;
    let started = manager.start_device_login("command-1").await.unwrap();
    assert_eq!(started.status, "PENDING");
    assert_eq!(started.user_code.as_deref(), Some("ABCD-EFGH"));
    assert!(manager.status_if_idle().await.is_none(), "the login owns the credential store");

    let cancelled = manager.cancel_device_login(&started.login_id).await.unwrap();

    assert_eq!(cancelled.status, "CANCELLED");
    assert_eq!(cancelled.user_code, None);
    assert!(cancelled.completed_at.is_some());
    assert!(fake.state.lock().unwrap().cancelled);
    manager.logout().await.unwrap(); // proves cancellation released the exclusive auth gate
    // Replaying the command returns the same attempt.
    let replayed = manager.start_device_login("command-1").await.unwrap();
    assert_eq!(replayed, manager.login_attempt(&started.login_id).unwrap());
    assert_eq!(replayed.status, "CANCELLED");
}

#[tokio::test]
async fn login_attempts_and_modes_are_validated() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp.path().join("a"), "chatgpt", None, FakeConnector::new(None)).await;
    let error = manager.login_attempt("nope").unwrap_err();
    assert_eq!(error.message, "The Codex login attempt does not exist.");
    assert_eq!(error.details["login_id"], "nope");
    let error = manager.login_from_environment("password").await.unwrap_err();
    assert_eq!(error.message, "Runtime login method must be api-key or access-token.");

    let missing = manager.turn().await.err().unwrap();
    assert_eq!(missing.code(), "authentication_required");
    assert!(missing.message.starts_with("Codex is not authenticated."));

    let api_key_only =
        self::manager(&temp.path().join("b"), "api-key", None, FakeConnector::new(None)).await;
    let error = api_key_only.start_device_login("c").await.unwrap_err();
    assert_eq!(error.message, "ChatGPT login is disabled by LIMINA_CODEX_AUTH_MODE.");
    assert_eq!(error.details["configured_mode"], "api-key");
    let error = api_key_only.ensure_ready().await.unwrap_err();
    assert_eq!(error.message, "Codex API-key auth requires OPENAI_API_KEY on the Limina server.");
}

#[tokio::test]
async fn auto_mode_keeps_a_cached_chatgpt_session() {
    let temp = tempfile::tempdir().unwrap();
    let fake = FakeConnector::new(Some("chatgpt"));
    let manager =
        manager(&temp.path().join("codex"), "auto", Some("server-key"), Arc::clone(&fake)).await;
    let status = manager.ensure_ready().await.unwrap();
    assert_eq!(status.source, "cached_chatgpt");
    assert_eq!(fake.state.lock().unwrap().received_key, None, "the human session is not replaced");
}

// Over the real app-server protocol, against the fake provider ---------------------------------

fn write_scenario(home: &Path, scenario: &Value) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(home.join("fake-codex.json"), scenario.to_string()).unwrap();
}

async fn app_server_manager(home: &Path, options: CodexAuthOptions) -> CodexAuthManager {
    CodexAuthManager::new(home, CodexAuthOptions { codex_bin: fake_provider(), ..options })
        .await
        .unwrap()
}

async fn until_finished(manager: &CodexAuthManager, login_id: &str) -> DeviceLoginAttempt {
    for _ in 0..250 {
        let attempt = manager.login_attempt(login_id).unwrap();
        if attempt.status != "PENDING" {
            return attempt;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the login attempt never finished");
}

#[tokio::test]
async fn device_login_succeeds_over_the_app_server() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    write_scenario(&home, &json!({"login": "success"}));
    let manager = app_server_manager(&home, CodexAuthOptions::chatgpt_only("unused")).await;

    let started = manager.start_device_login("command-1").await.unwrap();
    assert_eq!(
        (started.status.as_str(), started.user_code.as_deref(), started.verification_url.as_str()),
        ("PENDING", Some("WXYZ-1234"), "https://example.invalid/device")
    );
    let finished = until_finished(&manager, &started.login_id).await;
    assert_eq!(finished.status, "SUCCEEDED");
    assert_eq!(finished.error, None);

    let status = manager.status().await;
    assert_eq!(status.active_method.as_deref(), Some("chatgpt"));
    assert_eq!(status.source, "cached_chatgpt");
    assert_eq!(status.account_label().as_deref(), Some("olivia@example.test · pro"));
    #[cfg(unix)]
    assert_eq!(mode_of(&home.join("auth.json")), 0o600);

    let log = read_log(&home.join("fake-codex.log"));
    let launch = &starts(&log)[0];
    assert_eq!(
        launch["args"],
        json!([
            "--config",
            "cli_auth_credentials_store=\"file\"",
            "app-server",
            "--listen",
            "stdio://"
        ])
    );
    let initialize = &requests(&log, "initialize")[0];
    assert_eq!(initialize["params"]["clientInfo"]["name"], "limina");
    assert_eq!(initialize["params"]["capabilities"]["experimentalApi"], true);
    assert!(initialize.get("jsonrpc").is_none());
    assert_eq!(requests(&log, "initialized").len(), starts(&log).len());
    let login = &requests(&log, "account/login/start")[0];
    assert_eq!(login["params"], json!({"type": "chatgptDeviceCode"}));
    assert_eq!(requests(&log, "account/read")[0]["params"], json!({"refreshToken": false}));
}

#[tokio::test]
async fn failed_and_cancelled_device_logins_release_the_writer() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("failing");
    write_scenario(&home, &json!({"login": "fail"}));
    let manager = app_server_manager(&home, CodexAuthOptions::chatgpt_only("unused")).await;
    let started = manager.start_device_login("command-1").await.unwrap();
    let finished = until_finished(&manager, &started.login_id).await;
    assert_eq!(finished.status, "FAILED");
    assert_eq!(finished.error.as_deref(), Some("ChatGPT login failed or expired."));
    assert!(manager.status_if_idle().await.is_some());

    let home = temp.path().join("waiting");
    write_scenario(&home, &json!({"login": "wait"}));
    let manager = app_server_manager(&home, CodexAuthOptions::chatgpt_only("unused")).await;
    let started = manager.start_device_login("command-2").await.unwrap();
    let other = manager.start_device_login("command-3").await.unwrap_err();
    assert_eq!(other.code(), "version_conflict", "one login at a time");
    let cancelled = manager.cancel_device_login(&started.login_id).await.unwrap();
    assert_eq!(cancelled.status, "CANCELLED");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        manager.login_attempt(&started.login_id).unwrap().status,
        "CANCELLED",
        "the provider's failed completion does not overwrite the cancellation"
    );
    let log = read_log(&home.join("fake-codex.log"));
    assert_eq!(requests(&log, "account/login/cancel")[0]["params"], json!({"loginId": "login-1"}));
    manager.logout().await.unwrap();
}

#[tokio::test]
async fn server_credentials_are_materialized_over_the_app_server() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("api-key");
    write_scenario(&home, &json!({}));
    let options = CodexAuthOptions {
        mode: "api-key".into(),
        api_key: Some("sk-server-only".into()),
        ..CodexAuthOptions::default()
    };
    let manager = app_server_manager(&home, options).await;
    let status = manager.ensure_ready().await.unwrap();
    assert_eq!(
        (status.active_method.as_deref(), status.source.as_str()),
        (Some("api-key"), "server_api_key")
    );
    let log = read_log(&home.join("fake-codex.log"));
    assert_eq!(
        requests(&log, "account/login/start")[0]["params"],
        json!({"type": "apiKey", "apiKey": "sk-server-only"})
    );
    for start in starts(&log) {
        assert!(start["env"].get("OPENAI_API_KEY").is_none());
        assert!(
            start["env"].get("CARGO_MANIFEST_DIR").is_none(),
            "providers start from an empty environment"
        );
    }
    // A second readiness check reuses the materialized credential.
    manager.ensure_ready().await.unwrap();
    assert_eq!(requests(&read_log(&home.join("fake-codex.log")), "account/login/start").len(), 1);
    // Logout clears it.
    let status = manager.logout().await.unwrap();
    assert!(!status.configured);
    assert_eq!(requests(&read_log(&home.join("fake-codex.log")), "account/logout").len(), 1);
}

#[tokio::test]
async fn access_tokens_log_in_through_the_codex_cli() {
    let temp = tempfile::tempdir().unwrap();
    let login = |token: &str| CodexAuthOptions {
        mode: "access-token".into(),
        access_token: Some(token.into()),
        ..CodexAuthOptions::default()
    };
    let home = temp.path().join("good");
    write_scenario(&home, &json!({}));
    let manager = app_server_manager(&home, login("valid-access-token")).await;
    let status = manager.ensure_ready().await.unwrap();
    assert_eq!(
        (status.active_method.as_deref(), status.source.as_str(), status.account_plan.as_deref()),
        (Some("chatgpt"), "server_access_token", Some("team"))
    );
    let log = read_log(&home.join("fake-codex.log"));
    let cli = starts(&log).into_iter().find(|start| start["args"][0] == "login").unwrap();
    assert_eq!(cli["args"], json!(["login", "--with-access-token"]));
    assert!(log.iter().any(|entry| entry["access_token"] == "valid-access-token"));

    let home = temp.path().join("bad");
    write_scenario(&home, &json!({}));
    let manager = app_server_manager(&home, login("rejected-token")).await;
    let error = manager.ensure_ready().await.unwrap_err();
    assert_eq!(error.code(), "transport_error");
    assert_eq!(error.message, "Limina could not materialize Codex access-token authentication.");
    assert_eq!(error.details["reason"], "Codex login rejected the configured access token.");
}

#[tokio::test]
async fn unavailable_codex_reports_an_unavailable_status() {
    let temp = tempfile::tempdir().unwrap();
    let options =
        CodexAuthOptions { codex_bin: "/nonexistent/codex".into(), ..CodexAuthOptions::default() };
    let manager = CodexAuthManager::new(temp.path().join("codex"), options).await.unwrap();
    let status = manager.status().await;
    assert_eq!((status.configured, status.source.as_str()), (false, "unavailable"));
    assert_eq!(status.error.as_deref(), Some("Codex account status is unavailable."));
    assert_eq!(serde_json::to_value(&status).unwrap()["single_runtime_node"], true);
}

// Project accounts (`test_cloud_engine_accounts.py`) ------------------------------------------

#[tokio::test]
async fn codex_sign_in_is_scoped_to_the_project() {
    let temp = tempfile::tempdir().unwrap();
    let runtime_home = temp.path().join("codex");
    write_scenario(&runtime_home, &json!({}));
    let runtime = app_server_manager(&runtime_home, CodexAuthOptions::default()).await;
    let accounts = Arc::new(ProjectCodexAccounts::new(
        temp.path().join("accounts").join("codex"),
        fake_provider(),
    ));
    let router = CodexAuthRouter::new(runtime.clone(), Arc::clone(&accounts));
    assert!(router.for_project("accounts").await.unwrap().same_as(&runtime));

    let project = accounts.manager("accounts").await.unwrap();
    assert_eq!(project.mode(), "chatgpt");
    let started = project.start_device_login("command-1").await.unwrap();
    assert_eq!(started.user_code.as_deref(), Some("WXYZ-1234"));
    assert_eq!(until_finished(&project, &started.login_id).await.status, "SUCCEEDED");
    assert_eq!(
        project.status().await.account_label().as_deref(),
        Some("olivia@example.test · pro")
    );

    let resolved = router.for_project("accounts").await.unwrap();
    assert!(resolved.same_as(&project));
    assert!(!resolved.same_as(&runtime));
    assert_eq!(resolved.home().file_name().unwrap(), "accounts");
    assert_ne!(resolved.home(), runtime.home());
    assert!(accounts.signed_in("accounts").await);

    accounts.sign_out("accounts").await.unwrap();
    assert!(!resolved.home().join("auth.json").exists());
    assert!(router.for_project("accounts").await.unwrap().same_as(&runtime));
    let log = read_log(&accounts.home("accounts").join("fake-codex.log"));
    assert_eq!(requests(&log, "account/logout").len(), 1);

    accounts.forget("accounts").await;
    assert!(!accounts.home("accounts").exists());
    assert!(
        received(&read_log(&runtime_home.join("fake-codex.log"))).is_empty(),
        "the runtime store was never touched"
    );
}
