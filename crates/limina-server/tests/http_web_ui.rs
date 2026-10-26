//! `tests/test_cloud_web_ui.py`: the console served next to the API — the SPA fallback with a
//! revalidated `index.html`, 404s for missing files, immutable assets, API paths never shadowed,
//! no path traversal, and an API-only runtime without a build. Also the sign-in routes'
//! precedence over the console (`/auth/config` vs. an unknown `/auth/…` path).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use app_support::*;
use axum::http::Method;
use http_support::*;
use limina_server::auth::AuthConfig;
use limina_server::http::web::{resolve_web_dir, static_file};
use limina_server::server::ServerConfig;
use serde_json::json;

const INDEX: &str = "<!doctype html><title>Limina</title><div id=root></div>";
const SECRET: &[(&str, &str)] = &[("authorization", "Bearer secret")];

/// A build directory like the Python `setUp`'s, next to a secret file it must never serve.
fn build(root: &Path) -> PathBuf {
    let web = root.join("dist");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), INDEX).unwrap();
    std::fs::write(web.join("assets").join("app-3f9a.js"), "console.log('limina')").unwrap();
    std::fs::write(web.join("favicon.svg"), "<svg/>").unwrap();
    std::fs::write(root.join("secret.txt"), "TOP-SECRET-OUTSIDE-WEB-DIR").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("secret.txt"), web.join("escape.txt")).unwrap();
    web
}

async fn console() -> (Api, PathBuf) {
    let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
    let web = build(&test.root);
    let mut config = config(&test);
    config.web_dir = Some(web.clone());
    let auth = limina_server::auth::Auth::new(
        std::sync::Arc::new(
            limina_server::auth::LocalAuthenticator::new(Some("secret".into()), None).unwrap(),
        ),
        None,
        Default::default(),
    )
    .unwrap();
    (Api::with_config(test, auth, config).await, web)
}

#[tokio::test]
async fn client_routes_fall_back_to_the_revalidated_index() {
    let (api, _) = console().await;
    for path in ["/", "/projects/retrieval", "/projects/retrieval/runs/abc?tab=events", "/signin"] {
        let response = api.get(path, &[]).await;
        response.expect(200);
        assert_eq!(response.text(), INDEX, "{path}");
        assert!(response.header("content-type").unwrap().starts_with("text/html"));
        assert_eq!(response.header("cache-control"), Some("no-cache"));
    }
    let head = api.call(Method::HEAD, "/projects/retrieval", &[]).await;
    head.expect(200);
    assert!(head.body.is_empty());
    // A revalidation with the same entity tag is answered without the body.
    let first = api.get("/projects/retrieval", &[]).await;
    let etag = first.header("etag").unwrap().to_owned();
    let cached = api.get("/projects/retrieval", &[("if-none-match", etag.as_str())]).await;
    cached.expect(304);
    assert!(cached.body.is_empty());
    api.close().await;
}

#[tokio::test]
async fn missing_files_are_not_answered_with_the_app_shell() {
    let (api, _) = console().await;
    for path in ["/favicon.ico", "/robots.txt", "/deep/missing.js"] {
        let response = api.get(path, &[]).await;
        response.expect(404);
        assert_eq!(response.json(), json!({"detail": "Not Found"}), "{path}");
    }
    api.close().await;
}

#[tokio::test]
async fn static_files_are_served_with_appropriate_caching() {
    let (api, _) = console().await;
    let asset = api.get("/assets/app-3f9a.js", &[]).await;
    asset.expect(200);
    assert_eq!(asset.text(), "console.log('limina')");
    assert_eq!(asset.header("cache-control"), Some("public, max-age=31536000, immutable"));
    assert!(asset.header("content-type").unwrap().starts_with("text/javascript"));
    let favicon = api.get("/favicon.svg", &[]).await;
    favicon.expect(200);
    assert_eq!(favicon.text(), "<svg/>");
    assert_eq!(favicon.header("content-type"), Some("image/svg+xml"));
    let missing = api.get("/assets/missing.js", &[]).await;
    missing.expect(404);
    assert_eq!(missing.json(), json!({"detail": "Not Found"}));
    api.close().await;
}

