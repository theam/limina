//! Claude sessions end to end over the CLI's stream-json and control protocol, against the fake
//! provider. Ports `test_claude_adapter_resumes_and_normalizes_the_sdk_contract` and
//! `test_claude_adapter_interrupts_and_redirects_an_active_turn` from
//! `tests/test_cloud_runtime.py`, plus permission requests, interrupts, failures and cleanup.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use limina_engines::claude::permissions::{
    OUTSIDE_WORKSPACE_DENIED, UNSANDBOXED_COMMAND_DENIED, resolve_lenient,
};
use limina_engines::{
    AgentSession, ClaudeAgentSession, ClaudeSessionConfig, TurnRequest, turn_output_schema,
};
use serde_json::{Value, json};
use support::{Recorder, eventually, fake_provider, process_alive, read_log, received, starts};

struct Fixture {
    temp: tempfile::TempDir,
    config_dir: PathBuf,
    workspace: PathBuf,
}

fn fixture(scenario: Value) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("claude").join("project");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(config_dir.join("fake-claude.json"), scenario.to_string()).unwrap();
    Fixture { temp, config_dir, workspace }
}

impl Fixture {
    fn session(&self, model: Option<&str>) -> Arc<ClaudeAgentSession> {
        Arc::new(ClaudeAgentSession::new(ClaudeSessionConfig {
            claude_bin: fake_provider(),
            model: model.map(str::to_owned),
            ..ClaudeSessionConfig::new(&self.config_dir, self.temp.path())
        }))
    }

    fn request(&self, continuation: Option<&str>, recorder: &Arc<Recorder>) -> TurnRequest {
        TurnRequest {
            settings: Default::default(),
            prompt: "Advance the mission.".to_owned(),
            workspace: self.workspace.clone(),
            continuation_id: continuation.map(str::to_owned),
            runtime_env: BTreeMap::from([
                ("LIMINA_INTERNAL_TOKEN".to_owned(), "capability".to_owned()),
                ("FAKE_LEAK".to_owned(), "sensitive-123".to_owned()),
            ]),
            mcp_servers: Vec::new(),
            observer: recorder.clone(),
        }
    }

    fn log(&self) -> Vec<Value> {
        read_log(&self.config_dir.join("fake-claude.log"))
    }

    fn start(&self) -> Value {
        starts(&self.log()).remove(0)
    }

    fn user_prompts(&self) -> Vec<String> {
        received(&self.log())
            .into_iter()
            .filter(|message| message["type"] == "user")
            .map(|message| message["message"]["content"].as_str().unwrap().to_owned())
            .collect()
    }

    fn control_requests(&self, subtype: &str) -> Vec<Value> {
        received(&self.log())
            .into_iter()
            .filter(|message| {
                message["type"] == "control_request" && message["request"]["subtype"] == subtype
            })
            .collect()
    }
}

