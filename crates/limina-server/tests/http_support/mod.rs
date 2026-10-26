//! Fixtures for the HTTP tests: the application router built with `build_app` over the
//! application-layer test runtime (`app_support`), a request helper that drives it with
//! `tower::ServiceExt::oneshot` as a fixed client address, the Python suites' token
//! authenticators, a real listener for WebSocket tests and a small live-socket client. Every
//! value here is fake.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::header::{CONTENT_TYPE, HOST};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use futures::{SinkExt, StreamExt};
use limina_core::{LiminaError, Principal, Result, new_uuid};
use limina_server::app::RuntimeConfig;
use limina_server::auth::{Auth, AuthConfig, AuthOptions, Authenticator, LocalAuthenticator};
use limina_server::server::{ServerConfig, build_app};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tower::ServiceExt;

use crate::app_support::{self, TestRuntime};

// ─── Authenticators ───────────────────────────────────────────────────────────

/// The Python suites' `TokenAuthenticator` (`mode = "oidc"`): fixed bearer tokens → principals.
pub struct Tokens(HashMap<String, Principal>);

#[async_trait]
impl Authenticator for Tokens {
    fn mode(&self) -> &str {
        "oidc"
    }

    async fn authenticate(&self, bearer: Option<&str>, _actor: Option<&str>) -> Result<Principal> {
        self.0.get(bearer.unwrap_or_default()).cloned().ok_or_else(LiminaError::authentication)
    }
}

/// `owner-token`, `viewer-token`, `editor-token`, `same-name-token` and `admin-token`.
pub fn ui_tokens() -> Auth {
    let principals = HashMap::from([
        ("owner-token".to_owned(), app_support::owner()),
        ("viewer-token".to_owned(), app_support::viewer()),
        ("editor-token".to_owned(), app_support::editor()),
        ("same-name-token".to_owned(), app_support::namesake()),
        ("admin-token".to_owned(), app_support::admin()),
    ]);
    token_auth(principals)
}

pub fn token_auth(principals: HashMap<String, Principal>) -> Auth {
    Auth::new(Arc::new(Tokens(principals)), None, AuthOptions::default()).unwrap()
}

/// `LIMINA_API_TOKEN=secret`, `LIMINA_ADMIN_API_TOKEN=admin-secret` (the API tests' setup).
pub fn local_tokens() -> Auth {
    let local =
        LocalAuthenticator::new(Some("secret".into()), Some("admin-secret".into())).unwrap();
    Auth::new(Arc::new(local), None, AuthOptions::default()).unwrap()
}

/// The localhost open mode (no tokens).
pub fn open_auth() -> Auth {
    Auth::new(Arc::new(LocalAuthenticator::open()), None, AuthOptions::default()).unwrap()
}

// ─── The application ──────────────────────────────────────────────────────────

/// The router over a test runtime.
pub struct Api {
    pub app: Router,
    pub test: TestRuntime,
    /// The client address requests come from (`ConnectInfo`).
    pub peer: SocketAddr,
}

/// A server configuration for `test` (defaults; no console, no CORS).
pub fn config(test: &TestRuntime) -> ServerConfig {
    let runtime = RuntimeConfig::new("sqlite://unused", test.root.join("workspaces"));
    ServerConfig::new(runtime, AuthConfig::default())
}

impl Api {
    pub async fn new(test: TestRuntime, auth: Auth) -> Self {
        let config = config(&test);
        Self::with_config(test, auth, config).await
    }

    pub async fn with_config(test: TestRuntime, auth: Auth, config: ServerConfig) -> Self {
        let app = build_app(test.runtime.clone(), auth, &config).await;
        Self { app, test, peer: "127.0.0.1:50000".parse().unwrap() }
    }

    pub async fn send(&self, request: Request<Body>) -> Answer {
        let mut request = request;
        request.extensions_mut().insert(ConnectInfo(self.peer));
        if !request.headers().contains_key(HOST) {
            request.headers_mut().insert(HOST, "testserver".parse().unwrap());
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Answer { status, headers, body }
    }

    /// A request without a body.
    pub async fn call(&self, method: Method, path: &str, headers: &[(&str, &str)]) -> Answer {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        self.send(builder.body(Body::empty()).unwrap()).await
    }

    pub async fn get(&self, path: &str, headers: &[(&str, &str)]) -> Answer {
        self.call(Method::GET, path, headers).await
    }

    /// A request with a JSON body.
    pub async fn json(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Answer {
        self.raw(method, path, headers, "application/json", body.to_string().into_bytes()).await
    }

    /// A request with any body and content type.
    pub async fn raw(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        content_type: &str,
        body: Vec<u8>,
    ) -> Answer {
        let mut builder =
            Request::builder().method(method).uri(path).header(CONTENT_TYPE, content_type);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        self.send(builder.body(Body::from(body)).unwrap()).await
    }

    pub async fn post(&self, path: &str, headers: &[(&str, &str)], body: &Value) -> Answer {
        self.json(Method::POST, path, headers, body).await
    }

    pub async fn put(&self, path: &str, headers: &[(&str, &str)], body: &Value) -> Answer {
        self.json(Method::PUT, path, headers, body).await
    }

    pub async fn patch(&self, path: &str, headers: &[(&str, &str)], body: &Value) -> Answer {
        self.json(Method::PATCH, path, headers, body).await
    }

    pub async fn delete(&self, path: &str, headers: &[(&str, &str)]) -> Answer {
        self.call(Method::DELETE, path, headers).await
    }

    /// Serves the router on a loopback port (for WebSockets).
    pub async fn listen(&self) -> Listening {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let service = self.app.clone().into_make_service_with_connect_info::<SocketAddr>();
        let server = tokio::spawn(async move {
            axum::serve(listener, service).await.unwrap();
        });
        Listening { address, server }
    }

    pub async fn close(self) {
        self.test.close().await;
    }
}

/// A served router; stops when dropped.
pub struct Listening {
    pub address: SocketAddr,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Listening {
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{path}", self.address)
    }
}

/// One response, fully read.
#[derive(Debug)]
pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("not JSON ({}): {}", self.status, self.text()))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|value| value.to_str().unwrap())
    }

    /// Asserts the status, showing the body when it differs.
    #[track_caller]
    pub fn expect(&self, status: u16) -> &Self {
        assert_eq!(self.status.as_u16(), status, "{}", self.text());
        self
    }
}

