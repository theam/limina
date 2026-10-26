//! The public commands against a fake instance: routes, headers, bodies and output.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use support::{FakeServer, Recorded, Reply, error_body, limina, ok, run, run_with_stdin};

fn project(slug: &str, status: &str) -> Value {
    json!({
        "slug": slug,
        "name": "Demo",
        "mission": "Find the cause.",
        "success_criteria": "A finding.",
        "context": "",
        "runtime": "claude-code",
        "status": status,
        "current_objective": "Frame it.",
        "next_step": "Write H001.",
        "blocker": "",
        "created_at": "2026-01-01T00:00:00+00:00",
        "updated_at": "2026-01-01T00:00:00+00:00",
        "role": "OWNER",
    })
}

fn not_found() -> Reply {
    Reply::Json(404, error_body("not_found", "No route.", json!({})))
}

#[test]
fn no_color_environment_accepts_shell_presence_values() {
    for value in ["1", "true", "enabled", "0", "false", ""] {
        let result = run(limina().env("NO_COLOR", value).arg("--version"));
        assert_eq!(result.code, 0, "NO_COLOR={value:?}: {result:?}");
        assert!(!result.output().contains('\u{1b}'));
    }
}

#[test]
fn lifecycle_commands_post_actions_with_command_headers() {
    for (args, action) in [
        (vec!["start", "demo"], "start"),
        (vec!["pause", "demo"], "pause"),
        (vec!["resume", "demo"], "resume"),
        (vec!["stop", "demo"], "stop"),
        (vec!["project", "archive", "demo"], "archive"),
    ] {
        let server = FakeServer::start(|_| ok(project("demo", "RUNNING")));
        let result = run(limina().args(["--url", &server.url, "--actor", "ana"]).args(&args));
        assert_eq!(result.code, 0, "{result:?}");
        assert!(result.stdout.starts_with("demo\n"), "{}", result.stdout);
        assert!(result.stdout.contains("Engine     Claude"));
        assert!(result.stdout.contains("Status     RUNNING"));
        let requests = server.requests();
        let request = &requests[0];
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, format!("/v1/projects/demo/actions/{action}"));
        assert_eq!(request.body, b"{}");
        assert_eq!(request.header("x-limina-actor"), Some("ana"));
        assert!(request.header("idempotency-key").is_some());
        assert_eq!(request.header("authorization"), None);
    }
}

#[test]
fn tokens_authenticate_and_admin_tokens_take_precedence_for_runtime_commands() {
    let server = FakeServer::start(|request| {
        if request.path.starts_with("/v1/runtime") {
            ok(json!({"engine": "codex", "configured_mode": "auto", "configured": false,
                      "active_method": null, "account_email": null, "source": "none"}))
        } else {
            ok(project("demo", "CREATED"))
        }
    });
    run(limina().args(["--url", &server.url, "--token", "public-t", "project", "show", "demo"]));
    run(limina()
        .args(["--url", &server.url, "runtime", "codex", "status"])
        .env("LIMINA_API_TOKEN", "public-t")
        .env("LIMINA_ADMIN_API_TOKEN", "admin-t"));
    run(limina().args(["--url", &server.url, "--token", "public-t", "runtime", "codex", "logout"]));
    let requests = server.requests();
    assert_eq!(requests[0].header("authorization"), Some("Bearer public-t"));
    assert_eq!(requests[1].path, "/v1/runtime/engines/codex/auth");
    assert_eq!(requests[1].header("authorization"), Some("Bearer admin-t"));
    assert_eq!(
        (requests[2].method.as_str(), requests[2].header("authorization")),
        ("DELETE", Some("Bearer public-t"))
    );
}

