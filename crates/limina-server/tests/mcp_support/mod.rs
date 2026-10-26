//! Fixtures for the MCP tests: the MCP router nested at `/mcp` on a loopback listener (as the
//! HTTP layer mounts it), the SDK's Streamable HTTP client, and raw HTTP for the transport
//! boundary. Every credential here is fake.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderName, HeaderValue};
use limina_core::Principal;
use limina_server::auth::{Auth, AuthOptions, Authenticator, LocalAuthenticator};
use limina_server::mcp::{self, McpOptions};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    ReadResourceRequestParams, ReadResourceResult,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

use crate::app_support::TestRuntime;

/// The project token of [`token_auth`].
pub const TOKEN: &str = "secret";

/// `LIMINA_API_TOKEN=secret`: every caller with the token is a local user named by its actor.
pub fn token_auth() -> Auth {
    auth_with(
        Arc::new(LocalAuthenticator::new(Some(TOKEN.into()), None).unwrap()),
        AuthOptions::default(),
    )
}

pub fn auth_with(authenticator: Arc<dyn Authenticator>, options: AuthOptions) -> Auth {
    Auth::new(authenticator, None, options).unwrap()
}

/// An identity provider stand-in: each bearer token is one fixed principal; `outsider` is a
/// valid credential whose owner may not use the runtime (403).
pub struct People;

impl People {
    pub fn principal(token: &str) -> Option<Principal> {
        match token {
            "owner" => Some(crate::app_support::owner()),
            "editor" => Some(crate::app_support::editor()),
            "viewer" => Some(crate::app_support::viewer()),
            // A service identity without a display name.
            "service" => Some(Principal::new("service-account", "")),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl Authenticator for People {
    fn mode(&self) -> &str {
        "oidc"
    }

    async fn authenticate(
        &self,
        bearer_token: Option<&str>,
        _actor_hint: Option<&str>,
    ) -> limina_core::Result<Principal> {
        match bearer_token {
            Some("outsider") => Err(limina_core::LiminaError::authorization_with(
                "This account is not allowed to use this Limina instance.",
            )),
            Some(token) => {
                Self::principal(token).ok_or_else(limina_core::LiminaError::authentication)
            }
            None => Err(limina_core::LiminaError::authentication()),
        }
    }
}

pub fn people_auth() -> Auth {
    auth_with(Arc::new(People), AuthOptions::default())
}

/// The principal [`token_auth`] signs `actor` in as.
pub fn local(actor: &str) -> Principal {
    Principal::local(actor, false)
}

/// A runtime served over HTTP with the MCP router at `/mcp`.
pub struct McpServer {
    pub test: TestRuntime,
    pub address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl McpServer {
    /// `http://127.0.0.1:<port>`.
    pub fn base(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The MCP endpoint, as Python served it (`/mcp/`).
    pub fn url(&self) -> String {
        format!("{}/mcp/", self.base())
    }

    /// An MCP client with the project token and `X-Limina-Actor: <actor>`.
    pub async fn client(&self, actor: Option<&str>) -> McpClient {
        connect(&self.url(), Some(TOKEN), actor).await.unwrap()
    }

    /// An MCP client with another bearer token (see [`People`]).
    pub async fn client_as(&self, token: &str) -> McpClient {
        connect(&self.url(), Some(token), None).await.unwrap()
    }

    pub async fn close(self) {
        self.task.abort();
        self.test.close().await;
    }
}

pub async fn serve(test: TestRuntime, auth: Auth) -> McpServer {
    serve_with(test, auth, McpOptions::default()).await
}

pub async fn serve_with(test: TestRuntime, auth: Auth, options: McpOptions) -> McpServer {
    let app = Router::new().nest_service("/mcp", mcp::router(&test.runtime, auth, options));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    McpServer { test, address, task }
}

pub type McpClient = RunningService<RoleClient, ClientConfig>;

/// Connects the SDK's Streamable HTTP client (`clientInfo.name` is `limina-tests`).
pub async fn connect(
    url: &str,
    token: Option<&str>,
    actor: Option<&str>,
) -> Result<McpClient, String> {
    let mut headers = HashMap::new();
    if let Some(actor) = actor {
        headers.insert(
            HeaderName::from_static("x-limina-actor"),
            HeaderValue::from_str(actor).unwrap(),
        );
    }
    let mut config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);
    if let Some(token) = token {
        config = config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    let info =
        ClientConfig::new(ClientCapabilities::default(), Implementation::new("limina-tests", "1"));
    info.serve(transport).await.map_err(|error| error.to_string())
}

/// Calls a tool; the arguments are a JSON object.
pub async fn call(client: &McpClient, tool: &str, arguments: Value) -> CallToolResult {
    let mut params = CallToolRequestParams::new(tool.to_owned());
    if let Value::Object(arguments) = arguments {
        params = params.with_arguments(arguments);
    }
    client.call_tool(params).await.unwrap()
}

/// Calls a tool that must succeed and returns its structured content.
pub async fn structured(client: &McpClient, tool: &str, arguments: Value) -> Value {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(false), "{tool} failed: {result:?}");
    result.structured_content.unwrap()
}

/// Calls a tool that must fail and returns its error text.
pub async fn tool_error(client: &McpClient, tool: &str, arguments: Value) -> String {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(true), "{tool} succeeded: {result:?}");
    assert!(result.structured_content.is_none());
    texts(&result).join("\n")
}

/// The text blocks of a result.
pub fn texts(result: &CallToolResult) -> Vec<String> {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .map(|text| text.text.clone())
        .collect()
}

pub async fn read(client: &McpClient, uri: &str) -> Result<ReadResourceResult, ServiceError> {
    client.read_resource(ReadResourceRequestParams::new(uri)).await
}

/// One raw HTTP exchange.
pub struct Exchange {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: String,
}

impl Exchange {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.body))
    }
}

/// The MCP headers Python's test client sent, with the project token.
pub fn mcp_headers() -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {TOKEN}")),
        ("x-limina-actor", "maya".into()),
        ("accept", "application/json, text/event-stream".into()),
        ("content-type", "application/json".into()),
    ]
}

/// A JSON-RPC request body.
pub fn rpc(id: i64, method: &str, params: Value) -> String {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// Sends a request to `url` with exactly `headers` (a `host` entry replaces the real one).
pub async fn send(
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Option<String>,
) -> Exchange {
    let client =
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let mut request = client.request(method.parse().unwrap(), url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response.text().await.unwrap();
    Exchange { status, headers, body }
}

pub async fn post(url: &str, headers: &[(&str, String)], body: String) -> Exchange {
    send("POST", url, headers, Some(body)).await
}
