//! Live smoke tests against the real providers. They use real accounts and cost tokens, so they
//! run only with `LIMINA_LIVE_PROVIDER_TESTS=1`:
//!
//! ```bash
//! LIMINA_LIVE_PROVIDER_TESTS=1 cargo test -p limina-engines --test live -- --nocapture
//! ```
//!
//! - Codex uses the engine settings from the environment (`LIMINA_CODEX_BIN`,
//!   `LIMINA_CODEX_MODEL`, `LIMINA_CODEX_AUTH_MODE`, `OPENAI_API_KEY`, `CODEX_ACCESS_TOKEN`) and
//!   the Codex home in `LIMINA_LIVE_CODEX_HOME`, else `CODEX_HOME`, else `~/.codex` (which must
//!   already hold a sign-in unless a server credential is set). The home is made private (0700).
//! - Claude uses `LIMINA_CLAUDE_BIN` and `LIMINA_CLAUDE_MODEL`, a fresh configuration directory,
//!   and credentials from `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN` or a cloud provider
//!   switch. Set `LIMINA_CLAUDE_WEAKER_NESTED_SANDBOX=1` inside containers.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use limina_domain::environment::{claude_credential_source, process_environment};
use limina_engines::{
    AgentSession, ClaudeAgentSession, ClaudeSessionConfig, CodexAgentSession, CodexAuthManager,
    CodexAuthOptions, CodexSessionConfig, EngineSettings, TurnRequest,
};
use support::Recorder;

const PROMPT: &str = "This is a connectivity check for Limina. Do not run commands, read files or \
                      edit anything. Reply only with the project checkpoint: summary \"Live check \
                      passed.\", status \"COMPLETE\", current_objective \"Verify connectivity.\", \
                      next_step \"Nothing further.\", blocker \"None\".";

fn enabled(provider: &str) -> bool {
    let enabled = std::env::var("LIMINA_LIVE_PROVIDER_TESTS").as_deref() == Ok("1");
    if !enabled {
        eprintln!("skipping the live {provider} smoke test: set LIMINA_LIVE_PROVIDER_TESTS=1");
    }
    enabled
}

fn request(
    workspace: &std::path::Path,
    continuation: Option<String>,
    recorder: &Arc<Recorder>,
) -> TurnRequest {
    TurnRequest {
        settings: Default::default(),
        prompt: PROMPT.to_owned(),
        workspace: workspace.to_path_buf(),
        continuation_id: continuation,
        runtime_env: BTreeMap::new(),
        mcp_servers: Vec::new(),
        observer: recorder.clone(),
    }
}

#[tokio::test]
async fn live_codex_turn_and_resume() {
    if !enabled("Codex") {
        return;
    }
    let settings = EngineSettings::from_environment().unwrap();
    let home = std::env::var_os("LIMINA_LIVE_CODEX_HOME")
        .or_else(|| std::env::var_os("CODEX_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".codex"));
    let auth = CodexAuthManager::new(
        home,
        CodexAuthOptions {
            mode: settings.codex_auth_mode.clone(),
            api_key: settings.openai_api_key.clone(),
            access_token: settings.codex_access_token.clone(),
            codex_bin: settings.codex_bin.clone(),
            ..CodexAuthOptions::default()
        },
    )
    .await
    .unwrap();
    let status = auth.ensure_ready().await.unwrap();
    eprintln!("Codex account: {status:?}");
    let session = CodexAgentSession::new(
        CodexSessionConfig {
            codex_bin: settings.codex_bin.clone(),
            model: settings.codex_model.clone(),
            sandbox: "read-only".to_owned(),
            pricing: settings.codex_pricing.clone(),
        },
        Arc::new(auth),
    );
    let workspace = tempfile::tempdir().unwrap();

    let recorder = Recorder::new();
    let first = session.run_turn(request(workspace.path(), None, &recorder)).await.unwrap();
    eprintln!("first turn: {first:?}\nevents: {:?}", recorder.kinds());
    assert_eq!(first.decision.status, "COMPLETE");
    let thread = first.continuation_id.clone().expect("a Codex thread");
    assert_eq!(recorder.continuations().first(), Some(&thread));

    let recorder = Recorder::new();
    let resumed =
        session.run_turn(request(workspace.path(), Some(thread.clone()), &recorder)).await.unwrap();
    eprintln!("resumed turn: {resumed:?}");
    assert_eq!(resumed.continuation_id.as_deref(), Some(thread.as_str()));
    assert_eq!(resumed.decision.status, "COMPLETE");
}

#[tokio::test]
async fn live_claude_turn_and_resume() {
    if !enabled("Claude") {
        return;
    }
    if claude_credential_source(&process_environment()).is_none() {
        eprintln!("skipping the live Claude smoke test: no Claude credential in the environment");
        return;
    }
    let settings = EngineSettings::from_environment().unwrap();
    let state = tempfile::tempdir().unwrap();
    let workspace = state.path().join("workspaces").join("live");
    std::fs::create_dir_all(&workspace).unwrap();
    let session = ClaudeAgentSession::new(ClaudeSessionConfig {
        claude_bin: settings.claude_bin.clone(),
        model: settings.claude_model.clone(),
        weaker_nested_sandbox: settings.claude_weaker_nested_sandbox,
        ..ClaudeSessionConfig::new(state.path().join("claude").join("live"), state.path())
    });

    let recorder = Recorder::new();
    let first = session.run_turn(request(&workspace, None, &recorder)).await.unwrap();
    eprintln!("first turn: {first:?}\nevents: {:?}", recorder.kinds());
    assert_eq!(first.decision.status, "COMPLETE");
    let session_id = first.continuation_id.clone().expect("a Claude session");
    assert!(first.usage.is_some());

    let recorder = Recorder::new();
    let resumed = session.run_turn(request(&workspace, Some(session_id), &recorder)).await.unwrap();
    eprintln!("resumed turn: {resumed:?}");
    assert_eq!(resumed.decision.status, "COMPLETE");
}