fn argument<'a>(args: &'a [Value], flag: &str) -> &'a str {
    let position =
        args.iter().position(|arg| arg == flag).unwrap_or_else(|| panic!("{flag} missing"));
    args[position + 1].as_str().unwrap()
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn a_turn_resumes_and_normalizes_the_cli_stream() {
    let fixture = fixture(json!({}));
    let inside = fixture.workspace.join("inside.txt");
    let outside = fixture.temp.path().join("outside.txt");
    let scenario = json!({"scenario": "complete", "permissions": [
        {"tool_name": "Read", "input": {"file_path": inside.to_string_lossy()}},
        {"tool_name": "Read", "input": {"file_path": outside.to_string_lossy()}},
        {"tool_name": "Bash", "input": {"command": "curl x", "dangerouslyDisableSandbox": true}},
    ]});
    std::fs::write(fixture.config_dir.join("fake-claude.json"), scenario.to_string()).unwrap();
    let session = fixture.session(Some("claude-test"));
    let recorder = Recorder::new();

    let turn =
        session.run_turn(fixture.request(Some("previous-session"), &recorder)).await.unwrap();

    // The command line and environment the SDK would have produced.
    let start = fixture.start();
    let args = start["args"].as_array().unwrap();
    assert_eq!(argument(args, "--resume"), "previous-session");
    assert_eq!(argument(args, "--model"), "claude-test");
    assert_eq!(argument(args, "--permission-prompt-tool"), "stdio");
    assert_eq!(argument(args, "--permission-mode"), "default");
    assert_eq!(argument(args, "--tools"), "default");
    assert!(args.contains(&json!("--setting-sources=")));
    let schema: Value = serde_json::from_str(argument(args, "--json-schema")).unwrap();
    assert_eq!(schema, turn_output_schema());
    let settings: Value = serde_json::from_str(argument(args, "--settings")).unwrap();
    let sandbox = &settings["sandbox"];
    assert_eq!(settings["cleanupPeriodDays"], 3650);
    assert_eq!(sandbox["enabled"], true);
    assert_eq!(sandbox["failIfUnavailable"], true);
    assert_eq!(sandbox["allowUnsandboxedCommands"], false);
    assert_eq!(sandbox["enableWeakerNestedSandbox"], false);
    let workspace_root = resolve_lenient(&fixture.workspace);
    assert_eq!(sandbox["filesystem"]["allowRead"], json!([workspace_root.to_string_lossy()]));
    let state_root = resolve_lenient(fixture.temp.path()).to_string_lossy().into_owned();
    assert!(sandbox["filesystem"]["denyRead"].as_array().unwrap().contains(&json!(state_root)));
    let env = &start["env"];
    assert_eq!(env["LIMINA_INTERNAL_TOKEN"], "capability");
    assert_eq!(env["CLAUDE_CONFIG_DIR"], fixture.config_dir.to_string_lossy().as_ref());
    assert_eq!(env["CLAUDE_CODE_SUBPROCESS_ENV_SCRUB"], "1");
    assert!(env.get("CLAUDE_CODE_ENTRYPOINT").is_none(), "Limina does not pose as an official SDK");
    assert!(env["CLAUDE_AGENT_SDK_CLIENT_APP"].as_str().unwrap().starts_with("limina/"));
    assert_eq!(env["PWD"], fixture.workspace.to_string_lossy().as_ref());
    assert!(env.get("CARGO_MANIFEST_DIR").is_none(), "providers start from an empty environment");
    assert_eq!(
        Path::new(start["cwd"].as_str().unwrap()).canonicalize().unwrap(),
        fixture.workspace.canonicalize().unwrap()
    );

    // The private settings file that keeps sessions resumable.
    let settings_file = fixture.config_dir.join("limina-settings.json");
    assert_eq!(std::fs::read_to_string(&settings_file).unwrap(), "{\"cleanupPeriodDays\": 3650}\n");
    #[cfg(unix)]
    {
        assert_eq!(mode_of(&settings_file), 0o600);
        assert_eq!(mode_of(&fixture.config_dir), 0o700);
    }

    // The handshake, the prompt and the permission answers.
    let first = &received(&fixture.log())[0];
    assert_eq!(first["type"], "control_request");
    assert_eq!(first["request"], json!({"subtype": "initialize", "hooks": null}));
    assert!(first["request_id"].as_str().unwrap().starts_with("req_1_"));
    assert_eq!(fixture.user_prompts(), ["Advance the mission."]);
    let user =
        received(&fixture.log()).into_iter().find(|message| message["type"] == "user").unwrap();
    assert_eq!(
        user,
        json!({"type": "user", "message": {"role": "user", "content": "Advance the mission."},
               "parent_tool_use_id": null, "session_id": "default"})
    );
    let answers: Vec<Value> = received(&fixture.log())
        .into_iter()
        .filter(|message| message["type"] == "control_response")
        .map(|message| message["response"].clone())
        .collect();
    assert_eq!(answers.len(), 3);
    assert_eq!(answers[0]["subtype"], "success");
    assert_eq!(
        answers[0]["response"],
        json!({"behavior": "allow", "updatedInput": {"file_path": inside.to_string_lossy()}})
    );
    assert_eq!(
        answers[1]["response"],
        json!({"behavior": "deny", "message": OUTSIDE_WORKSPACE_DENIED})
    );
    assert_eq!(
        answers[2]["response"],
        json!({"behavior": "deny", "message": UNSANDBOXED_COMMAND_DENIED})
    );
    assert!(
        fixture.log().iter().any(|entry| entry.get("eof").is_some()),
        "the session closed stdin"
    );

    // The turn, its continuation and its transcript.
    assert!(!recorder.continuations().is_empty());
    assert!(recorder.continuations().iter().all(|id| id == "claude-session"));
    assert_eq!(turn.continuation_id.as_deref(), Some("claude-session"));
    assert_eq!(turn.turn_id, "message-123");
    assert_eq!(turn.decision.status, "COMPLETE");
    assert_eq!(turn.decision.summary, "Claude checkpointed the work.");
    let usage = turn.usage.unwrap();
    assert_eq!(
        (usage.input_tokens, usage.cached_input_tokens, usage.output_tokens),
        (Some(1200), Some(300), Some(80))
    );
    assert_eq!(
        (usage.cost_microusd, usage.cost_source.as_deref()),
        (Some(12_346), Some("provider"))
    );
    assert_eq!(recorder.kinds(), ["message", "tool", "tool", "status", "status", "status"]);
    let events = recorder.events();
    assert!(events.iter().all(|event| event.event_type == "runtime.claude-code"));
    assert_eq!(events[0].detail["text"], "Planning the benchmark run.");
    assert_eq!(events[1].summary, "Using Bash");
    assert_eq!(
        (&events[2].summary, &events[2].detail["state"]),
        (&"Bash finished".to_owned(), &json!("ok"))
    );
    assert_eq!(events[3].detail["parent_item_id"], "toolu_task");
    assert_eq!(events[5].detail["method"], "system/compact");
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(!serialized.contains("private reasoning"));
    assert!(!serialized.contains("SECRET-OUTPUT"));
    assert!(!serialized.contains("checkpointed the work"), "the structured output stays hidden");
}

