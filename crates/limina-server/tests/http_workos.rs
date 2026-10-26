//! The WorkOS browser flows of `tests/test_cloud_workos_auth.py` through the full API, which the
//! sign-in port tested against stand-in routes: the session cookie authenticates the real
//! `/v1/me` and `POST /v1/projects`, writes need the console's `Origin`, an expired session is
//! refreshed and its cookie rotated on an API response, WorkOS bearer tokens work for API clients,
//! and unknown `/auth/…` paths are JSON 404s even with the console served (never the app shell).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod auth_support;
mod http_support;

use std::sync::Arc;

use app_support::*;
use auth_support::{CLIENT_ID, FakeIdp, PASSWORD, TestClient, TestResponse, query};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::header::{CONTENT_TYPE, COOKIE, HOST, SET_COOKIE};
use axum::http::{Method, Request, StatusCode};
use limina_core::new_uuid;
use limina_server::auth::workos::SESSION_COOKIE;
use limina_server::auth::{Auth, AuthOptions, WorkOsAuth, WorkOsSettings};
use limina_server::server::build_app;
use serde_json::{Value, json};
use tower::ServiceExt;

const INDEX: &str = "<!doctype html><title>Limina</title>";

struct Browser {
    idp: FakeIdp,
    workos: Arc<WorkOsAuth>,
    app: Router,
    client: TestClient,
    test: TestRuntime,
}

impl Browser {
    /// WorkOS requiring `limina:access`, no machine tokens, and the console served.
    async fn start() -> Self {
        let idp = FakeIdp::start().await;
        let mut settings = WorkOsSettings::new(CLIENT_ID, "sk_test", PASSWORD).unwrap();
        settings.api_base_url.clone_from(&idp.base_url);
        settings.required_permission = Some("limina:access".into());
        let workos = Arc::new(WorkOsAuth::new(settings, reqwest::Client::new()).unwrap());
        let auth = Auth::with_workos(workos.clone(), None, None, AuthOptions::default()).unwrap();
        let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
        let web = test.root.join("dist");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), INDEX).unwrap();
        let mut config = http_support::config(&test);
        config.web_dir = Some(web);
        let app = build_app(test.runtime.clone(), auth, &config).await;
        let client = TestClient::new(app.clone());
        Self { idp, workos, app, client, test }
    }

    async fn sign_in(&mut self, return_to: &str) -> TestResponse {
        let start = self.client.get(&format!("/auth/login?return_to={return_to}")).await;
        assert_eq!(start.status, StatusCode::SEE_OTHER);
        let state = query(start.location())["state"].clone();
        self.client.get(&format!("/auth/callback?code=code_01&state={state}")).await
    }

    /// A JSON request with the browser's cookies (the shared test client sends no bodies).
    async fn send_json(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> (StatusCode, Value, Vec<String>) {
        let cookie = self
            .client
            .cookies
            .iter()
            .filter(|(_, (_, cookie_path))| cookie_path == "/")
            .map(|(name, (value, _))| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(HOST, "testserver")
            .header(CONTENT_TYPE, "application/json")
            .extension(ConnectInfo("127.0.0.1:50000".parse::<std::net::SocketAddr>().unwrap()));
        if !cookie.is_empty() {
            request = request.header(COOKIE, cookie);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let cookies = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), cookies)
    }

    async fn close(self) {
        drop(self.idp);
        self.test.close().await;
    }
}

fn project(slug: &str) -> Value {
    json!({"slug": slug, "name": "Signed in", "objective": "Use the session.", "success_criteria": "It works."})
}