#[tokio::test]
async fn api_routes_are_never_shadowed() {
    let (api, _) = console().await;
    for (method, path) in [
        (Method::GET, "/v1/unknown"),
        (Method::POST, "/v1/unknown"),
        (Method::GET, "/internal/v1/unknown"),
        (Method::GET, "/docs/unknown"),
        (Method::GET, "/auth/unknown"),
    ] {
        let response = api.call(method.clone(), path, SECRET).await;
        response.expect(404);
        assert_eq!(response.json(), json!({"detail": "Not Found"}), "{method} {path}");
    }
    // `/mcp` belongs to the MCP server (which rejects this test Host), never to the console.
    for path in ["/mcp", "/mcp/"] {
        let response = api.get(path, SECRET).await;
        assert_ne!(response.status, axum::http::StatusCode::OK, "{path}");
        assert!(!String::from_utf8_lossy(&response.body).contains("<div id=root>"), "{path}");
    }
    let projects = api.get("/v1/projects", SECRET).await;
    projects.expect(200);
    assert_eq!(projects.json()["total"], 0);
    api.get("/v1/projects", &[]).await.expect(401);
    let not_allowed = api.delete("/v1/projects", SECRET).await;
    not_allowed.expect(405);
    assert_eq!(not_allowed.json(), json!({"detail": "Method Not Allowed"}));
    api.call(Method::POST, "/projects/retrieval", &[]).await.expect(404);
    assert!(api.get("/openapi.json", &[]).await.json().get("paths").is_some());
    assert_eq!(api.get("/livez", &[]).await.json(), json!({"ok": true}));
    // The sign-in routes win over the console.
    assert_eq!(api.get("/auth/config", &[]).await.json(), json!({"mode": "token"}));
    let docs = api.get("/docs", &[]).await;
    docs.expect(200);
    assert!(docs.text().contains("swagger-ui"));
    assert!(docs.text().contains("/openapi.json"));
    api.get("/redoc", &[]).await.expect(200);
    api.close().await;
}

#[tokio::test]
async fn path_traversal_never_escapes_the_build_directory() {
    let (api, web) = console().await;
    for path in [
        "/%2e%2e/secret.txt",
        "/..%2fsecret.txt",
        "/escape.txt",
        "/assets/%2e%2e/%2e%2e/secret.txt",
        "/assets/../../secret.txt",
        "/assets/%2e%2e/index.html",
    ] {
        let response = api.get(path, &[]).await;
        assert!(!response.text().contains("TOP-SECRET"), "{path}");
        assert_ne!(
            response.header("cache-control"),
            Some("public, max-age=31536000, immutable"),
            "{path}"
        );
    }
    let root = web.canonicalize().unwrap();
    assert_eq!(static_file(&root, "/../secret.txt").await, None);
    assert_eq!(static_file(&root, "/escape.txt").await, None);
    assert_eq!(static_file(&root, "/bad\0name").await, None);
    assert_eq!(static_file(&root, "/favicon.svg").await, Some(root.join("favicon.svg")));
    api.close().await;
}

#[tokio::test]
async fn runtime_stays_api_only_without_a_build() {
    let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
    let missing = test.root.join("missing");
    assert_eq!(resolve_web_dir(Some(&missing)), None);
    let mut api_only = config(&test);
    api_only.web_dir = Some(missing);
    let api = Api::with_config(test, local_tokens(), api_only).await;
    api.get("/", &[]).await.expect(404);
    assert_eq!(api.get("/projects/x", &[]).await.json(), json!({"detail": "Not Found"}));
    api.get("/assets/app.js", &[]).await.expect(404);

    let web = build(&api.test.root);
    let env = BTreeMap::from([("LIMINA_WEB_DIR".to_owned(), web.display().to_string())]);
    let configured = ServerConfig::from_map(&env, AuthConfig::default()).unwrap();
    assert_eq!(configured.web_dir.as_deref(), Some(web.as_path()));
    assert_eq!(resolve_web_dir(configured.web_dir.as_deref()), Some(web.canonicalize().unwrap()));
    api.close().await;
}