#[tokio::test]
async fn steering_interrupts_and_redirects_the_active_query() {
    let fixture = fixture(json!({"scenario": "steer"}));
    let session = fixture.session(None);
    let recorder = Recorder::new();
    assert!(!session.steer("too early").await.unwrap());

    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();
    assert!(session.steer("Prefer the stronger baseline.").await.unwrap());
    let turn = running.await.unwrap().unwrap();

    assert_eq!(turn.decision.summary, "Steering incorporated.");
    let prompts = fixture.user_prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("Prefer the stronger baseline"));
    assert!(prompts[1].starts_with("Human steering arrived while you were working."));
    assert_eq!(fixture.control_requests("interrupt").len(), 1);
    let start = fixture.start();
    let args = start["args"].as_array().unwrap();
    assert_eq!(argument(args, "--model"), "default");
    assert_eq!(start["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "auto");
    assert!(!args.contains(&json!("--resume")));
}

#[tokio::test]
async fn interrupt_ends_the_turn_before_checkpointing() {
    let fixture = fixture(json!({"scenario": "interrupt"}));
    let session = fixture.session(None);
    let recorder = Recorder::new();
    assert!(!session.interrupt().await.unwrap(), "nothing to interrupt yet");
    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();

    assert!(session.interrupt().await.unwrap());
    assert!(!session.steer("too late").await.unwrap(), "no steering after a stop");
    let error = running.await.unwrap().unwrap_err();

    assert_eq!(error.code(), "transport_error");
    assert_eq!(error.message, "The managed Claude turn was interrupted before checkpointing.");
    assert_eq!(fixture.user_prompts().len(), 1);
}

#[tokio::test]
async fn error_results_and_crashes_fail_the_turn() {
    let fixture = self::fixture(json!({"scenario": "error-result"}));
    let error =
        fixture.session(None).run_turn(fixture.request(None, &Recorder::new())).await.unwrap_err();
    assert_eq!(
        error.message,
        "The managed Claude turn failed before Limina accepted its checkpoint."
    );
    assert_eq!(error.details["stop_reason"], "max_tokens");

    let fixture = self::fixture(json!({"scenario": "exit"}));
    let error = fixture
        .session(None)
        .run_turn(fixture.request(Some("s-1"), &Recorder::new()))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "The managed Claude turn failed before Limina accepted its checkpoint."
    );
    let reason = error.details["reason"].as_str().unwrap();
    assert!(reason.starts_with("Command failed with exit code 2 (exit code: 2)"), "{reason}");
    assert!(reason.contains("exited on purpose [REDACTED]"), "{reason}");
    assert!(!reason.contains("sensitive-123"));
    assert_eq!(error.details["continuation_id"], "s-1");

    let missing = self::fixture(json!({}));
    let session = ClaudeAgentSession::new(ClaudeSessionConfig {
        claude_bin: "/nonexistent/claude".to_owned(),
        ..ClaudeSessionConfig::new(&missing.config_dir, missing.temp.path())
    });
    let error = session.run_turn(missing.request(None, &Recorder::new())).await.unwrap_err();
    assert_eq!(
        error.message,
        "The managed Claude turn failed before Limina accepted its checkpoint."
    );
    assert!(
        error.details["reason"]
            .as_str()
            .unwrap()
            .starts_with("Claude was not found at: /nonexistent/claude")
    );
}

