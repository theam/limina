//! Serving a complete instance (`server::serve`) and the server-level settings: graceful
//! shutdown that closes live sessions and stops the runtime, CORS for `LIMINA_CORS_ORIGINS`,
//! request body limits and uploads in either field order.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use std::time::Duration;

use app_support::*;
use axum::http::Method;
use http_support::*;
use limina_server::app::RuntimeConfig;
use limina_server::auth::AuthConfig;
use limina_server::server::{ServerConfig, serve};
use serde_json::json;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test]
async fn serve_runs_until_shutdown_and_closes_live_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut runtime = RuntimeConfig::new(
        format!("sqlite:///{}", root.join("server.db").display()),
        root.join("workspaces"),
    );
    runtime.agent_factory = Some(factory(ScriptedSession::always(Reply::ok(complete()))));
    runtime.poll_interval = Duration::from_millis(10);
    let auth = AuthConfig { allow_insecure_no_auth: true, ..AuthConfig::default() };
    let mut config = ServerConfig::new(runtime, auth);
    config.port = free_port();
    let base = format!("http://127.0.0.1:{}", config.port);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(config, async move {
        let _ = stopped.await;
    }));

    let client = reqwest::Client::new();
    let mut ready = None;
    for _ in 0..200 {
        if let Ok(response) = client.get(format!("{base}/readyz")).send().await {
            ready = Some(response.status());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(ready, Some(reqwest::StatusCode::OK));
    let health: serde_json::Value =
        client.get(format!("{base}/healthz")).send().await.unwrap().json().await.unwrap();
    assert_eq!(health["auth_mode"], "local");
    assert_eq!(
        client.get(format!("{base}/auth/config")).send().await.unwrap().text().await.unwrap(),
        r#"{"mode":"open"}"#
    );
    let created = client
        .post(format!("{base}/v1/projects"))
        .header("idempotency-key", "serve-test")
        .json(&json!({"slug": "served", "name": "Served", "objective": "Run.", "success_criteria": "Done."}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);

    let url =
        format!("ws://127.0.0.1:{}/v1/projects/served/live", base.rsplit(':').next().unwrap());
    let (mut socket, _) = connect(&url, &["limina.v1"], &[]).await.unwrap();
    assert_eq!(next_json(&mut socket).await["type"], "snapshot");
    stop.send(()).unwrap();
    assert_eq!(close_of(&mut socket).await, Some((1001, "The runtime is shutting down.".into())));
    let outcome = tokio::time::timeout(Duration::from_secs(10), server).await.unwrap().unwrap();
    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(client.get(format!("{base}/livez")).send().await.is_err());
}

#[tokio::test]
async fn serving_refuses_an_open_non_local_bind() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = RuntimeConfig::new(
        format!("sqlite:///{}", dir.path().join("server.db").display()),
        dir.path().join("workspaces"),
    );
    let mut config = ServerConfig::new(runtime, AuthConfig::default());
    config.host = "0.0.0.0".into();
    let error = serve(config, async {}).await.unwrap_err();
    assert!(error.message.contains("non-local bind"), "{}", error.message);
}

#[tokio::test]
async fn cors_follows_the_configured_origins() {
    let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
    let mut settings = config(&test);
    settings.cors_origins = vec!["https://console.example".into()];
    let api = Api::with_config(test, local_tokens(), settings).await;
    let preflight = api
        .call(
            Method::OPTIONS,
            "/v1/projects",
            &[
                ("origin", "https://console.example"),
                ("access-control-request-method", "POST"),
                ("access-control-request-headers", "authorization, idempotency-key, content-type"),
            ],
        )
        .await;
    preflight.expect(200);
    assert_eq!(preflight.header("access-control-allow-origin"), Some("https://console.example"));
    let methods = preflight.header("access-control-allow-methods").unwrap().to_owned();
    assert!(methods.contains("PATCH") && methods.contains("DELETE"), "{methods}");
    let headers = preflight.header("access-control-allow-headers").unwrap().to_lowercase();
    assert!(headers.contains("idempotency-key"), "{headers}");
    assert_eq!(preflight.header("access-control-allow-credentials"), None);

    let simple = api
        .get(
            "/v1/projects",
            &[("origin", "https://console.example"), ("authorization", "Bearer secret")],
        )
        .await;
    simple.expect(200);
    assert_eq!(simple.header("access-control-allow-origin"), Some("https://console.example"));
    let foreign = api
        .get(
            "/v1/projects",
            &[("origin", "https://evil.example"), ("authorization", "Bearer secret")],
        )
        .await;
    assert_eq!(foreign.header("access-control-allow-origin"), None);
    api.close().await;

    // Without configured origins there are no CORS headers at all.
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, local_tokens())
            .await;
    let plain = api.get("/livez", &[("origin", "https://console.example")]).await;
    assert_eq!(plain.header("access-control-allow-origin"), None);
    api.close().await;
}

