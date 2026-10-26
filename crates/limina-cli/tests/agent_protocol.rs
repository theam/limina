//! The hidden `_agent` protocol against a fake internal API: routes, headers, request bodies and
//! the exact bytes agents read back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::process::Command;

use serde_json::{Value, json};
use support::{FakeServer, Recorded, Reply, error_body, limina, ok, run};

/// What the fake internal API answers: a fixed artifact with nested values, floats and non-ASCII
/// text, so the output bytes exercise Python's `json.dumps` formatting.
fn artifact() -> Value {
    json!({
        "id": "H001",
        "kind": "H",
        "title": "Caché effect",
        "payload": {"statement": "Ünïcode \"quoted\"", "scores": [0.5, 1.0, 0.00001], "nested": {}},
        "hypothesis_id": null,
        "version": 1,
    })
}

const ARTIFACT_LINE: &str = "{\"id\": \"H001\", \"kind\": \"H\", \"title\": \"Caché effect\", \
    \"payload\": {\"statement\": \"Ünïcode \\\"quoted\\\"\", \"scores\": [0.5, 1.0, 1e-05], \
    \"nested\": {}}, \"hypothesis_id\": null, \"version\": 1}\n";

fn internal_api(request: &Recorded) -> Reply {
    if request.path.ends_with("/MISSING") {
        return Reply::Json(404, error_body("not_found", "Nothing here.", json!({"x": 1})));
    }
    match request.method.as_str() {
        "GET" if request.path.ends_with("/artifacts") => ok(json!([artifact()])),
        _ => Reply::Json(if request.method == "POST" { 201 } else { 200 }, artifact()),
    }
}

/// `limina _agent …` inside a managed turn.
fn agent(server: &FakeServer) -> Command {
    let mut command = limina();
    command
        .env("LIMINA_INTERNAL_URL", &server.url)
        .env("LIMINA_INTERNAL_TOKEN", "capability-123") // gitleaks:allow (synthetic capability for the local test server)
        .env("LIMINA_PROJECT", "demo")
        .env("LIMINA_ACTOR", "limina:runtime-1")
        .env("LIMINA_AGENT_LANE", "lane-2")
        .arg("_agent");
    command
}

fn single_request(server: &FakeServer) -> Recorded {
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests.into_iter().next().unwrap()
}

fn assert_capability(request: &Recorded) {
    assert_eq!(request.header("authorization"), Some("Bearer capability-123"));
    assert_eq!(request.header("x-limina-agent-lane"), Some("lane-2"));
}

fn assert_command(request: &Recorded) {
    assert_capability(request);
    assert_eq!(request.header("x-limina-actor"), Some("limina:runtime-1"));
    let key = request.header("idempotency-key").expect("an Idempotency-Key");
    assert_eq!(key.len(), 36, "{key}");
    assert_eq!(key.matches('-').count(), 4, "{key}");
    assert_eq!(request.header("content-type"), Some("application/json"));
}

#[test]
fn reads_use_the_project_scoped_routes() {
    let server = FakeServer::start(internal_api);
    let status = run(agent(&server).arg("status"));
    assert_eq!(status.code, 0, "{status:?}");
    assert_eq!(status.stdout, ARTIFACT_LINE);
    let request = single_request(&server);
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("GET", "/internal/v1/projects/demo/status")
    );
    assert_capability(&request);
    assert_eq!(request.header("idempotency-key"), None);
    assert_eq!(request.header("x-limina-actor"), None);
    assert!(request.body.is_empty());

    let server = FakeServer::start(internal_api);
    let list = run(agent(&server).args(["list", "--kind", "h"]));
    assert_eq!(list.code, 0, "{list:?}");
    assert_eq!(list.stdout, format!("[{}]\n", ARTIFACT_LINE.trim_end()));
    let request = single_request(&server);
    assert_eq!(request.path, "/internal/v1/projects/demo/artifacts");
    assert_eq!(request.query.as_deref(), Some("kind=h"));

    let server = FakeServer::start(internal_api);
    run(agent(&server).arg("list"));
    assert_eq!(single_request(&server).query, None);

    let server = FakeServer::start(internal_api);
    let show = run(agent(&server).args(["show", "e001"]));
    assert_eq!(show.code, 0, "{show:?}");
    assert_eq!(single_request(&server).path, "/internal/v1/projects/demo/artifacts/E001");
}

