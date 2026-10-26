//! The transport boundary: authentication failures as the API's JSON errors (401, 403, 429),
//! DNS rebinding protection on `Host` and `Origin`, and the Streamable HTTP details clients rely
//! on (paths, methods, media types).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod mcp_support;

use std::sync::Arc;

use app_support::*;
use limina_server::auth::{AuthLimits, AuthOptions, LocalAuthenticator, RateLimitSettings};
use limina_server::mcp::McpOptions;
use mcp_support::*;
use serde_json::json;

async fn token_server() -> McpServer {
    serve(runtime(ScriptedSession::always(Reply::ok(complete()))).await, token_auth()).await
}

fn list() -> String {
    rpc(1, "tools/list", json!({}))
}

fn with(headers: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    let mut all: Vec<(&'static str, String)> = mcp_headers()
        .into_iter()
        .filter(|(name, _)| !headers.iter().any(|(replaced, _)| replaced == name))
        .collect();
    all.extend(
        headers.iter().filter(|(_, value)| !value.is_empty()).map(|(n, v)| (*n, (*v).to_owned())),
    );
    all
}

fn tool_count(exchange: &Exchange) -> usize {
    assert_eq!(exchange.status, 200, "{}", exchange.body);
    exchange.json()["result"]["tools"].as_array().map_or(0, Vec::len)
}

#[tokio::test]
async fn missing_and_wrong_tokens_are_401_json() {
    let server = token_server().await;
    for authorization in ["", "Bearer wrong", "Basic c2VjcmV0"] {
        let response =
            post(&server.url(), &with(&[("authorization", authorization)]), list()).await;
        assert_eq!(response.status, 401, "{authorization}");
        assert_eq!(
            response.json(),
            json!({"error": {"code": "authentication_required", "message": "Authentication required.", "details": {}}})
        );
        assert_eq!(response.headers["www-authenticate"], "Bearer");
        assert_eq!(response.headers["content-type"], "application/json");
    }
    // Authentication comes first, as in Python: a bad Host without credentials is a 401.
    let response =
        post(&server.url(), &with(&[("authorization", ""), ("host", "attacker.invalid")]), list())
            .await;
    assert_eq!(response.status, 401);
    // The SDK client sees the same refusal.
    let refused = connect(&server.url(), Some("wrong"), None).await.unwrap_err();
    assert!(refused.contains("Auth required"), "{refused}");
    server.close().await;
}

#[tokio::test]
async fn refused_identities_are_403_json() {
    let server =
        serve(runtime(ScriptedSession::always(Reply::ok(complete()))).await, people_auth()).await;
    let response =
        post(&server.url(), &with(&[("authorization", "Bearer outsider")]), list()).await;
    assert_eq!(response.status, 403);
    assert_eq!(
        response.json(),
        json!({"error": {
            "code": "permission_denied",
            "message": "This account is not allowed to use this Limina instance.",
            "details": {},
        }})
    );
    assert!(response.headers.get("www-authenticate").is_none());
    let response = post(&server.url(), &with(&[("authorization", "Bearer owner")]), list()).await;
    assert_eq!(tool_count(&response), limina_server::mcp::tools::TOOLS.len());
    server.close().await;
}

#[tokio::test]
async fn repeated_failures_are_429_json_with_retry_after() {
    let limits = AuthLimits { global: RateLimitSettings::new(2, 60), ..AuthLimits::default() };
    let tokens = LocalAuthenticator::new(Some(TOKEN.into()), None).unwrap();
    let auth = auth_with(Arc::new(tokens), AuthOptions { limits, ..AuthOptions::default() });
    let server = serve(runtime(ScriptedSession::always(Reply::ok(complete()))).await, auth).await;
    for _ in 0..2 {
        let response =
            post(&server.url(), &with(&[("authorization", "Bearer wrong")]), list()).await;
        assert_eq!(response.status, 401);
    }
    // The global ceiling now refuses everyone, even the right token, until the window passes.
    let response = post(&server.url(), &mcp_headers(), list()).await;
    assert_eq!(response.status, 429);
    let body = response.json();
    assert_eq!(body["error"]["code"], "rate_limited");
    assert_eq!(body["error"]["message"], "Too many failed authentication attempts.");
    let retry_after = body["error"]["details"]["retry_after_seconds"].as_u64().unwrap();
    assert!((1..=60).contains(&retry_after));
    assert_eq!(response.headers["retry-after"], retry_after.to_string().as_str());
    server.close().await;
}

#[tokio::test]
async fn disallowed_hosts_and_origins_are_refused() {
    let server = token_server().await;
    let port = server.address.port();
    let cases: [(&str, &str, u16, &str); 8] = [
        ("host", "attacker.invalid", 421, "Invalid Host header"),
        // `localhost:*` needs a port, like the Python SDK's wildcard.
        ("host", "localhost", 421, "Invalid Host header"),
        ("host", "localhost.attacker.invalid:80", 421, "Invalid Host header"),
        ("origin", "http://evil.example", 403, "Invalid Origin header"),
        ("origin", "https://localhost:5173", 403, "Invalid Origin header"),
        ("origin", "null", 403, "Invalid Origin header"),
        ("host", "", 200, ""),
        ("origin", "", 200, ""),
    ];
    for (header, value, status, message) in cases {
        let response = post(&server.url(), &with(&[(header, value)]), list()).await;
        assert_eq!(response.status, status, "{header}: {value}");
        if status != 200 {
            assert_eq!(response.body, message, "{header}: {value}");
        }
    }
    for (header, value) in [
        ("host", format!("localhost:{port}")),
        ("host", format!("[::1]:{port}")),
        ("origin", format!("http://localhost:{port}")),
        ("origin", "http://127.0.0.1:5173".to_owned()),
    ] {
        let response = post(&server.url(), &with(&[(header, &value)]), list()).await;
        assert_eq!(
            tool_count(&response),
            limina_server::mcp::tools::TOOLS.len(),
            "{header}: {value}"
        );
    }
    server.close().await;
}

#[tokio::test]
async fn configured_hosts_and_origins_replace_the_loopback_defaults() {
    let options = McpOptions {
        allowed_hosts: vec!["limina.example.com".into(), "mcp.example.com:*".into()],
        allowed_origins: vec!["https://console.example.com".into()],
    };
    let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
    let server = serve_with(test, token_auth(), options).await;
    let allowed = [
        ("host", "limina.example.com"),
        ("host", "mcp.example.com:8443"),
        ("origin", "https://console.example.com"),
    ];
    for (header, value) in allowed {
        let mut headers = with(&[(header, value)]);
        if header == "origin" {
            headers.push(("host", "limina.example.com".into()));
        }
        assert_eq!(
            tool_count(&post(&server.url(), &headers, list()).await),
            limina_server::mcp::tools::TOOLS.len(),
            "{value}"
        );
    }
    let loopback = post(&server.url(), &mcp_headers(), list()).await;
    assert_eq!(loopback.status, 421);
    let origin = with(&[("host", "limina.example.com"), ("origin", "http://localhost:5173")]);
    assert_eq!(post(&server.url(), &origin, list()).await.status, 403);
    server.close().await;
}

#[tokio::test]
async fn the_streamable_http_transport_behaves_like_python() {
    let server = token_server().await;
    // Requests must be JSON.
    let response = post(&server.url(), &with(&[("content-type", "text/plain")]), list()).await;
    assert_eq!((response.status, response.body.as_str()), (400, "Invalid Content-Type header"));
    let response = post(&server.url(), &with(&[("content-type", "")]), list()).await;
    assert_eq!(response.status, 400);
    // JSON responses, also for clients that only accept JSON.
    let response = post(&server.url(), &with(&[("accept", "application/json")]), list()).await;
    assert_eq!(tool_count(&response), limina_server::mcp::tools::TOOLS.len());
    assert_eq!(response.headers["content-type"], "application/json");
    // Stateless: no session, no initialize required, notifications are accepted.
    let response = post(&server.url(), &mcp_headers(), list()).await;
    assert!(response.headers.get("mcp-session-id").is_none());
    let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
    assert_eq!(post(&server.url(), &mcp_headers(), notification).await.status, 202);
    let ping = post(&server.url(), &mcp_headers(), rpc(7, "ping", json!({}))).await;
    assert_eq!(ping.json(), json!({"jsonrpc": "2.0", "id": 7, "result": {}}));
    // No server-initiated stream and no session to delete.
    let get = with(&[("accept", "text/event-stream")]);
    assert_eq!(send("GET", &server.url(), &get, None).await.status, 405);
    assert_eq!(send("DELETE", &server.url(), &mcp_headers(), None).await.status, 405);
    // `/mcp` and `/mcp/` are the endpoint; nothing lives below it.
    let bare = format!("{}/mcp", server.base());
    assert_eq!(
        tool_count(&post(&bare, &mcp_headers(), list()).await),
        limina_server::mcp::tools::TOOLS.len()
    );
    let below = format!("{}/mcp/tools", server.base());
    assert_eq!(post(&below, &mcp_headers(), list()).await.status, 404);
    server.close().await;
}

#[tokio::test]
async fn the_sdk_client_negotiates_a_published_protocol_version() {
    let server = token_server().await;
    for url in [server.url(), format!("{}/mcp", server.base())] {
        let client = connect(&url, Some(TOKEN), Some("maya")).await.unwrap();
        let info = client.peer_info().unwrap();
        let version = serde_json::to_value(&info.protocol_version).unwrap();
        assert_eq!(version, "2025-11-25");
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), limina_server::mcp::tools::TOOLS.len());
        client.cancel().await.unwrap();
    }
    // Older clients keep their version; newer ones are offered the newest served.
    for (requested, negotiated) in [("2025-06-18", "2025-06-18"), ("2026-07-28", "2025-11-25")] {
        let initialize = rpc(
            1,
            "initialize",
            json!({
                "protocolVersion": requested,
                "capabilities": {},
                "clientInfo": {"name": "limina-tests", "version": "1"},
            }),
        );
        let response = post(&server.url(), &mcp_headers(), initialize).await;
        assert_eq!(response.json()["result"]["protocolVersion"], negotiated, "{}", response.body);
    }
    server.close().await;
}
