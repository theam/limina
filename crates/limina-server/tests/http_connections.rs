#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod app_support;
mod http_support;
use app_support::*;
use axum::{
    Json, Router,
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use http_support::*;
use limina_store::args;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Provider {
    base: String,
    tokens: Arc<Mutex<Vec<HashMap<String, String>>>>,
}
struct Fake {
    provider: Provider,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fake() -> Fake {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = Provider {
        base: format!("http://{}", listener.local_addr().unwrap()),
        tokens: Arc::default(),
    };
    async fn resource(State(p): State<Provider>) -> Json<Value> {
        Json(
            json!({"resource":format!("{}/mcp",p.base),"authorization_servers":[p.base],"scopes_supported":["read","offline_access"]}),
        )
    }
    async fn metadata(State(p): State<Provider>) -> Json<Value> {
        Json(
            json!({"issuer":p.base,"authorization_endpoint":format!("{}/authorize",p.base),"token_endpoint":format!("{}/token",p.base),"registration_endpoint":format!("{}/register",p.base),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"scopes_supported":["read","offline_access"]}),
        )
    }
    async fn register(Json(body): Json<Value>) -> Json<Value> {
        Json(
            json!({"client_id":"limina-test-client","redirect_uris":body["redirect_uris"],"token_endpoint_auth_method":"none"}),
        )
    }
    async fn token(
        State(p): State<Provider>,
        Form(body): Form<HashMap<String, String>>,
    ) -> Json<Value> {
        let refresh = body.get("grant_type").is_some_and(|v| v == "refresh_token");
        p.tokens.lock().unwrap().push(body);
        Json(
            json!({"access_token":if refresh{"test-refreshed-secret"}else{"test-access-secret"},"token_type":"Bearer","expires_in":if refresh{3600}else{1},"refresh_token":"test-refresh-secret","scope":"read"}),
        )
    }
    async fn mcp(State(p): State<Provider>, headers: HeaderMap) -> axum::response::Response {
        if headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| !v.starts_with("Bearer test-"))
        {
            return (
                StatusCode::UNAUTHORIZED,
                [(
                    "www-authenticate",
                    format!(
                        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
                        p.base
                    ),
                )],
                Json(json!({"error":"unauthorized"})),
            )
                .into_response();
        }
        Json(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"test","version":"1"}}})).into_response()
    }
    let app = Router::new()
        .route("/.well-known/oauth-protected-resource", get(resource))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource))
        .route("/.well-known/oauth-authorization-server", get(metadata))
        .route("/.well-known/openid-configuration", get(metadata))
        .route("/register", post(register))
        .route("/token", post(token))
        .route("/mcp", post(mcp))
        .with_state(provider.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fake { provider, task }
}
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
async fn api() -> Api {
    api_with(ScriptedSession::always(Reply::ok(complete()))).await
}
async fn api_with(session: Arc<dyn limina_engines::AgentSession>) -> Api {
    let database = std::env::var("LIMINA_TEST_POSTGRES_URL").ok();
    if let Some(url) = &database {
        let db = limina_store::Db::connect(url).await.unwrap();
        let mut tx = db.begin_write().await.unwrap();
        tx.execute_script("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
        tx.commit().await.unwrap();
        db.upgrade().await.unwrap();
        db.close().await;
    }
    let test = runtime_with(factory(session), |config| {
        if let Some(database) = database {
            config.database_url = database;
        }
    })
    .await;
    create_project(test.runtime.operations(), "demo").await;
    add_member(test.runtime.operations(), "demo", &viewer(), "VIEWER").await;
    Api::new(test, ui_tokens()).await
}
const OWNER: &[(&str, &str)] =
    &[("authorization", "Bearer owner-token"), ("idempotency-key", "connection-key")];

#[tokio::test]
async fn email_shares_bind_on_verified_sign_in_through_the_http_api() {
    let test = runtime(ScriptedSession::always(Reply::ok(complete()))).await;
    create_project(test.runtime.operations(), "private").await;
    let reader = limina_core::Principal::new("reader", "Reader")
        .with_email(Some("reader@example.test".into()));
    let auth = token_auth(HashMap::from([
        ("owner-token".into(), owner()),
        ("unverified".into(), reader.clone()),
        ("verified".into(), reader.email_verified(true)),
    ]));
    let api = Api::new(test, auth).await;
    api.put(
        "/v1/projects/private/sharing",
        OWNER,
        &json!({"email":"Reader@Example.Test","role":"VIEWER"}),
    )
    .await
    .expect(200);
    let hidden =
        api.get("/v1/projects", &[("authorization", "Bearer unverified")]).await.expect(200).json();
    assert_eq!(hidden["total"], 0);
    let visible =
        api.get("/v1/projects", &[("authorization", "Bearer verified")]).await.expect(200).json();
    assert_eq!(visible["items"][0]["role"], "VIEWER");
    api.put(
        "/v1/projects/private/sharing",
        &[("authorization", "Bearer verified")],
        &json!({"email":"other@example.test","role":"EDITOR"}),
    )
    .await
    .expect(403);
    api.put(
        "/v1/projects/private/sharing",
        OWNER,
        &json!({"email":"later@example.test","role":"VIEWER"}),
    )
    .await
    .expect(200);
    api.delete("/v1/projects/private/sharing?email=later@example.test", OWNER).await.expect(200);
    api.close().await;
}

#[tokio::test]
async fn credentials_are_private_and_projects_opt_in() {
    let _serial = SERIAL.lock().await;
    let fake = fake().await;
    let session =
        ScriptedSession::always(Reply::ok(complete()).event(limina_engines::RuntimeEvent::new(
            "runtime.codex",
            "test-access-secret",
            detail(json!({"text":"test-access-secret"})),
        )));
    let api = api_with(session.clone()).await;
    let body = json!({"name":"Notes","server_url":format!("{}/mcp",fake.provider.base),"auth":"bearer","secret":"test-access-secret"});
    let created = api.post("/v1/connections", OWNER, &body).await.expect(200).json();
    let id = created["id"].as_str().unwrap();
    assert!(!created.to_string().contains("test-access-secret"));
    assert_eq!(api.post("/v1/connections", OWNER, &body).await.expect(200).json()["id"], id);
    let path = format!("/v1/connections/{id}");
    api.get(&path, &[("authorization", "Bearer admin-token")]).await.expect(404);
    let connect = api.post(&format!("{path}/connect"), OWNER, &json!({})).await;
    assert_eq!(connect.expect(200).json()["connection"]["status"], "CONNECTED");
    assert!(api.test.runtime.supervisor().connections().resolve("demo").await.unwrap().is_empty());
    api.put(
        &format!("/v1/projects/demo/connections/{id}"),
        &[("authorization", "Bearer viewer-token")],
        &json!({}),
    )
    .await
    .expect(403);
    let enabled = api
        .put(&format!("/v1/projects/demo/connections/{id}"), OWNER, &json!({}))
        .await
        .expect(200)
        .json();
    assert_eq!(enabled["active"], true);
    let servers = api.test.runtime.supervisor().connections().resolve("demo").await.unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].bearer_token.as_deref(), Some("test-access-secret"));
    let view = api
        .get("/v1/projects/demo/connections", &[("authorization", "Bearer viewer-token")])
        .await
        .expect(200)
        .json();
    assert_eq!(view["can_enable"], false);
    assert!(!view.to_string().contains("test-access-secret"));
    api.post(
        "/v1/projects/demo/actions/start",
        &[("authorization", "Bearer owner-token"), ("idempotency-key", "turn-check")],
        &json!({}),
    )
    .await
    .expect(200);
    api.test.runtime.supervisor().join("demo").await;
    let turns = session.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].mcp_servers[0].bearer_token.as_deref(), Some("test-access-secret"));
    let events = api.get("/v1/projects/demo/events", OWNER).await.expect(200).json();
    assert!(!events.to_string().contains("test-access-secret"));
    api.post(&format!("{path}/disconnect"), OWNER, &json!({})).await.expect(200);
    assert!(api.test.runtime.supervisor().connections().resolve("demo").await.unwrap().is_empty());
    assert!(
        api.get("/v1/projects/demo/connections", OWNER).await.expect(200).json()["connections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    api.close().await;
}

#[tokio::test]
async fn oauth_is_browser_bound_single_use_durable_and_refreshes_before_a_turn() {
    let _serial = SERIAL.lock().await;
    let fake = fake().await;
    let api = api().await;
    let body = json!({"name":"OAuth notes","server_url":format!("{}/mcp",fake.provider.base),"auth":"oauth"});
    let row = api.post("/v1/connections", OWNER, &body).await.expect(200).json();
    let id = row["id"].as_str().unwrap();
    let response = api
        .post(&format!("/v1/connections/{id}/connect"), OWNER, &json!({"return_to":"/connections"}))
        .await;
    let outcome = response.expect(200).json();
    let url = url::Url::parse(outcome["authorization_url"].as_str().unwrap()).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["resource"], format!("{}/mcp", fake.provider.base));
    let cookie = response.header("set-cookie").unwrap().split(';').next().unwrap();
    let callback = format!(
        "/auth/mcp/callback?state={}&code=test-code&iss={}",
        query["state"], fake.provider.base
    );
    let wrong = api.get(&callback, OWNER).await;
    assert!(wrong.expect(303).header("location").unwrap().contains("browser"));
    // A new Connections object has no in-memory OAuth session and can finish from the DB.
    let connections =
        limina_server::app::connections::Connections::new(api.test.runtime.service().clone());
    assert_eq!(connections.list(&owner()).await.unwrap().len(), 1);
    let headers = [("authorization", "Bearer owner-token"), ("cookie", cookie)];
    api.get(&callback, &headers).await.expect(303);
    assert_eq!(
        api.get(&format!("/v1/connections/{id}"), OWNER).await.expect(200).json()["status"],
        "CONNECTED"
    );
    assert!(
        api.get(&callback, &headers)
            .await
            .expect(303)
            .header("location")
            .unwrap()
            .contains("expired")
    );
    api.put(&format!("/v1/projects/demo/connections/{id}"), OWNER, &json!({})).await.expect(200);
    let servers = connections.resolve("demo").await.unwrap();
    assert_eq!(servers[0].bearer_token.as_deref(), Some("test-refreshed-secret"));
    let requests = fake.provider.tokens.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains_key("code_verifier"));
    assert_eq!(requests[1]["grant_type"], "refresh_token");
    let mut tx = api.test.runtime.db().begin().await.unwrap();
    let encrypted: String = tx
        .scalar("SELECT secret_ciphertext FROM mcp_connections WHERE id=$1", args![id])
        .await
        .unwrap();
    assert!(!encrypted.contains("test-access-secret"));
    tx.commit().await.unwrap();
    api.close().await;
}

#[tokio::test]
async fn unsafe_urls_and_non_member_admins_are_refused() {
    let _serial = SERIAL.lock().await;
    let api = api().await;
    for server in [
        "http://example.com/mcp",
        "https://169.254.169.254/",
        "https://user:password@example.com/mcp",
    ] {
        api.post(
            "/v1/connections",
            OWNER,
            &json!({"name":"Bad","server_url":server,"auth":"none"}),
        )
        .await
        .expect(422);
    }
    for path in ["/v1/projects/demo", "/v1/projects/demo/sharing", "/v1/projects/demo/connections"]
    {
        api.get(path, &[("authorization", "Bearer admin-token")]).await.expect(403);
    }
    api.get("/v1/projects", &[("authorization", "Bearer admin-token")]).await.expect(200);
    api.close().await;
}