#[test]
fn writes_send_the_python_bodies_with_idempotency_keys() {
    let cases: Vec<(Vec<&str>, &str, Value)> = vec![
        (
            vec![
                "hypothesis",
                "add",
                "A claim",
                "--statement",
                "-3% when cached",
                "--test-plan",
                "Run it",
            ],
            "/internal/v1/projects/demo/hypotheses",
            json!({
                "title": "A claim",
                "statement": "-3% when cached",
                "mechanism": "",
                "generalization": "",
                "shortcut_risks": "",
                "test_plan": "Run it",
            }),
        ),
        (
            vec![
                "hypothesis",
                "decide",
                "h001",
                "--status",
                "CONFIRMED",
                "--conclusion",
                "Held.",
                "--expect",
                "3",
            ],
            "/internal/v1/projects/demo/hypotheses/H001/decision",
            json!({"status": "CONFIRMED", "conclusion": "Held.", "expected_version": 3}),
        ),
        (
            vec![
                "experiment",
                "add",
                "h001",
                "Probe",
                "--objective",
                "Measure",
                "--success",
                "p<0.05",
            ],
            "/internal/v1/projects/demo/experiments",
            json!({
                "hypothesis_id": "H001",
                "title": "Probe",
                "objective": "Measure",
                "procedure": "",
                "success_criteria": "p<0.05",
                "guardrails": "",
            }),
        ),
        (
            vec!["experiment", "claim", "e001"],
            "/internal/v1/projects/demo/experiments/E001/claim",
            json!({"ttl_seconds": 1800}),
        ),
        (
            vec!["experiment", "claim", "e002", "--ttl", "60"],
            "/internal/v1/projects/demo/experiments/E002/claim",
            json!({"ttl_seconds": 60}),
        ),
        (
            vec!["experiment", "observe", "e001", "Latency fell"],
            "/internal/v1/projects/demo/experiments/E001/observations",
            json!({"body": "Latency fell", "evidence_ref": null}),
        ),
        (
            vec!["experiment", "observe", "e001", "Latency fell", "--evidence", "runs/1.json"],
            "/internal/v1/projects/demo/experiments/E001/observations",
            json!({"body": "Latency fell", "evidence_ref": "runs/1.json"}),
        ),
        (
            vec![
                "experiment",
                "complete",
                "e001",
                "--results",
                "R",
                "--analysis",
                "A",
                "--decision",
                "D",
                "--expect",
                "2",
            ],
            "/internal/v1/projects/demo/experiments/E001/complete",
            json!({"results": "R", "analysis": "A", "decision": "D", "expected_version": 2}),
        ),
        (
            vec![
                "finding",
                "publish",
                "e001",
                "Faster",
                "--finding",
                "It is faster.",
                "--evidence",
                "E001 runs",
                "--next",
                "Ship it",
                "--debt",
                "Cold start",
            ],
            "/internal/v1/projects/demo/findings",
            json!({
                "experiment_id": "E001",
                "title": "Faster",
                "finding": "It is faster.",
                "evidence": "E001 runs",
                "improvement": "",
                "remaining_debt": "Cold start",
                "next_move": "Ship it",
                "impact": "HIGH",
            }),
        ),
    ];
    for (args, path, body) in cases {
        let server = FakeServer::start(internal_api);
        let result = run(agent(&server).args(&args));
        assert_eq!(result.code, 0, "{args:?}: {result:?}");
        assert_eq!(result.stdout, ARTIFACT_LINE, "{args:?}");
        let request = single_request(&server);
        assert_eq!((request.method.as_str(), request.path.as_str()), ("POST", path), "{args:?}");
        assert_command(&request);
        // Same keys, values and order as the Python client sent.
        assert_eq!(serde_json::to_string(&request.json()).unwrap(), body.to_string(), "{args:?}");
    }
}

#[test]
fn every_command_gets_a_fresh_idempotency_key() {
    let server = FakeServer::start(internal_api);
    run(agent(&server).args(["experiment", "claim", "E001"]));
    run(agent(&server).args(["experiment", "claim", "E001"]));
    let keys: Vec<String> = server
        .requests()
        .iter()
        .map(|request| request.header("idempotency-key").unwrap().to_owned())
        .collect();
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
}

#[test]
fn errors_keep_the_server_code_and_exit_code() {
    let server = FakeServer::start(internal_api);
    let result = run(agent(&server).args(["show", "missing"]));
    assert_eq!(result.code, 3);
    assert_eq!(result.stdout, "");
    assert_eq!(result.stderr, "not_found  Nothing here.\n");

    // `--json` is a root option, so it goes before `_agent`.
    let server = FakeServer::start(internal_api);
    let mut command = limina();
    command
        .env("LIMINA_INTERNAL_URL", &server.url)
        .env("LIMINA_INTERNAL_TOKEN", "capability-123") // gitleaks:allow (synthetic capability for the local test server)
        .env("LIMINA_PROJECT", "demo")
        .args(["--json", "_agent", "show", "missing"]);
    let result = run(&mut command);
    assert_eq!(result.code, 3);
    assert_eq!(
        result.stdout,
        "{\"error\": {\"code\": \"not_found\", \"message\": \"Nothing here.\", \"details\": {\"x\": 1}}}\n"
    );
}

#[test]
fn the_project_comes_from_the_environment() {
    let server = FakeServer::start(internal_api);
    let mut command = agent(&server);
    command.env_remove("LIMINA_PROJECT").env("LIMINA_CHALLENGE", "legacy");
    let result = run(command.arg("status"));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(single_request(&server).path, "/internal/v1/projects/legacy/status");

    let server = FakeServer::start(internal_api);
    let mut command = agent(&server);
    command.env_remove("LIMINA_PROJECT");
    let result = run(command.arg("status"));
    assert_eq!(result.code, 5);
    assert_eq!(
        result.stderr,
        "invariant_violation  The internal agent protocol requires LIMINA_PROJECT.\n\
         Try: Run this command from a Limina-managed project turn.\n"
    );
    assert!(server.requests().is_empty());
}

#[test]
fn agent_lane_is_optional() {
    let server = FakeServer::start(internal_api);
    let mut command = agent(&server);
    command.env_remove("LIMINA_AGENT_LANE");
    run(command.arg("status"));
    let request = single_request(&server);
    assert_eq!(request.header("x-limina-agent-lane"), None);
    assert_eq!(request.header("authorization"), Some("Bearer capability-123"));
}
