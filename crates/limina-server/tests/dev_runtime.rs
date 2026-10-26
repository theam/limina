//! The development runtime as `limina dev` runs it: options, the prepared configuration, engine
//! readiness, the seeded demo on the real supervisor, and the scripted Codex sign-in.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod dev_support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dev_support::{DevRuntime, operator, wait_until};
use limina_core::ErrorKind;
use limina_domain::Engine;
use limina_engines::codex::auth::{CodexAuthClient, CodexAuthConnector};
use limina_server::app::{Guidance, Runtime};
use limina_server::dev::{
    self, DEFAULT_PACE, DEMO_ACTOR, DEMO_SLUG, DEVICE_USER_CODE, DEVICE_VERIFICATION_URL,
    DevOptions, NO_SHARED_CREDENTIALS_DETAIL, SCRIPTED_ACCOUNT_EMAIL, SCRIPTED_ENGINE_DETAIL,
    SCRIPTED_MODEL, ScriptedCodexConnector, SeedOutcome,
};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
}

const QUESTION: &str = "How much answer rate can we give up? At 0.6 we lose 1.5 pts for −41% \
                        unsupported citations; at 0.7 we lose 3.2 pts for −55%.";

#[test]
fn options_follow_the_python_defaults() {
    let options = DevOptions::from_map(&BTreeMap::new()).unwrap();
    assert_eq!(options.pace, DEFAULT_PACE);
    assert_eq!(options.pace, Duration::from_secs_f64(1.2));
    assert_eq!(options.data, PathBuf::from(".limina/ui-dev"));
    assert_eq!(options.database_url, "sqlite:///.limina/ui-dev/runtime.db");
    assert!(!options.seed_only);
    assert_eq!(options.port, 7433);
    assert!(options.shared_credentials);
    assert_eq!((options.api_token.as_str(), options.admin_token.as_str()), ("dev", "dev-admin"));
    assert_eq!(options.sign_in_delay, Duration::from_secs(6));
    assert_eq!(options.internal_url(), "http://127.0.0.1:7433");
    let without_build = DevOptions { web_dir: None, ..options.clone() };
    assert_eq!(without_build.console_url(), "http://127.0.0.1:5173/#token=dev-admin");
    assert_eq!(
        without_build.startup_notice(),
        "\n  Limina dev runtime  →  http://127.0.0.1:5173/#token=dev-admin\n"
    );
    let with_build = DevOptions { web_dir: Some("web/dist".into()), ..options };
    assert_eq!(with_build.console_url(), "http://127.0.0.1:7433/#token=dev-admin");
    let debug = format!("{with_build:?}");
    assert!(!debug.contains("dev-admin"), "tokens stay out of debug output: {debug}");
}

#[test]
fn options_read_the_environment() {
    let options = DevOptions::from_map(&env(&[
        ("LIMINA_DEV_PACE", "0.6"),
        ("LIMINA_DEV_DATA", "/tmp/limina-seed"),
        ("LIMINA_DEV_SEED", "YES"),
        ("LIMINA_DEV_PORT", "7439"),
        ("LIMINA_DEV_SHARED_CREDENTIALS", "0"),
        ("LIMINA_API_TOKEN", "people"),
        ("LIMINA_ADMIN_API_TOKEN", "admins"),
    ]))
    .unwrap();
    assert_eq!(options.pace, Duration::from_millis(600));
    assert_eq!(options.data, PathBuf::from("/tmp/limina-seed"));
    assert_eq!(options.database_url, "sqlite:////tmp/limina-seed/runtime.db");
    assert!(options.seed_only);
    assert_eq!(options.internal_url(), "http://127.0.0.1:7439");
    assert!(!options.shared_credentials);
    assert_eq!((options.api_token.as_str(), options.admin_token.as_str()), ("people", "admins"));
    assert_eq!(options.startup_notice(), format!("  Seeding the '{DEMO_SLUG}' demo…"));

    let options = DevOptions::from_map(&env(&[
        ("LIMINA_DEV_DATABASE_URL", "sqlite:////var/lib/limina/runtime.db"),
        ("LIMINA_DEV_SHARED_CREDENTIALS", "FALSE"),
        ("LIMINA_DEV_SEED", "no"),
        ("LIMINA_DEV_PACE", ""),
    ]))
    .unwrap();
    assert_eq!(options.database_url, "sqlite:////var/lib/limina/runtime.db");
    assert!(!options.shared_credentials);
    assert!(!options.seed_only);
    assert_eq!(options.pace, DEFAULT_PACE);
    let shared = DevOptions::from_map(&env(&[("LIMINA_DEV_SHARED_CREDENTIALS", "1")])).unwrap();
    assert!(shared.shared_credentials);

    for (name, value) in [
        ("LIMINA_DEV_PACE", "fast"),
        ("LIMINA_DEV_PACE", "-1"),
        ("LIMINA_DEV_PACE", "NaN"),
        ("LIMINA_DEV_PORT", "99999"),
    ] {
        let error = DevOptions::from_map(&env(&[(name, value)])).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Invariant, "{name}={value}");
        assert!(error.message.starts_with(name), "{}", error.message);
    }
}

