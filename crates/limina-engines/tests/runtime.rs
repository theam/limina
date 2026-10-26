//! Engine-level ports of `tests/test_cloud_runtime.py`: child environments, secret redaction
//! and the default session factory. (The Claude adapter tests live in `claude_session.rs`;
//! the supervisor tests wait for the supervisor port.)

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use limina_domain::Engine;
use limina_domain::environment::{claude_environment, codex_environment};
use limina_engines::{
    CodexAuthManager, CodexAuthOptions, CodexAuthRouter, DefaultAgentFactory, EngineSettings,
    ProjectCodexAccounts, RuntimeDecision, RuntimeEvent, RuntimeTurn, redact_event, redact_turn,
};
use serde_json::{Map, json};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

#[test]
fn codex_child_environment_excludes_control_plane_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let inherited = map(&[
        ("OPENAI_API_KEY", "sdk-key"),
        ("CLAUDE_CODE_OAUTH_TOKEN", "claude-oauth-token"),
        ("CODEX_HOME", "/var/lib/limina/codex"),
        ("LIMINA_DATABASE_URL", "postgresql://control-plane"),
        ("LIMINA_API_TOKEN", "admin-token"),
        ("LIMINA_SECRET_KEY", "master-key"),
        ("UNRELATED_SECRET", "other-secret"),
    ]);
    let overlay = map(&[
        ("LIMINA_INTERNAL_URL", "http://runtime:7433"),
        ("LIMINA_INTERNAL_TOKEN", "project-capability"),
    ]);
    let home = temp.path().join("codex");

    let child = codex_environment(&inherited, &overlay, &home).unwrap();

    // Python blanked these names; the Rust runtime spawns with `env_clear()` and omits them.
    for name in [
        "OPENAI_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "LIMINA_DATABASE_URL",
        "LIMINA_API_TOKEN",
        "LIMINA_SECRET_KEY",
        "UNRELATED_SECRET",
    ] {
        assert!(!child.contains_key(name), "{name} reached Codex");
    }
    assert_eq!(child["CODEX_HOME"], home.to_string_lossy());
    assert_eq!(child["LIMINA_INTERNAL_TOKEN"], "project-capability");
}

#[test]
fn claude_child_environment_is_scoped_and_persistent() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("claude").join("retrieval");
    let inherited = map(&[
        ("ANTHROPIC_API_KEY", "claude-key"),
        ("CLAUDE_CODE_OAUTH_TOKEN", "claude-oauth-token"),
        ("OPENAI_API_KEY", "other-provider-key"),
        ("LIMINA_DATABASE_URL", "postgresql://control-plane"),
        ("LIMINA_SECRET_KEY", "master-key"),
    ]);

    let child = claude_environment(
        &inherited,
        &map(&[("LIMINA_INTERNAL_TOKEN", "project-capability")]),
        &config_dir,
    )
    .unwrap();

    assert_eq!(child["ANTHROPIC_API_KEY"], "claude-key");
    assert_eq!(child["CLAUDE_CODE_OAUTH_TOKEN"], "claude-oauth-token");
    for name in ["OPENAI_API_KEY", "LIMINA_DATABASE_URL", "LIMINA_SECRET_KEY"] {
        assert!(!child.contains_key(name), "{name} reached Claude");
    }
    assert_eq!(child["CLAUDE_CONFIG_DIR"], config_dir.to_string_lossy());
    assert_eq!(child["CLAUDE_CODE_SUBPROCESS_ENV_SCRUB"], "1");
    assert_eq!(child["LIMINA_INTERNAL_TOKEN"], "project-capability");
}

#[test]
fn runtime_events_and_decisions_redact_project_secret_values() {
    let secrets = ["sensitive-123"];
    let mut detail = Map::new();
    detail.insert("nested".into(), json!(["sensitive-123", {"value": "prefix-sensitive-123"}]));
    let event =
        redact_event(&RuntimeEvent::new("runtime.codex", "Used sensitive-123", detail), &secrets);
    let turn = redact_turn(
        &RuntimeTurn {
            continuation_id: Some("thread".into()),
            turn_id: "turn".into(),
            decision: RuntimeDecision {
                mission_update: None,
                summary: "Found sensitive-123".into(),
                status: "WAITING".into(),
                current_objective: "Rotate sensitive-123".into(),
                next_step: "Ask for access".into(),
                blocker: "sensitive-123 expired".into(),
            },
            usage: None,
        },
        &secrets,
    );
    let event = format!("{event:?}");
    let turn = format!("{turn:?}");
    assert!(!event.contains("sensitive-123"));
    assert!(!turn.contains("sensitive-123"));
    assert!(event.contains("[REDACTED]"));
    assert!(turn.contains("[REDACTED]"));
    // Longer secrets win over the secrets they contain, and empty values never redact.
    assert_eq!(
        limina_engines::redact_text("a-b-c a-b", &["a-b", "a-b-c", ""]),
        "[REDACTED] [REDACTED]"
    );
}

#[tokio::test]
async fn default_factory_selects_and_hardens_the_claude_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let workspace_root = temp.path().join("workspaces");
    let state_root = workspace_root.parent().unwrap().to_path_buf();
    let config_root = temp.path().join("claude-state");
    let settings = EngineSettings::from_map(&map(&[
        ("LIMINA_CLAUDE_CONFIG_DIR", config_root.to_str().unwrap()),
        ("LIMINA_CLAUDE_WEAKER_NESTED_SANDBOX", "true"),
        ("LIMINA_CODEX_MODEL", "gpt-test"),
    ]))
    .unwrap();
    let runtime =
        CodexAuthManager::new(state_root.join("codex"), CodexAuthOptions::default()).await.unwrap();
    let accounts =
        Arc::new(ProjectCodexAccounts::new(state_root.join("accounts").join("codex"), "codex"));
    let factory = DefaultAgentFactory::new(
        settings,
        &state_root,
        Arc::new(CodexAuthRouter::new(runtime, accounts)),
    );

    let claude = factory.claude_session("retrieval");
    assert_eq!(claude.config().config_dir, config_root.join("retrieval"));
    assert_eq!(claude.config().state_root, state_root);
    assert!(claude.config().weaker_nested_sandbox);
    assert_eq!(claude.config().claude_bin, "claude");

    let factory = factory.into_factory();
    assert_eq!(factory("retrieval", Engine::Codex).unwrap().model().as_deref(), Some("gpt-test"));
    assert_eq!(factory("retrieval", Engine::ClaudeCode).unwrap().model(), None);
    // Unknown engines cannot reach the factory: they fail when the engine id is parsed.
    assert_eq!(Engine::parse("unknown").unwrap_err().code(), "invariant_violation");
    assert!(Path::new(&state_root).join("codex").is_dir());
}
