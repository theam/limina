//! Error responses map back to the same error codes and exit codes the Python CLI reported.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use serde_json::json;
use support::{FakeServer, Reply, error_body, limina, run};

fn failing(status: u16, code: &'static str) -> FakeServer {
    FakeServer::start(move |_| {
        Reply::Json(
            status,
            error_body(
                code,
                "It failed.",
                json!({"project": "demo", "suggestion": "Try something else."}),
            ),
        )
    })
}

#[test]
fn error_codes_become_exit_codes() {
    let cases = [
        (404, "not_found", "not_found", 3),
        (409, "version_conflict", "version_conflict", 4),
        (422, "invariant_violation", "invariant_violation", 5),
        (409, "lease_conflict", "lease_conflict", 6),
        (401, "authentication_required", "authentication_required", 7),
        (403, "permission_denied", "permission_denied", 7),
        (429, "rate_limited", "rate_limited", 9),
        (503, "transport_error", "transport_error", 8),
        (422, "invalid_request", "transport_error", 8),
        (503, "database_error", "transport_error", 8),
    ];
    for (status, code, reported, exit_code) in cases {
        let server = failing(status, code);
        let human = run(limina().args(["--url", &server.url, "project", "show", "demo"]));
        assert_eq!(human.code, exit_code, "{code}: {human:?}");
        assert_eq!(human.stdout, "", "{code}");
        assert_eq!(
            human.stderr,
            format!("{reported}  It failed.\nTry: Try something else.\n"),
            "{code}"
        );

        let machine =
            run(limina().args(["--url", &server.url, "--json", "project", "show", "demo"]));
        assert_eq!(machine.code, exit_code, "{code}");
        let body = machine.json();
        assert_eq!(body["error"]["code"], reported, "{code}");
        assert_eq!(body["error"]["message"], "It failed.");
        assert_eq!(body["error"]["details"]["project"], "demo");
        if reported == "transport_error" {
            assert_eq!(body["error"]["details"]["status"], status, "{code}");
        }
    }
}

#[test]
fn json_errors_use_python_formatting() {
    let server = FakeServer::start(|_| {
        Reply::Json(
            404,
            error_body("not_found", "Project 'demo' does not exist.", json!({"slug": "demo"})),
        )
    });
    let result = run(limina().args(["--url", &server.url, "--json", "status", "demo"]));
    assert_eq!(result.code, 3);
    assert_eq!(
        result.stdout,
        "{\"error\": {\"code\": \"not_found\", \"message\": \"Project 'demo' does not exist.\", \
         \"details\": {\"slug\": \"demo\"}}}\n"
    );
    assert_eq!(result.stderr, "");
}

#[test]
fn responses_without_an_error_body_are_transport_errors() {
    let server = FakeServer::start(|_| Reply::Text(502, "<html>Bad gateway</html>".into()));
    let result = run(limina().args(["--url", &server.url, "--json", "project", "list"]));
    assert_eq!(result.code, 8);
    assert_eq!(
        result.json()["error"]["message"],
        "Runtime returned HTTP 502 without a valid error body."
    );

    let server = FakeServer::start(|_| Reply::Json(500, json!({"detail": "boom"})));
    let result = run(limina().args(["--url", &server.url, "--json", "project", "list"]));
    assert_eq!(result.code, 8);
    assert_eq!(result.json()["error"]["message"], "Runtime returned HTTP 500.");
    assert_eq!(result.json()["error"]["details"], json!({"status": 500}));
}

#[test]
fn an_unreachable_instance_is_a_transport_error() {
    let address = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let url = format!("http://{address}");
    let result = run(limina().args(["--url", &url, "--json", "doctor"]));
    assert_eq!(result.code, 8, "{result:?}");
    let error = &result.json()["error"];
    assert_eq!(error["code"], "transport_error");
    assert_eq!(error["message"], format!("Cannot reach Limina runtime at {url}."));
    assert!(error["details"]["reason"].as_str().unwrap().len() > 3);

    let result = run(limina().args(["--url", &format!("{url}/"), "doctor"]));
    assert_eq!(result.code, 8);
    assert_eq!(
        result.stderr.lines().next().unwrap(),
        format!("transport_error  Cannot reach Limina runtime at {url}.")
    );
}

#[test]
fn usage_errors_exit_with_two_before_any_request() {
    let server = FakeServer::start(|_| Reply::Json(200, json!({})));
    let cases: [&[&str]; 5] = [
        &["resource", "secret", "demo", "TOKEN", "--from-env", "A", "--from-stdin"],
        &["resource", "secret", "demo", "TOKEN", "--from-env", "LIMINA_TEST_UNSET_VARIABLE"],
        &["--json", "attach", "demo"],
        &["runtime", "codex", "login", "--method", "password"],
        &["project", "create", "demo", "--runtime", "gemini", "-m", "M", "-s", "S"],
    ];
    for args in cases {
        let result = run(limina().args(["--url", &server.url]).args(args));
        assert_eq!(result.code, 2, "{args:?}: {result:?}");
        assert!(result.stderr.starts_with("error: "), "{args:?}: {}", result.stderr);
    }
    let result = run(limina().args([
        "resource",
        "secret",
        "demo",
        "TOKEN",
        "--from-env",
        "LIMINA_TEST_UNSET_VARIABLE",
    ]));
    assert!(
        result.stderr.contains("environment variable 'LIMINA_TEST_UNSET_VARIABLE' is not set"),
        "{}",
        result.stderr
    );
    assert!(server.requests().is_empty());

    let missing = run(limina().arg("project"));
    assert_eq!(missing.code, 2);
    let missing = run(&mut limina());
    assert_eq!(missing.code, 2);
    assert!(missing.stderr.contains("Usage: limina"));
}