#[tokio::test]
async fn the_prepared_runtime_runs_the_scripted_engine_without_providers() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = DevOptions::from_map(&BTreeMap::new()).unwrap();
    options.data = dir.path().join("state");
    options.database_url = format!("sqlite:///{}", dir.path().join("state/runtime.db").display());
    options.port = 7499;
    let config = dev::prepare(&options).await.unwrap();
    assert!(options.data.is_dir(), "prepare creates the data directory");
    assert_eq!(config.database_url, options.database_url);
    assert_eq!(config.workspace_root, options.data.join("workspaces"));
    assert_eq!(config.secret_key_path, options.data.join("secret.key"));
    assert_eq!(config.internal_url, "http://127.0.0.1:7499");
    let session = (config.agent_factory.as_ref().unwrap())("demo", Engine::ClaudeCode).unwrap();
    assert_eq!(session.model().as_deref(), Some(SCRIPTED_MODEL));
    assert!(config.codex_connector.is_some());
    assert!(format!("{config:?}").contains("custom_engine_status: true"));
}

#[tokio::test]
async fn engines_report_the_scripted_engine() {
    let dev = DevRuntime::start(|_| {}).await;
    let engines = dev.runtime.operations().engine_availability().await;
    let engines = serde_json::to_value(engines).unwrap();
    assert_eq!(
        engines,
        serde_json::json!([
            {"engine": "codex", "label": "Codex", "installed": true, "credentials": true,
             "detail": SCRIPTED_ENGINE_DETAIL, "model": SCRIPTED_MODEL},
            {"engine": "claude-code", "label": "Claude", "installed": true, "credentials": true,
             "detail": SCRIPTED_ENGINE_DETAIL, "model": SCRIPTED_MODEL},
        ])
    );
    dev.close().await;

    let dev = DevRuntime::start(|options| options.shared_credentials = false).await;
    for engine in dev.runtime.operations().engine_availability().await {
        assert!(engine.installed);
        assert!(!engine.credentials);
        assert_eq!(engine.detail, NO_SHARED_CREDENTIALS_DETAIL);
    }
    dev.create_project("accounts").await;
    let accounts = dev.runtime.operations().engine_accounts("accounts", &operator()).await.unwrap();
    assert!(accounts.iter().all(|account| !account.runtime_available && account.account.is_none()));
    dev.close().await;
}

async fn coordinator(runtime: &Runtime) -> limina_domain::service::CoordinatorView {
    runtime.service().get_challenge(DEMO_SLUG).await.unwrap().coordinator.unwrap()
}