#[test]
fn project_list_follows_cursors() {
    let server = FakeServer::start(|request| {
        let query = request.query_pairs();
        match query.get("cursor").map(String::as_str) {
            None => {
                ok(json!({"items": [project("a", "CREATED")], "next_cursor": "c2", "total": 2}))
            }
            Some("c2") => {
                ok(json!({"items": [project("b", "PAUSED")], "next_cursor": null, "total": 2}))
            }
            Some(_) => not_found(),
        }
    });
    let result = run(limina().args(["--url", &server.url, "--json", "project", "list", "--all"]));
    assert_eq!(result.code, 0, "{result:?}");
    let projects = result.json();
    let slugs: Vec<&str> =
        projects.as_array().unwrap().iter().map(|p| p["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["a", "b"]);
    let requests = server.requests();
    assert_eq!(requests[0].query.as_deref(), Some("include_archived=true&limit=200"));
    assert_eq!(requests[1].query.as_deref(), Some("include_archived=true&limit=200&cursor=c2"));

    let human = run(limina().args(["--url", &server.url, "project", "list"]));
    assert!(
        human.stdout.starts_with("Limina projects\nProject  Engine  Status"),
        "{}",
        human.stdout
    );
    assert!(human.stdout.contains("a        Claude  CREATED  Find the cause.  Write H001."));
}

#[test]
fn status_and_review_render_knowledge() {
    let server = FakeServer::start(|request| match request.path.as_str() {
        "/v1/projects/demo/status" => ok(json!({
            "project": project("demo", "RUNNING"),
            "knowledge": {"H": {"TESTING": 1, "PROPOSED": 2}, "E": {"RUNNING": 1}},
            "active_work": [],
            "pending_guidance": 3,
            "event_cursor": 9,
        })),
        "/v1/projects/demo/review" => ok(json!({
            "project": project("demo", "RUNNING"),
            "findings": [],
            "experiments": [{"id": "E001", "status": "RUNNING", "title": "Probe"}],
            "hypotheses": [{"id": "H001", "status": "TESTING", "title": "Claim"}],
        })),
        "/v1/projects/demo/knowledge/H001" => ok(json!({
            "id": "H001", "kind": "H", "title": "Claim", "status": "TESTING",
            "content": {"statement": "It holds.", "test_plan": "", "shortcut_risks": "Few."},
        })),
        _ => not_found(),
    });
    let status = run(limina().args(["--url", &server.url, "status", "demo"]));
    assert_eq!(status.code, 0, "{status:?}");
    assert!(status.stdout.contains("Knowledge\nKind  State     Count\n"), "{}", status.stdout);
    assert!(status.stdout.contains("E     RUNNING   1\nH     PROPOSED  2\nH     TESTING   1\n"));
    assert!(status.stdout.ends_with("Pending team guidance: 3\n"));

    let review = run(limina().args(["--url", &server.url, "review", "demo"]));
    assert!(review.stdout.contains("Experiments\nID    Status   Title\n"), "{}", review.stdout);
    assert!(!review.stdout.contains("Findings"));

    let artifact = run(limina().args(["--url", &server.url, "review", "demo", "-a", "h001"]));
    assert_eq!(artifact.code, 0, "{artifact:?}");
    assert!(artifact.stdout.starts_with("H001\n"), "{}", artifact.stdout);
    assert!(artifact.stdout.contains("\nStatement\nIt holds.\n"));
    assert!(artifact.stdout.contains("\nShortcut Risks\nFew.\n"));
    assert!(!artifact.stdout.contains("Test Plan"));

    let json_status = run(limina().args(["--url", &server.url, "--json", "status", "demo"]));
    assert!(json_status.stdout.starts_with("{\n  \"project\": {\n    \"slug\": \"demo\","));
}

#[test]
fn steer_sends_guidance() {
    let server = FakeServer::start(|_| {
        Reply::Json(
            202,
            json!({"id": "g1", "delivery": "LIVE", "kind": "ANSWER", "accepted_at": "x", "status": "PENDING"}),
        )
    });
    let result = run(limina().args([
        "--url",
        &server.url,
        "steer",
        "demo",
        "Use the staging data.",
        "--kind",
        "ANSWER",
    ]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(result.stdout, "Live — Limina accepted your guidance.\n");
    let request = &server.requests()[0];
    assert_eq!(request.path, "/v1/projects/demo/steering");
    assert_eq!(request.json(), json!({"body": "Use the staging data.", "kind": "ANSWER"}));
}

#[test]
fn watch_prints_events_from_the_cursor() {
    let server = FakeServer::start(|request| {
        let after: i64 = request.query_pairs()["after"].parse().unwrap();
        ok(json!({
            "events": [{
                "sequence": after + 1,
                "type": "hypothesis.created",
                "actor": "Limina",
                "artifact_id": "H001",
                "detail": {"title": "Claim", "ratio": 0.5},
                "created_at": "2026-01-01T10:00:00.000001+00:00",
            }],
            "cursor": after + 1,
        }))
    });
    let human =
        run(limina().args(["--url", &server.url, "watch", "demo", "--after", "4", "--no-follow"]));
    assert_eq!(human.code, 0, "{human:?}");
    assert_eq!(human.stdout, "    5 2026-01-01T10:00:00 hypothesis.created H001 by Limina Claim\n");
    let request = &server.requests()[0];
    assert_eq!(request.path, "/v1/projects/demo/events");
    assert_eq!(request.query.as_deref(), Some("after=4&limit=200"));

    let machine =
        run(limina().args(["--url", &server.url, "--json", "watch", "demo", "--no-follow"]));
    assert_eq!(
        machine.stdout,
        "{\"sequence\": 1, \"type\": \"hypothesis.created\", \"actor\": \"Limina\", \
         \"artifact_id\": \"H001\", \"detail\": {\"title\": \"Claim\", \"ratio\": 0.5}, \
         \"created_at\": \"2026-01-01T10:00:00.000001+00:00\"}\n"
    );
}

#[test]
fn watch_follows_until_an_error_stops_it() {
    let calls = Arc::new(Mutex::new(0));
    let counter = Arc::clone(&calls);
    let server = FakeServer::start(move |_| {
        let mut calls = counter.lock().unwrap();
        *calls += 1;
        if *calls < 3 {
            ok(json!({"events": [], "cursor": 0}))
        } else {
            Reply::Json(404, error_body("not_found", "Project 'demo' does not exist.", json!({})))
        }
    });
    let result = run(limina().args(["--url", &server.url, "watch", "demo", "--interval", "0.01"]));
    assert_eq!(result.code, 3, "{result:?}");
    assert_eq!(*calls.lock().unwrap(), 3);
}

#[test]
fn export_writes_the_snapshot_into_an_empty_directory() {
    let server = FakeServer::start(|request| {
        if request.path == "/v1/projects/demo/snapshot" {
            ok(
                json!({"files": {"research/hypotheses/H001-x.md": "# H001\n", "ACTIVE.md": "# Active\n"}}),
            )
        } else {
            not_found()
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("kb");
    let result = run(limina().current_dir(dir.path()).args([
        "--url",
        &server.url,
        "--json",
        "export",
        "demo",
        "kb",
    ]));
    assert_eq!(result.code, 0, "{result:?}");
    let value = result.json();
    assert_eq!(value["file_count"], 2);
    assert_eq!(value["target"], std::fs::canonicalize(&target).unwrap().display().to_string());
    assert_eq!(std::fs::read_to_string(target.join("ACTIVE.md")).unwrap(), "# Active\n");

    let again =
        run(limina().current_dir(dir.path()).args(["--url", &server.url, "export", "demo", "kb"]));
    assert_eq!(again.code, 4, "{again:?}");
    assert_eq!(
        again.stderr,
        "version_conflict  Export target 'kb' is not empty.\n\
         Try: Choose an empty directory so stale artifacts cannot survive the export.\n"
    );
}

#[test]
fn resources_list_hide_secrets_and_remove_by_name() {
    let server = FakeServer::start(|request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/v1/projects/demo/resources") => ok(json!([
                {"name": "SOURCE_URL", "type": "VARIABLE", "status": "ACTIVE", "value": "s3://data"},
                {"name": "TOKEN", "type": "SECRET", "status": "ACTIVE", "configured": true},
                {"name": "OLD", "type": "SECRET", "status": "ACTIVE", "configured": false},
            ])),
            ("DELETE", "/v1/projects/demo/resources/TOKEN") => {
                ok(json!({"name": "TOKEN", "type": "SECRET", "status": "REMOVED"}))
            }
            _ => not_found(),
        }
    });
    let list = run(limina().args(["--url", &server.url, "resource", "list", "demo"]));
    assert_eq!(list.code, 0, "{list:?}");
    assert!(list.stdout.contains("SOURCE_URL  VARIABLE  s3://data\n"), "{}", list.stdout);
    assert!(list.stdout.contains("TOKEN       SECRET    configured · hidden\n"));
    assert!(list.stdout.contains("OLD         SECRET    unavailable\n"));

    let removed = run(limina().args(["--url", &server.url, "resource", "remove", "demo", "TOKEN"]));
    assert_eq!(removed.stdout, "Removed TOKEN\n");
    let request = server.requests().into_iter().find(|r| r.method == "DELETE").unwrap();
    assert!(request.body.is_empty());
    assert!(request.header("idempotency-key").is_some());
}

#[test]
fn secrets_can_come_from_standard_input() {
    let server = FakeServer::start(|request: &Recorded| {
        ok(
            json!({"name": "TOKEN", "type": "SECRET", "configured": true, "status": "ACTIVE", "echo": request.json()["value"]}),
        )
    });
    let result = run_with_stdin(
        limina().args([
            "--url",
            &server.url,
            "resource",
            "secret",
            "demo",
            "TOKEN",
            "--from-stdin",
        ]),
        "multi\nline-secret\r\n",
    );
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(result.stdout, "Stored secret TOKEN · value hidden\n");
    assert_eq!(server.requests()[0].json(), json!({"value": "multi\nline-secret"}));
}

#[test]
fn project_create_prompts_for_missing_text() {
    let server = FakeServer::start(|request: &Recorded| {
        Reply::Json(201, {
            let payload = request.json();
            let mut created = project(payload["slug"].as_str().unwrap(), "CREATED");
            created["mission"] = payload["objective"].clone();
            created
        })
    });
    let result = run_with_stdin(
        limina().args(["--url", &server.url, "--json", "project", "create", "my-2nd-project"]),
        "\nThe mission.\nThe criteria.\n",
    );
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(result.json()["mission"], "The mission.");
    assert!(result.stderr.contains("Mission: "));
    let body = server.requests()[0].json();
    assert_eq!(body["name"], "My 2Nd Project");
    assert_eq!(body["success_criteria"], "The criteria.");

    let aborted = run(limina().args(["--url", &server.url, "project", "create", "x"]));
    assert_eq!(aborted.code, 1, "{aborted:?}");
    assert!(aborted.stderr.ends_with("Aborted!\n"));
}

#[test]
fn doctor_reports_the_instance() {
    let server = FakeServer::start(|request| {
        assert_eq!(request.path, "/healthz");
        ok(json!({"ok": true, "version": "0.2.0", "runtime_owner": "limina", "auth_mode": "local",
                  "runtimes": ["codex", "claude-code"], "interfaces": {"rest": "/v1", "mcp": "/mcp/"}}))
    });
    let result = run(limina().args(["--url", &server.url, "doctor"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(
        result.stdout,
        format!("ok {} · runtime owned by limina · engines Codex, Claude\n", server.url)
    );
}

#[test]
fn codex_device_login_polls_until_it_completes() {
    let polls = Arc::new(Mutex::new(0));
    let counter = Arc::clone(&polls);
    let server = FakeServer::start(move |request| {
        let attempt = |status: &str, error: Value| {
            json!({"login_id": "l-1", "status": status, "verification_url": "https://auth.example/device",
                   "user_code": "ABCD-1234", "error": error, "created_at": "x", "completed_at": null})
        };
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/v1/runtime/engines/codex/auth/login") => ok(attempt("PENDING", Value::Null)),
            ("GET", "/v1/runtime/engines/codex/auth/login/l-1") => {
                let mut polls = counter.lock().unwrap();
                *polls += 1;
                if *polls < 2 {
                    ok(attempt("PENDING", Value::Null))
                } else {
                    ok(attempt("FAILED", json!("Expired.")))
                }
            }
            _ => not_found(),
        }
    });
    let result = run(limina().args([
        "--url",
        &server.url,
        "runtime",
        "codex",
        "login",
        "--method",
        " ChatGPT ",
    ]));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(result.stdout.contains("Open https://auth.example/device\nEnter code ABCD-1234\n"));
    assert!(result.stdout.ends_with("failed\n"), "{}", result.stdout);
    assert_eq!(result.stderr, "Expired.\n");
    let login = &server.requests()[0];
    assert_eq!(login.json(), json!({"method": "chatgpt"}));
    assert!(login.header("idempotency-key").is_some());
    assert_eq!(login.header("x-limina-actor"), None);

    // With --json the device login is printed, not polled.
    let machine = run(limina().args(["--url", &server.url, "--json", "runtime", "codex", "login"]));
    assert_eq!(machine.json()["status"], "PENDING");
}

#[test]
fn codex_machine_logins_print_the_status() {
    let server = FakeServer::start(|_| {
        ok(json!({"engine": "codex", "configured_mode": "api-key", "configured": true,
                  "active_method": "api-key", "account_email": null, "source": "environment"}))
    });
    let result = run(limina().args([
        "--url",
        &server.url,
        "runtime",
        "codex",
        "login",
        "--method",
        "api-key",
    ]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(
        result.stdout,
        "Codex authentication\n────────────────────\nMethod  api-key\nSource  environment\n\
         Account not disclosed\nMode    api-key\n"
    );
}

#[test]
fn healthcheck_exits_zero_only_for_success() {
    let healthy = FakeServer::start(|_| ok(json!({"ok": true})));
    let result = run(limina().args(["_healthcheck", "--url", &format!("{}/readyz", healthy.url)]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(healthy.requests()[0].path, "/readyz");

    let unhealthy = FakeServer::start(|_| Reply::Json(503, json!({"ok": false})));
    let result =
        run(limina().args(["_healthcheck", "--url", &format!("{}/readyz", unhealthy.url)]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(result.stderr.starts_with("unhealthy: "));

    let address = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let result = run(limina().args(["_healthcheck", "--url", &format!("http://{address}/readyz")]));
    assert_eq!(result.code, 1, "{result:?}");
}

#[test]
fn serve_validates_the_bind_and_its_configuration() {
    let open = run(limina().args(["serve", "--host", "0.0.0.0"]));
    assert_eq!(open.code, 2, "{open:?}");
    assert!(open.stderr.contains("a shared token, WorkOS, or OIDC configuration is required"));

    // Without a token or the explicit open mode, nothing starts (Python's create_app refused).
    let local = run(limina().args(["--json", "serve", "--port", "7999"]));
    assert_eq!(local.code, 1, "{local:?}");
    assert_eq!(
        local.json()["error"]["message"],
        "Configure LIMINA_API_TOKEN for local development or LIMINA_OIDC_ISSUER and \
         LIMINA_OIDC_AUDIENCE for team deployment."
    );

    // A token passes the bind check; an unreachable database stops the start (exit 8).
    let tokened = run(limina()
        .args(["serve", "--host", "0.0.0.0", "--database-url", "unsupported://nowhere"])
        .env("LIMINA_API_TOKEN", "t"));
    assert_eq!(tokened.code, 8, "{tokened:?}");
    assert!(tokened.stderr.contains("The runtime database is unavailable."));
}

/// `limina serve` runs a working instance and stops cleanly on SIGTERM.
#[cfg(unix)]
#[test]
fn serve_runs_until_terminated() {
    use std::io::{Read, Write};

    let dir = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let database = format!("sqlite:///{}", dir.path().join("server.db").display());
    let workspaces = dir.path().join("workspaces");
    let child = limina()
        .args(["serve", "--port", &port.to_string(), "--database-url", &database])
        .arg("--workspace-root")
        .arg(&workspaces)
        .env("LIMINA_ALLOW_INSECURE_NO_AUTH", "1")
        .env("RUST_LOG", "warn")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let get = |path: &str| -> Option<String> {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
        write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        Some(response)
    };
    let mut ready = None;
    for _ in 0..300 {
        ready = get("/readyz");
        if ready.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let ready = ready.expect("the server never answered");
    assert!(ready.starts_with("HTTP/1.1 200"), "{ready}");
    assert!(get("/auth/config").unwrap().ends_with(r#"{"mode":"open"}"#));
    let status = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(dir.path().join("server.db").exists());
}

mod live {
    use super::*;
    use axum::Router;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use axum::routing::get;

    #[derive(Default)]
    struct Seen {
        headers: Vec<(String, String)>,
        path_ok: bool,
        messages: Vec<String>,
    }

    async fn session(mut socket: WebSocket, seen: Arc<Mutex<Seen>>) {
        let snapshot =
            json!({"type": "snapshot", "value": {"project": project("demo", "RUNNING")}});
        let _ = socket.send(Message::text(snapshot.to_string())).await;
        let event = json!({"type": "event", "value": {"sequence": 3, "type": "guidance.sent",
            "actor": "ana", "artifact_id": null, "detail": {"summary": "Try B"},
            "created_at": "2026-01-01T00:00:00+00:00"}});
        let _ = socket.send(Message::text(event.to_string())).await;
        while let Some(Ok(message)) = socket.recv().await {
            if let Message::Text(text) = message {
                seen.lock().unwrap().messages.push(text.to_string());
                let reply = json!({"type": "delivery", "value": "QUEUED"});
                let _ = socket.send(Message::text(reply.to_string())).await;
            }
        }
    }

    #[test]
    fn attach_streams_activity_and_sends_guidance() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let state = Arc::clone(&seen);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let runtime =
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async move {
                let app = Router::new().route(
                    "/v1/projects/{slug}/live",
                    get(move |upgrade: WebSocketUpgrade, headers: HeaderMap, axum::extract::Path(slug): axum::extract::Path<String>| {
                        let state = Arc::clone(&state);
                        async move {
                            {
                                let mut seen = state.lock().unwrap();
                                seen.path_ok = slug == "demo";
                                seen.headers = headers
                                    .iter()
                                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_owned()))
                                    .collect();
                            }
                            upgrade
                                .protocols(["limina.v1"])
                                .on_upgrade(move |socket| session(socket, state))
                                .into_response()
                        }
                    }),
                );
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });

        let result = run_with_stdin(
            limina().args(["--url", &url, "--token", "t-1", "--actor", "ana", "attach", "demo"]),
            "Try B\n/help\n/pause\n/interrupt\n/detach\n",
        );
        assert_eq!(result.code, 0, "{result:?}");
        assert!(result.stdout.contains("Attached to demo\n"), "{}", result.stdout);
        assert!(result.stdout.contains(HELP_TEXT));

        // Give the server a moment to record the last frames.
        for _ in 0..50 {
            if seen.lock().unwrap().messages.len() >= 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let seen = seen.lock().unwrap();
        assert!(seen.path_ok);
        let header =
            |name: &str| seen.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(header("authorization"), Some("Bearer t-1"));
        assert_eq!(header("x-limina-actor"), Some("ana"));
        assert_eq!(header("sec-websocket-protocol"), Some("limina.v1"));
        assert_eq!(
            seen.messages,
            [
                r#"{"type": "steer", "body": "Try B"}"#,
                r#"{"type": "action", "action": "pause"}"#,
                r#"{"type": "interrupt"}"#,
            ]
        );
    }

    const HELP_TEXT: &str = "Type feedback, or use /pause, /resume, /stop, /interrupt, /detach.";

    #[test]
    fn attach_failures_are_reported() {
        let address = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let result = run(limina().args(["--url", &format!("http://{address}"), "attach", "demo"]));
        assert_eq!(result.code, 5, "{result:?}");
        assert!(
            result.stderr.starts_with("invariant_violation  Could not attach to the live project.")
        );
    }
}