#[tokio::test]
async fn config_reports_workos_and_nothing_is_anonymous() {
    let mut browser = Browser::start().await;
    assert_eq!(browser.client.get("/auth/config").await.json(), json!({"mode": "workos"}));
    assert_eq!(browser.client.get("/v1/me").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(browser.client.get("/v1/projects").await.status, StatusCode::UNAUTHORIZED);
    let unknown = browser.client.get("/auth/unknown").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.json(), json!({"detail": "Not Found"}));
    let logout = browser.client.get("/auth/logout").await;
    assert_eq!(logout.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(logout.json(), json!({"detail": "Method Not Allowed"}));
    // The sign-in page itself is a console route.
    let signin = browser.client.get("/signin?error=state").await;
    assert_eq!(String::from_utf8_lossy(&signin.body), INDEX);
    browser.close().await;
}

#[tokio::test]
async fn callback_creates_a_session_the_api_accepts() {
    let mut browser = Browser::start().await;
    let callback = browser.sign_in("/projects/demo").await;
    assert_eq!(callback.status, StatusCode::SEE_OTHER);
    assert_eq!(callback.location(), "/projects/demo");
    assert!(browser.client.has_cookie(SESSION_COOKIE));
    let me = browser.client.get("/v1/me").await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(
        me.json(),
        json!({
            "subject": "user_01",
            "display_name": "Ada Lovelace",
            "email": "ada@example.com",
            "instance_admin": false,
            "auth_mode": "workos",
        })
    );
    browser.close().await;
}

#[tokio::test]
async fn session_writes_require_the_console_origin() {
    let mut browser = Browser::start().await;
    browser.sign_in("/projects/demo").await;
    let key = new_uuid();
    let headers = [("idempotency-key", key.as_str())];
    let (status, body, _) =
        browser.send_json(Method::POST, "/v1/projects", &headers, &project("blocked")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["details"]["reason"], "origin");
    let forged = [("idempotency-key", key.as_str()), ("origin", "https://evil.example")];
    let (status, _, _) =
        browser.send_json(Method::POST, "/v1/projects", &forged, &project("forged")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let console = [("idempotency-key", key.as_str()), ("origin", "http://testserver")];
    let (status, created, _) =
        browser.send_json(Method::POST, "/v1/projects", &console, &project("signed-in")).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["role"], "OWNER");
    // The project belongs to the signed-in person.
    let listed = browser.client.get("/v1/projects").await.json();
    assert_eq!(listed["items"][0]["slug"], "signed-in");
    let events = browser.client.get("/v1/projects/signed-in/events").await.json();
    assert_eq!(events["events"][0]["actor"], "Ada Lovelace");
    // Reads need no Origin.
    assert_eq!(browser.client.get("/v1/projects/signed-in").await.status, StatusCode::OK);
    browser.close().await;
}

#[tokio::test]
async fn expired_sessions_refresh_on_api_requests_and_rotate_the_cookie() {
    let mut browser = Browser::start().await;
    browser.sign_in("/projects/demo").await;
    let stale = browser
        .workos
        .sessions()
        .seal(&json!({
            "access_token": browser.idp.token().permissions(&["limina:access"]).expires_in(-120).signed(),
            "refresh_token": "refresh_old",
            "user": {"id": "user_01", "email": "ada@example.com", "first_name": "Ada"},
        }))
        .unwrap();
    browser.client.set_cookie(SESSION_COOKIE, &stale);
    let first = browser.client.get("/v1/projects").await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(first.set_cookies().iter().any(|cookie| cookie.starts_with("limina_session=")));
    // A write with the rotated cookie passes too.
    let key = new_uuid();
    let headers = [("idempotency-key", key.as_str()), ("origin", "http://testserver")];
    let (status, _, _) =
        browser.send_json(Method::POST, "/v1/projects", &headers, &project("after-refresh")).await;
    assert_eq!(status, StatusCode::CREATED);
    browser.close().await;
}

#[tokio::test]
async fn workos_bearer_tokens_work_for_api_clients() {
    let browser = Browser::start().await;
    let token = browser.idp.token().permissions(&["limina:access"]).signed();
    let authorization = format!("Bearer {token}");
    let key = new_uuid();
    // A bearer token is not ambient, so writes need no Origin.
    let headers = [("authorization", authorization.as_str()), ("idempotency-key", key.as_str())];
    let (status, created, _) =
        browser.send_json(Method::POST, "/v1/projects", &headers, &project("api-client")).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, me, _) =
        browser.send_json(Method::GET, "/v1/me", &headers[..1], &Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["subject"], "user_01");
    let denied = browser.idp.token().permissions(&[]).signed();
    let denied = format!("Bearer {denied}");
    let (status, body, _) = browser
        .send_json(Method::GET, "/v1/me", &[("authorization", denied.as_str())], &Value::Null)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "permission_denied");
    browser.close().await;
}

#[tokio::test]
async fn logout_ends_the_session_for_the_api() {
    let mut browser = Browser::start().await;
    browser.sign_in("/projects/demo").await;
    assert_eq!(browser.client.get("/v1/me").await.status, StatusCode::OK);
    let response =
        browser.client.send(Method::POST, "/auth/logout", &[("origin", "http://testserver")]).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(!browser.client.has_cookie(SESSION_COOKIE));
    assert_eq!(browser.client.get("/v1/me").await.status, StatusCode::UNAUTHORIZED);
    browser.close().await;
}
