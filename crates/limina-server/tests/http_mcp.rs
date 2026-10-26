//! MCP mounted in the full application: `/mcp` and `/mcp/` reach the MCP server through the
//! same sign-in, request log and CORS layers as the REST API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use app_support::*;
use axum::http::StatusCode;
use http_support::*;
use serde_json::json;

fn initialize() -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }
    })
}

#[tokio::test]
async fn mcp_is_served_by_the_application_router() {
    let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
    let api = Api::new(test, ui_tokens()).await;
    let headers = [
        ("host", "127.0.0.1:7433"),
        ("accept", "application/json, text/event-stream"),
        ("authorization", "Bearer owner-token"),
    ];
    for path in ["/mcp", "/mcp/"] {
        let answer = api.post(path, &headers, &initialize()).await;
        assert_eq!(answer.status, StatusCode::OK, "{path}: {answer:?}");
        let body = String::from_utf8_lossy(&answer.body);
        assert!(body.contains("serverInfo"), "{path}: {body}");
    }

    let anonymous = api
        .post("/mcp/", &[("host", "127.0.0.1:7433"), ("accept", "application/json")], &initialize())
        .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.json()["error"]["code"], "authentication_required");

    let rebinding = api
        .post("/mcp/", &[("host", "attacker.example:80"), headers[1], headers[2]], &initialize())
        .await;
    assert_ne!(rebinding.status, StatusCode::OK, "a foreign Host header is rejected");
}