#[tokio::test]
async fn the_seeded_demo_plays_until_its_first_question() {
    let dev = DevRuntime::start(|_| {}).await;
    let runtime = &dev.runtime;
    dev::seed_demo(runtime).await.unwrap();
    let project = runtime.service().get_challenge(DEMO_SLUG).await.unwrap();
    assert_eq!(project.name, "Retrieval reliability (demo)");
    assert_eq!(project.runtime_engine, "codex");
    assert!(project.context.ends_with("the history and numbers are illustrative, not measured."));
    let members = runtime.collaboration().list_members(DEMO_SLUG).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(
        (members[0].subject.as_str(), members[0].display_name.as_str()),
        ("local:Dev", "Dev")
    );

    let outcome = tokio::time::timeout(Duration::from_secs(20), dev::wait_until_seeded(runtime))
        .await
        .expect("the demo never asked");
    let outcome = outcome.unwrap();
    assert_eq!(outcome, SeedOutcome::Waiting { question: QUESTION.to_owned() });
    assert_eq!(outcome.message(), format!("  Seeded '{DEMO_SLUG}': waiting on a human decision."));
    let state = coordinator(runtime).await;
    assert_eq!(state.status, "WAITING");
    assert_eq!(state.next_step, "Run E002 on the test split with the threshold you choose");
    assert_eq!(state.continuation_id, None, "a real engine can take over");
    let statuses = dev.statuses(DEMO_SLUG).await;
    assert_eq!(statuses["H001"], "CONFIRMED");
    assert_eq!(statuses["F001"], "PUBLISHED");
    assert_eq!(statuses["E002"], "DESIGNED");
    assert!(!statuses.contains_key("F002"));
    let runs = runtime.collaboration().runs(DEMO_SLUG, None, None, 50).await.unwrap();
    assert_eq!(runs.total, 3, "explore, first experiment, question");
    assert!(runs.items.iter().all(|run| run.status == "COMPLETED"
        && run.model.as_deref() == Some(SCRIPTED_MODEL)
        && run.tool_calls > 0
        && run.usage.total_tokens.is_some()));
    let events = runtime.service().events(DEMO_SLUG, 0, 500).await.unwrap();
    assert!(events.iter().any(|event| event.event_type == "runtime.codex"));

    // Seeding again changes nothing.
    dev::seed_demo(runtime).await.unwrap();
    assert_eq!(runtime.service().list_projects(false).await.unwrap().len(), 1);

    // The answer runs E002 and publishes F002.
    let answer = Guidance::new(DEMO_SLUG, "ANSWER", "Keep it under 2 points.", DEMO_ACTOR);
    runtime.supervisor().submit_message(answer).await.unwrap();
    wait_until("F002", || async {
        dev.statuses(DEMO_SLUG).await.get("F002").is_some_and(|status| status == "PUBLISHED")
    })
    .await;
    wait_until("the checkpoint after E002", || async {
        let state = coordinator(runtime).await;
        state.status == "WAITING" && state.blocker == "None"
    })
    .await;
    assert_eq!(dev.statuses(DEMO_SLUG).await["H002"], "CONFIRMED");
    dev.close().await;
}

#[tokio::test]
async fn a_failing_demo_ends_seeding_with_its_blocker() {
    let dev = DevRuntime::start(|_| {}).await;
    // A second runtime whose agents call the first one's internal API: their capabilities are
    // refused, so the demo's first turn fails.
    let dir = tempfile::tempdir().unwrap();
    let mut options = dev.options.clone();
    options.data = dir.path().join("other");
    options.database_url = format!("sqlite:///{}", dir.path().join("other.db").display());
    let runtime = Runtime::new(dev::prepare(&options).await.unwrap()).await.unwrap();
    dev::seed_demo(&runtime).await.unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(20), dev::wait_until_seeded(&runtime))
        .await
        .expect("seeding never finished")
        .unwrap();
    let SeedOutcome::Failed { blocker } = &outcome else { panic!("{outcome:?}") };
    assert!(!blocker.is_empty());
    assert_eq!(outcome.message(), format!("  Seeding failed: {blocker}"));
    let runs = runtime.collaboration().runs(DEMO_SLUG, None, None, 10).await.unwrap();
    let error = runs.items[0].error.clone().unwrap();
    assert_eq!(error.code.as_deref(), Some("transport_error"));
    assert_eq!(
        error.message.as_deref(),
        Some("The internal agent API rejected GET artifacts with HTTP 401.")
    );
    runtime.shutdown().await;
    runtime.db().close().await;
    dev.close().await;
}