#[tokio::test]
async fn the_cli_is_killed_when_a_turn_is_dropped_or_the_session_closes() {
    let fixture = fixture(json!({"scenario": "hang"}));
    let session = fixture.session(None);

    let recorder = Recorder::new();
    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();
    let pid = fixture.start()["pid"].as_u64().unwrap();
    assert!(process_alive(pid));
    running.abort();
    assert!(eventually(|| !process_alive(pid)).await, "dropping the turn kills the CLI");
    assert!(!session.steer("nobody is listening").await.unwrap());
    assert!(!session.interrupt().await.unwrap());

    std::fs::remove_file(fixture.config_dir.join("fake-claude.log")).unwrap();
    let recorder = Recorder::new();
    let first_event = recorder.first_event();
    let running = tokio::spawn({
        let session = Arc::clone(&session);
        let request = fixture.request(None, &recorder);
        async move { session.run_turn(request).await }
    });
    first_event.await.unwrap();
    let pid = fixture.start()["pid"].as_u64().unwrap();
    session.close().await;
    let error = running.await.unwrap().unwrap_err();
    assert_eq!(error.message, "The managed Claude turn was interrupted before checkpointing.");
    assert!(eventually(|| !process_alive(pid)).await, "closing the session ends the CLI");
}

#[tokio::test]
async fn project_defaults_override_claude_model_and_effort_on_every_launch() {
    let fixture = fixture(json!({"scenario":"complete"}));
    let session = fixture.session(Some("operator-model"));
    let recorder = Recorder::new();
    for (model, effort) in [("opus", "max"), ("sonnet", "low")] {
        let mut request = fixture.request(Some("previous-session"), &recorder);
        request.settings.model = Some(model.into());
        request.settings.reasoning_effort = Some(effort.into());
        session.run_turn(request).await.unwrap();
    }
    let log = fixture.log();
    let launches = starts(&log);
    for (launch, (model, effort)) in launches.iter().zip([("opus", "max"), ("sonnet", "low")]) {
        let args = launch["args"].as_array().unwrap();
        assert_eq!(argument(args, "--model"), model);
        assert_eq!(argument(args, "--effort"), effort);
        assert_eq!(argument(args, "--resume"), "previous-session");
        assert_eq!(launch["env"]["CLAUDE_CODE_EFFORT_LEVEL"], effort);
    }
    assert_eq!(launches.len(), 2);
    session.run_turn(fixture.request(Some("previous-session"), &recorder)).await.unwrap();
    let log = fixture.log();
    let launches = starts(&log);
    let reset = &launches[2];
    assert_eq!(argument(reset["args"].as_array().unwrap(), "--model"), "operator-model");
    assert_eq!(reset["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "auto");
    let account_default = fixture.session(None);
    account_default.run_turn(fixture.request(Some("previous-session"), &recorder)).await.unwrap();
    let log = fixture.log();
    let launches = starts(&log);
    assert_eq!(argument(launches[3]["args"].as_array().unwrap(), "--model"), "default");
    assert_eq!(launches[3]["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "auto");
}

#[tokio::test]
async fn discovery_reads_live_per_model_capabilities_without_a_user_turn() {
    let fixture = fixture(json!({"models":[
        {"value":"new-model", "displayName":"New model", "resolvedModel":"new-release", "supportedEffortLevels":["low","future"]},
        {"value":"fast-model", "displayName":"Fast model"}
    ]}));
    let session = fixture.session(None);
    let catalog = session.models(&BTreeMap::new()).await.unwrap();
    assert_eq!(catalog.models[0].id, "new-model");
    assert_eq!(catalog.models[0].resolved_model.as_deref(), Some("new-release"));
    assert_eq!(catalog.models[0].reasoning_efforts, ["low", "future"]);
    assert!(catalog.models[1].reasoning_efforts.is_empty());
    assert!(fixture.user_prompts().is_empty(), "metadata must never run a generation");
    std::fs::write(
        fixture.config_dir.join("fake-claude.json"),
        json!({"models":[{"value":"next-release", "supportedEffortLevels":["adaptive"]}]})
            .to_string(),
    )
    .unwrap();
    let refreshed = session.models(&BTreeMap::new()).await.unwrap();
    assert_eq!(refreshed.models[0].id, "next-release");
    assert_eq!(refreshed.models[0].reasoning_efforts, ["adaptive"]);
    assert_eq!(starts(&fixture.log()).len(), 2);
    assert!(fixture.user_prompts().is_empty());
}