// ─── Headers ──────────────────────────────────────────────────────────────────

pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// `Authorization` for `token`.
pub fn auth(token: &str) -> Vec<(String, String)> {
    vec![("authorization".into(), bearer(token))]
}

/// `Authorization`, a fresh (or given) `Idempotency-Key` and a spoofed `X-Limina-Actor`.
pub fn command(token: &str, key: Option<&str>, actor: &str) -> Vec<(String, String)> {
    vec![
        ("authorization".into(), bearer(token)),
        ("idempotency-key".into(), key.map_or_else(new_uuid, str::to_owned)),
        ("x-limina-actor".into(), actor.into()),
    ]
}

/// Borrows owned header pairs for the request helpers.
pub fn h(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect()
}

// ─── Live sockets ─────────────────────────────────────────────────────────────

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Opens a live socket offering `protocols` with extra `headers`. The handshake response's
/// selected subprotocol is returned with the socket.
pub async fn connect(
    url: &str,
    protocols: &[&str],
    headers: &[(&str, &str)],
) -> std::result::Result<(Socket, Option<String>), tokio_tungstenite::tungstenite::Error> {
    let mut request = url.into_client_request().unwrap();
    if !protocols.is_empty() {
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocols.join(", ").parse().unwrap());
    }
    for (name, value) in headers {
        request.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    let (socket, response) = tokio_tungstenite::connect_async(request).await?;
    let protocol = response
        .headers()
        .get("sec-websocket-protocol")
        .map(|value| value.to_str().unwrap().to_owned());
    Ok((socket, protocol))
}

/// What the server sent next.
#[derive(Debug, PartialEq)]
pub enum Frame {
    Json(Value),
    Closed(Option<(u16, String)>),
}

/// The next data or close frame (pings are answered by the client library), within 5 s.
pub async fn next_frame(socket: &mut Socket) -> Frame {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("no frame within 5 s");
        match message {
            None => return Frame::Closed(None),
            Some(Err(_)) => return Frame::Closed(None),
            Some(Ok(Message::Text(text))) => {
                return Frame::Json(serde_json::from_str(&text).unwrap());
            }
            Some(Ok(Message::Close(frame))) => {
                return Frame::Closed(
                    frame.map(|frame| (u16::from(frame.code), frame.reason.to_string())),
                );
            }
            Some(Ok(_)) => {}
        }
    }
}

/// The next JSON frame.
pub async fn next_json(socket: &mut Socket) -> Value {
    match next_frame(socket).await {
        Frame::Json(value) => value,
        Frame::Closed(close) => panic!("the socket closed: {close:?}"),
    }
}

/// The next JSON frame that is not an event.
pub async fn next_non_event(socket: &mut Socket) -> Value {
    loop {
        let frame = next_json(socket).await;
        if frame["type"] != "event" {
            return frame;
        }
    }
}

/// The close frame, skipping any data frames before it.
pub async fn close_of(socket: &mut Socket) -> Option<(u16, String)> {
    loop {
        if let Frame::Closed(close) = next_frame(socket).await {
            return close;
        }
    }
}

pub async fn send_json(socket: &mut Socket, value: &Value) {
    socket.send(Message::text(value.to_string())).await.unwrap();
}

/// Inserts events directly, like the Python tests' `session.add(Event(...))`.
pub async fn insert_events(test: &TestRuntime, slug: &str, events: &[(&str, &str, Value)]) {
    let challenge = test.runtime.service().get_challenge(slug).await.unwrap();
    let mut tx = test.runtime.db().begin_write().await.unwrap();
    for (event_type, actor, payload) in events {
        limina_domain::common::record_event(
            &mut tx,
            &challenge.id,
            event_type,
            actor,
            &new_uuid(),
            payload.clone(),
            None,
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

/// A JSON value's keys, for shape assertions.
pub fn keys(value: &Value) -> Vec<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

pub fn assert_error(reply: &Answer, status: u16, code: &str) -> Value {
    reply.expect(status);
    let body = reply.json();
    assert_eq!(body["error"]["code"], json!(code), "{body}");
    body
}
