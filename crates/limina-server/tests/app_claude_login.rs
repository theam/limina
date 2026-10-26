//! Signing a project in to Claude from the console, against a scripted `claude` CLI that behaves
//! like `claude auth login --claudeai` (prints the claude.com URL, waits for the pasted code).

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use limina_core::Principal;
use limina_domain::Engine;
use limina_domain::project::NewChallenge;
use limina_domain::service::ChallengeService;
use limina_engines::{ClaudeLogins, ProjectCodexAccounts};
use limina_server::app::ProjectEngineAccounts;
use limina_store::{Db, SecretCipher};

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1 $2" in
  "auth login")
    echo "Opening browser to sign in…"
    echo "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=test&state=abc"
    printf "Paste code here if prompted > "
    read code
    if [ "$code" = "good-code#abc" ]; then
      printf '{"claudeAiOauth":{}}' > "$CLAUDE_CONFIG_DIR/.credentials.json"
      echo "Login successful."
      exit 0
    fi
    echo "OAuth error: invalid code" >&2
    exit 1;;
  "auth status")
    if [ -f "$CLAUDE_CONFIG_DIR/.credentials.json" ]; then
      echo '{"loggedIn": true, "authMethod": "claude.ai", "email": "ada@example.com", "subscriptionType": "max"}'
    else
      echo '{"loggedIn": false, "authMethod": "none"}'
    fi;;
  "auth logout")
    rm -f "$CLAUDE_CONFIG_DIR/.credentials.json";;
esac
"#;

fn fake_claude(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let path = dir.join("bin").join("claude");
    std::fs::write(&path, FAKE_CLAUDE).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

async fn accounts(root: &Path) -> (ProjectEngineAccounts, PathBuf) {
    let db =
        Db::connect(&format!("sqlite:///{}", root.join("runtime.db").display())).await.unwrap();
    db.initialize().await.unwrap();
    let cipher = Arc::new(SecretCipher::ephemeral());
    ChallengeService::new(db.clone(), Some(Arc::clone(&cipher)))
        .create_challenge(
            NewChallenge {
                slug: "demo".into(),
                name: "Demo".into(),
                objective: "Run on my Claude account.".into(),
                success_criteria: "Turns use it.".into(),
                runtime_engine: Some("claude-code".into()),
                ..NewChallenge::default()
            },
            "Ada",
            "create-demo",
        )
        .await
        .unwrap();
    let config_root = root.join("claude");
    let logins = ClaudeLogins::new(fake_claude(root).display().to_string());
    let codex = Arc::new(ProjectCodexAccounts::new(root.join("codex"), "codex".to_owned()));
    (
        ProjectEngineAccounts::new(db, Some(cipher), codex, Arc::new(logins), config_root.clone()),
        config_root.join("demo"),
    )
}

fn ada() -> Principal {
    Principal::new("user_ada", "Ada").with_email(Some("ada@example.com".into()))
}

#[tokio::test]
async fn a_project_signs_in_to_claude_with_the_owners_account() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, config_dir) = accounts(dir.path()).await;

    let started = accounts.start_claude_login("demo", "cmd-1").await.unwrap();
    assert_eq!(started.status, "PENDING");
    assert!(started.verification_url.starts_with("https://claude.com/cai/oauth/authorize"));
    let again = accounts.start_claude_login("demo", "cmd-1").await.unwrap();
    assert_eq!(again.login_id, started.login_id, "the same command returns the same sign-in");

    let done = accounts
        .submit_claude_code("demo", &started.login_id, " good-code#abc ", &ada())
        .await
        .unwrap();
    assert_eq!(done.status, "SUCCEEDED", "{done:?}");
    assert!(
        config_dir.join(".credentials.json").exists(),
        "the CLI keeps the sign-in in the project dir"
    );

    let account = accounts.account("demo", Engine::ClaudeCode).await.unwrap().unwrap();
    assert_eq!(account.method, "claude-login");
    assert_eq!(account.label.as_deref(), Some("ada@example.com · max"));
    assert_eq!(account.connected_by.as_deref(), Some("Ada"));

    let overlay = accounts.claude_environment("demo").await.unwrap();
    assert!(overlay.secrets.is_empty(), "no token travels in the environment");
    assert_eq!(overlay.environment.get("ANTHROPIC_API_KEY").map(String::as_str), Some(""));
    assert_eq!(overlay.environment.get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str), Some(""));

    accounts.disconnect("demo", Engine::ClaudeCode).await.unwrap();
    assert!(accounts.account("demo", Engine::ClaudeCode).await.unwrap().is_none());
    assert!(!config_dir.join(".credentials.json").exists(), "disconnecting signs the CLI out");
}

#[tokio::test]
async fn a_wrong_code_fails_without_connecting_anything() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, _) = accounts(dir.path()).await;
    let started = accounts.start_claude_login("demo", "cmd-1").await.unwrap();
    let failed =
        accounts.submit_claude_code("demo", &started.login_id, "nope", &ada()).await.unwrap();
    assert_eq!(failed.status, "FAILED");
    assert!(failed.error.unwrap().contains("didn't accept that code"));
    assert!(accounts.account("demo", Engine::ClaudeCode).await.unwrap().is_none());

    let next = accounts.start_claude_login("demo", "cmd-2").await.unwrap();
    let cancelled = accounts.cancel_claude_login("demo", &next.login_id).await.unwrap();
    assert_eq!(cancelled.status, "CANCELLED");
    let missing = accounts.claude_login("other", &next.login_id, &ada()).await.unwrap_err();
    assert_eq!(missing.code(), "not_found");
}