#[tokio::test]
async fn the_scripted_codex_sign_in_connects_a_project_account() {
    let dev = DevRuntime::start(|options| options.shared_credentials = false).await;
    let ops = dev.runtime.operations();
    let owner = operator();
    dev.create_project("accounts").await;

    let attempt = ops.start_codex_account_login("accounts", "login-1", &owner).await.unwrap();
    assert_eq!(attempt.status, "PENDING");
    assert_eq!(attempt.user_code.as_deref(), Some(DEVICE_USER_CODE));
    assert_eq!(attempt.verification_url, DEVICE_VERIFICATION_URL);
    assert!(attempt.login_id.starts_with("dev-"));
    let replay = ops.start_codex_account_login("accounts", "login-1", &owner).await.unwrap();
    assert_eq!(replay.login_id, attempt.login_id);
    wait_until("the sign-in to confirm itself", || async {
        let current = ops.codex_account_login("accounts", &attempt.login_id, &owner).await.unwrap();
        current.status == "SUCCEEDED"
    })
    .await;
    let codex = ops
        .engine_accounts("accounts", &owner)
        .await
        .unwrap()
        .into_iter()
        .find(|account| account.engine == Engine::Codex)
        .unwrap();
    let account = codex.account.unwrap();
    assert_eq!(account.label.as_deref(), Some("dev@example.com · plus"));
    assert_eq!(account.connected_by.as_deref(), Some(DEMO_ACTOR));
    let status = dev.runtime.supervisor().project_engine_status("accounts", Engine::Codex).await;
    assert_eq!(
        status.unwrap().detail,
        "Runs on the project's own account (dev@example.com · plus), connected by Dev."
    );
    let home = dev.data().join("accounts/codex/accounts");
    assert!(home.join("auth.json").is_file());

    ops.disconnect_engine_account("accounts", "codex", &owner).await.unwrap();
    assert!(!home.join("auth.json").exists());

    // A cancelled sign-in stays signed out.
    let attempt = ops.start_codex_account_login("accounts", "login-2", &owner).await.unwrap();
    let cancelled =
        ops.cancel_codex_account_login("accounts", &attempt.login_id, &owner).await.unwrap();
    assert_eq!(cancelled.status, "CANCELLED");
    tokio::time::sleep(dev.options.sign_in_delay * 3).await;
    assert!(!home.join("auth.json").exists());
    let current = ops.codex_account_login("accounts", &attempt.login_id, &owner).await.unwrap();
    assert_eq!(current.status, "CANCELLED");
    dev.close().await;
}

#[tokio::test]
async fn the_scripted_codex_client_signs_a_home_in_and_out() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_string_lossy().into_owned();
    let connector = ScriptedCodexConnector::with_delay(Duration::from_millis(10));
    let client: Arc<dyn CodexAuthClient> =
        connector.connect(env(&[("CODEX_HOME", &home)])).await.unwrap();
    assert_eq!(client.account().await.unwrap(), None);
    client.login_api_key("ignored").await.unwrap();
    assert_eq!(client.account().await.unwrap(), None, "API keys are not scripted");
    let code = client.login_chatgpt_device_code().await.unwrap();
    assert_eq!(code.user_code, DEVICE_USER_CODE);
    let completion = client.wait_login(&code.login_id).await.unwrap();
    assert!(completion.success);
    assert_eq!(std::fs::read_to_string(dir.path().join("auth.json")).unwrap(), "{}\n");
    let account = client.account().await.unwrap().unwrap();
    assert_eq!(account.account_type, "chatgpt");
    assert_eq!(account.email.as_deref(), Some(SCRIPTED_ACCOUNT_EMAIL));
    assert_eq!(account.plan_type.as_deref(), Some("plus"));
    client.logout().await.unwrap();
    client.logout().await.unwrap();
    assert_eq!(client.account().await.unwrap(), None);
    client.close().await;
    assert!(connector.connect(BTreeMap::new()).await.is_err(), "CODEX_HOME is required");
}
