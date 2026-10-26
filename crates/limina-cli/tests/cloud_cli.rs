//! Port of the Python `tests/test_cloud_cli.py`. The Python tests replaced the HTTP client with
//! a fake; these run the real binary against a fake instance that answers like that fake did.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use limina_domain::project::NewChallenge;
use limina_domain::service::ChallengeService;
use limina_store::Db;
use serde_json::{Value, json};
use support::{FakeServer, Recorded, Reply, limina, ok, run};

/// The Python test's `FakePublicClient`: project creation and resource writes.
fn fake_public_client(request: &Recorded) -> Reply {
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/v1/projects") => {
            let payload = request.json();
            Reply::Json(
                201,
                json!({
                    "slug": payload["slug"],
                    "name": payload["name"],
                    "mission": payload["objective"],
                    "success_criteria": payload["success_criteria"],
                    "context": payload["context"],
                    "runtime": payload["runtime"],
                    "status": "CREATED",
                    "current_objective": payload["objective"],
                    "next_step": "Frame the first falsifiable hypothesis.",
                    "blocker": "None",
                    "created_at": "2026-01-01T00:00:00+00:00",
                    "updated_at": "2026-01-01T00:00:00+00:00",
                }),
            )
        }
        ("PUT", path) if path.contains("/resources/variables/") => {
            let name = path.rsplit('/').next().unwrap();
            ok(json!({
                "name": name,
                "type": "VARIABLE",
                "value": request.json()["value"],
                "configured": null,
                "status": "ACTIVE",
            }))
        }
        ("PUT", path) if path.contains("/resources/secrets/") => {
            let name = path.rsplit('/').next().unwrap();
            ok(json!({"name": name, "type": "SECRET", "configured": true, "status": "ACTIVE"}))
        }
        _ => Reply::Json(404, support::error_body("not_found", "No route.", json!({}))),
    }
}

#[test]
fn public_help_exposes_only_project_level_concepts() {
    let result = run(limina().arg("--help"));
    assert_eq!(result.code, 0, "{result:?}");
    for command in ["project", "review", "steer", "attach", "resource", "status"] {
        assert!(result.stdout.contains(command), "{command} missing: {}", result.stdout);
    }
    let lowered = result.stdout.to_lowercase();
    for internal in ["_agent", "worker", "coordinator", "thread", "lease", "_healthcheck"] {
        assert!(!lowered.contains(internal), "{internal} leaked: {}", result.stdout);
    }
}

#[test]
fn project_create_has_stable_json_output() {
    let server = FakeServer::start(fake_public_client);
    let result = run(limina().args([
        "--url",
        &server.url,
        "--json",
        "project",
        "create",
        "cli-test",
        "--mission",
        "Prove CLI composition.",
        "--success",
        "Project state is returned.",
    ]));
    assert_eq!(result.code, 0, "{result:?}");
    let value = result.json();
    assert_eq!(value["slug"], "cli-test");
    assert_eq!(value["status"], "CREATED");
    assert_eq!(value["runtime"], "codex");
    assert!(value.get("thread_id").is_none());

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        request.json(),
        json!({
            "slug": "cli-test",
            "name": "Cli Test",
            "objective": "Prove CLI composition.",
            "success_criteria": "Project state is returned.",
            "context": "",
            "runtime": "codex",
        })
    );
    assert_eq!(request.header("x-limina-actor"), Some("tester"));
    assert_eq!(request.header("idempotency-key").map(str::len), Some(36));
}

#[test]
fn project_create_accepts_the_claude_code_runtime() {
    let server = FakeServer::start(fake_public_client);
    let result = run(limina().args([
        "--url",
        &server.url,
        "--json",
        "project",
        "create",
        "claude-test",
        "--runtime",
        "claude-code",
        "--mission",
        "Prove engine selection.",
        "--success",
        "Claude Code is selected.",
    ]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(result.json()["runtime"], "claude-code");
}

#[test]
fn project_create_rejects_an_unknown_runtime_before_transport() {
    let server = FakeServer::start(fake_public_client);
    let result = run(limina().args([
        "--url",
        &server.url,
        "project",
        "create",
        "invalid-runtime",
        "--runtime",
        "other",
        "--mission",
        "Reject invalid selection.",
        "--success",
        "No request is sent.",
    ]));
    assert_ne!(result.code, 0);
    assert_eq!(result.code, 2);
    assert!(result.output().to_lowercase().contains("invalid value"), "{result:?}");
    assert!(result.output().contains("claude-code"), "{result:?}");
    assert!(server.requests().is_empty());
}

#[test]
fn resource_commands_make_secrets_write_only() {
    let secret_value = "cli-super-secret";
    let server = FakeServer::start(fake_public_client);

    let help = run(limina().args(["resource", "--help"]));
    let variable = run(limina().args([
        "--url",
        &server.url,
        "--json",
        "resource",
        "variable",
        "cli-test",
        "SOURCE_URL",
        "s3://data",
    ]));
    let secret = run(limina()
        .args([
            "--url",
            &server.url,
            "--json",
            "resource",
            "secret",
            "cli-test",
            "SOURCE_TOKEN",
            "--from-env",
            "TEST_SOURCE_TOKEN",
        ])
        .env("TEST_SOURCE_TOKEN", secret_value));

    assert_eq!(help.code, 0, "{help:?}");
    assert!(help.stdout.contains("variable"));
    assert!(help.stdout.contains("secret"));
    assert_eq!(variable.code, 0, "{variable:?}");
    assert_eq!(variable.json()["value"], "s3://data");
    assert_eq!(secret.code, 0, "{secret:?}");
    assert!(!secret.output().contains(secret_value));
    assert!(secret.json().get("value").is_none());

    let requests = server.requests();
    let stored = requests
        .iter()
        .find(|request| request.path == "/v1/projects/cli-test/resources/secrets/SOURCE_TOKEN")
        .expect("the secret was sent");
    assert_eq!(stored.method, "PUT");
    assert_eq!(stored.json(), json!({"value": secret_value}));
}

#[test]
fn hidden_agent_protocol_can_write_the_research_graph() {
    let dir = tempfile::tempdir().unwrap();
    let database_url = format!("sqlite:///{}", dir.path().join("cli.db").display());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let db = Db::connect(&database_url).await.unwrap();
        db.initialize().await.unwrap();
        let service = ChallengeService::new(db.clone(), None);
        service
            .create_challenge(
                NewChallenge {
                    slug: "agent-test".into(),
                    name: "Agent test".into(),
                    objective: "Validate the private protocol.".into(),
                    success_criteria: "Create a hypothesis.".into(),
                    ..NewChallenge::default()
                },
                "owner",
                &limina_core::new_uuid(),
            )
            .await
            .unwrap();
        db.close().await;
    });

    let result = run(limina()
        .args([
            "--database",
            &database_url,
            "--actor",
            "limina-runtime",
            "_agent",
            "hypothesis",
            "add",
            "Typed internal command",
            "--statement",
            "Typed commands preserve the H to E to F contract.",
        ])
        .env("LIMINA_PROJECT", "agent-test"));
    assert_eq!(result.code, 0, "{result:?}");
    let value: Value = result.json();
    assert_eq!(value["id"], "H001");
    assert_eq!(value["created_by"], "limina-runtime");
}