fn form(parts: &[(&str, Option<&str>, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "serve-boundary";
    let mut body = Vec::new();
    for (name, filename, content) in parts {
        let disposition = match filename {
            Some(filename) => format!(
                "form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: text/plain"
            ),
            None => format!("form-data; name=\"{name}\""),
        };
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: {disposition}\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[tokio::test]
async fn bodies_beyond_the_limits_are_refused() {
    let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
    let mut settings = config(&test);
    settings.json_body_limit_bytes = 1024;
    settings.upload_limit_bytes = DEFAULT_UPLOAD;
    let api = Api::with_config(test, local_tokens(), settings).await;
    let create = json!({"slug": "limits", "name": "Limits", "objective": "Bound bodies.", "success_criteria": "413."});
    api.post("/v1/projects", &h(&command("secret", None, "x")), &create).await.expect(201);
    let large = json!({"value": "v".repeat(2048)});
    let refused = api
        .put(
            "/v1/projects/limits/resources/variables/BIG",
            &h(&command("secret", None, "x")),
            &large,
        )
        .await;
    let body = assert_error(&refused, 413, "payload_too_large");
    assert_eq!(body["error"]["details"]["limit_bytes"], 1024);
    api.close().await;
}

/// The upload limit the runtime enforces (`app_support` keeps the 25 MiB default).
const DEFAULT_UPLOAD: u64 = 25 * 1024 * 1024;

#[tokio::test]
async fn uploads_are_limited_and_accept_either_field_order() {
    let test = runtime_with(factory(ScriptedSession::always(Reply::ok(complete()))), |config| {
        config.upload_limit_bytes = 1024;
    })
    .await;
    let mut settings = config(&test);
    settings.upload_limit_bytes = 1024;
    let api = Api::with_config(test, local_tokens(), settings).await;
    let create = json!({"slug": "uploads", "name": "Uploads", "objective": "Store files.", "success_criteria": "Stored."});
    api.post("/v1/projects", &h(&command("secret", None, "x")), &create).await.expect(201);
    let path = "/v1/projects/uploads/sources/upload";
    let secret = [("authorization", "Bearer secret")];

    let (content_type, body) =
        form(&[("name", None, b"notes"), ("file", Some("notes.txt"), b"hello")]);
    let stored = api.raw(Method::POST, path, &secret, &content_type, body).await;
    assert_eq!(stored.expect(201).json()["metadata"]["size_bytes"], 5);

    // A file before its name is kept until the name arrives.
    let (content_type, body) =
        form(&[("file", Some("first.txt"), b"file first"), ("name", None, b"first")]);
    let stored = api.raw(Method::POST, path, &secret, &content_type, body).await;
    let stored = stored.expect(201).json();
    assert_eq!(
        (stored["name"].clone(), stored["metadata"]["filename"].clone()),
        (json!("first"), json!("first.txt"))
    );

    // Over the upload limit: refused by the runtime, nothing kept.
    let (content_type, body) =
        form(&[("name", None, b"big"), ("file", Some("big.txt"), &[b'x'; 2048])]);
    let refused = api.raw(Method::POST, path, &secret, &content_type, body).await;
    let error = assert_error(&refused, 422, "invariant_violation");
    assert_eq!(error["error"]["message"], "Uploaded sources must be at most 1024 bytes.");
    let (content_type, body) =
        form(&[("file", Some("big.txt"), &[b'x'; 2048]), ("name", None, b"big")]);
    assert_error(
        &api.raw(Method::POST, path, &secret, &content_type, body).await,
        422,
        "invariant_violation",
    );
    let sources = api.get("/v1/projects/uploads/sources", &secret).await.json();
    assert_eq!(sources.as_array().unwrap().len(), 2);

    // Far over the limit the request itself is refused.
    let huge = vec![b'x'; 2 * 1024 * 1024 + 4096];
    let (content_type, body) = form(&[("name", None, b"huge"), ("file", Some("huge.txt"), &huge)]);
    let refused = api.raw(Method::POST, path, &secret, &content_type, body).await;
    assert!(matches!(refused.status.as_u16(), 413 | 422), "{}", refused.text());

    // Viewers cannot upload, whatever the order.
    let (content_type, body) = form(&[("file", Some("x.txt"), b"x"), ("name", None, b"x")]);
    let anonymous = api.raw(Method::POST, path, &[], &content_type, body).await;
    assert_error(&anonymous, 401, "authentication_required");
    api.close().await;
}
