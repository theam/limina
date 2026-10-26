//! Static tokens, the open local mode, configuration from the environment, and the request
//! entry point. Ports `tests/test_cloud_auth.py` (local authentication) and the API's token checks.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod auth_support;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use auth_support::TestClient;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::routing::get;
use axum::{Json, Router};
use limina_server::auth::{
    AdminCaller, Auth, AuthConfig, AuthOptions, Authenticator, Caller, ChainedAuthenticator,
    ConsoleMode, LocalAuthenticator, RequestCredentials, TrustedProxies,
    machine_token_authenticator,
};
use serde_json::{Value, json};

fn config(pairs: &[(&str, &str)]) -> Result<AuthConfig, String> {
    let env: HashMap<String, String> =
        pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect();
    AuthConfig::from_lookup(|name| env.get(name).cloned()).map_err(|error| error.to_string())
}

async fn auth(pairs: &[(&str, &str)]) -> Result<Auth, String> {
    Auth::from_config(config(pairs)?).await.map_err(|error| error.to_string())
}

/// The API surface these tests need: `/v1/me` (as the Python route answers it) and an
/// administrator-only route.
fn app(auth: &Auth) -> Router {
    async fn me(Caller(principal): Caller) -> Json<Value> {
        Json(json!({
            "subject": principal.subject,
            "display_name": principal.display_name,
            "email": principal.email,
            "instance_admin": principal.instance_admin,
            "auth_mode": principal.auth_mode,
        }))
    }
    async fn admin(AdminCaller(principal): AdminCaller) -> Json<Value> {
        Json(json!({"subject": principal.subject}))
    }
    Router::new()
        .route("/livez", get(|| async { Json(json!({"ok": true})) }))
        .route("/v1/me", get(me))
        .route("/v1/runtime/engines/codex/auth", get(admin))
        .merge(auth.router())
        .layer(auth.layer())
}

fn credentials(bearer: Option<&str>, actor: Option<&str>) -> RequestCredentials {
    let mut headers = HeaderMap::new();
    if let Some(bearer) = bearer {
        headers
            .insert("authorization", HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap());
    }
    if let Some(actor) = actor {
        headers.insert("x-limina-actor", HeaderValue::from_str(actor).unwrap());
    }
    RequestCredentials::from_headers(
        &Method::GET,
        &Uri::from_static("/v1/me"),
        &headers,
        Some(IpAddr::from([192, 0, 2, 10])),
        &TrustedProxies::default(),
    )
}

// ─── test_cloud_auth.py: LocalAuthenticationTests ─────────────────────────────

#[tokio::test]
async fn project_and_instance_tokens_have_distinct_authority() {
    let authenticator =
        LocalAuthenticator::new(Some("project-token".into()), Some("admin-token".into())).unwrap();
    let project_user =
        authenticator.authenticate(Some("project-token"), Some("researcher")).await.unwrap();
    let administrator =
        authenticator.authenticate(Some("admin-token"), Some("operator")).await.unwrap();

    assert!(project_user.project_admin);
    assert!(!project_user.instance_admin);
    assert!(administrator.project_admin);
    assert!(administrator.instance_admin);
    assert_eq!(
        (project_user.actor(), project_user.auth_mode.as_str()),
        ("researcher".into(), "local")
    );
    assert_eq!(administrator.subject, "local:operator");
}

#[test]
fn equal_project_and_instance_tokens_are_rejected() {
    assert!(LocalAuthenticator::new(Some("same".into()), Some("same".into())).is_err());
}

#[tokio::test]
async fn non_ascii_invalid_token_is_rejected_without_a_type_error() {
    let authenticator =
        LocalAuthenticator::new(Some("project-token".into()), Some("admin-token".into())).unwrap();
    let error = authenticator.authenticate(Some("not-the-token-é"), None).await.unwrap_err();
    assert_eq!((error.http_status(), error.code()), (401, "authentication_required"));
}

// ─── More static-token behavior ───────────────────────────────────────────────

#[tokio::test]
async fn token_modes_default_the_actor_and_refuse_anonymous_callers() {
    let project = LocalAuthenticator::new(Some("project-token".into()), None).unwrap();
    assert_eq!(
        project.authenticate(Some("project-token"), None).await.unwrap().actor(),
        "local-user"
    );
    assert!(project.authenticate(None, None).await.is_err());
    assert!(project.authenticate(Some("wrong"), None).await.is_err());

    let admin_only = LocalAuthenticator::new(None, Some("admin-token".into())).unwrap();
    assert!(admin_only.authenticate(None, Some("x")).await.is_err());
    let admin = admin_only.authenticate(Some("admin-token"), Some("  ")).await.unwrap();
    assert_eq!((admin.actor(), admin.instance_admin), ("local-admin".into(), true));

    let open = LocalAuthenticator::open();
    let anyone = open.authenticate(None, Some("ada")).await.unwrap();
    assert_eq!((anyone.subject.as_str(), anyone.instance_admin), ("local:ada", true));
    assert!(!open.requires_credentials());
}

#[tokio::test]
async fn machine_tokens_exist_only_when_configured() {
    assert!(machine_token_authenticator(None, None).unwrap().is_none());
    assert!(machine_token_authenticator(Some(""), Some("")).unwrap().is_none());
    assert!(machine_token_authenticator(Some("same"), Some("same")).is_err());
    let machine = machine_token_authenticator(None, Some("admin-token")).unwrap().unwrap();
    assert!(machine.authenticate(Some("admin-token"), None).await.unwrap().instance_admin);

    assert!(ChainedAuthenticator::new("workos", Vec::new()).is_err());
    let chain = ChainedAuthenticator::new("workos", vec![Arc::new(machine)]).unwrap();
    assert_eq!(chain.mode(), "workos");
    for bearer in [None, Some(""), Some("wrong")] {
        assert_eq!(chain.authenticate(bearer, None).await.unwrap_err().http_status(), 401);
    }
}

// ─── Configuration from the environment ───────────────────────────────────────

#[tokio::test]
async fn the_environment_selects_the_mode() {
    let error = auth(&[]).await.err().unwrap();
    assert!(error.starts_with("Configure LIMINA_API_TOKEN"), "{error}");

    let open = auth(&[("LIMINA_ALLOW_INSECURE_NO_AUTH", "TRUE")]).await.unwrap();
    assert_eq!((open.mode(), open.console_mode()), ("local", ConsoleMode::Open));

    // An empty token is unset, as everywhere else in the configuration.
    let empty = auth(&[("LIMINA_ALLOW_INSECURE_NO_AUTH", "yes"), ("LIMINA_API_TOKEN", "")]).await;
    assert_eq!(empty.unwrap().console_mode(), ConsoleMode::Open);

    let token = auth(&[("LIMINA_API_TOKEN", "project-token")]).await.unwrap();
    assert_eq!((token.mode(), token.console_mode()), ("local", ConsoleMode::Token));

    // As in the Python runtime, an admin token alone still needs the explicit opt-in.
    assert!(auth(&[("LIMINA_ADMIN_API_TOKEN", "admin-token")]).await.is_err());
    let admin_only =
        auth(&[("LIMINA_ADMIN_API_TOKEN", "admin-token"), ("LIMINA_ALLOW_INSECURE_NO_AUTH", "1")])
            .await
            .unwrap();
    assert_eq!(admin_only.console_mode(), ConsoleMode::Token);

    let same = auth(&[("LIMINA_API_TOKEN", "same"), ("LIMINA_ADMIN_API_TOKEN", "same")]).await;
    assert!(same.err().unwrap().contains("must be different"));
}

#[test]
fn invalid_combinations_and_values_are_refused() {
    let half_oidc = config(&[("LIMINA_OIDC_ISSUER", "https://identity.example.test")]);
    assert!(half_oidc.err().unwrap().contains("must be set together"));
    let both = config(&[
        ("WORKOS_CLIENT_ID", "client_test123"),
        ("WORKOS_API_KEY", "sk_test"),
        ("WORKOS_COOKIE_PASSWORD", auth_support::PASSWORD),
        ("LIMINA_OIDC_ISSUER", "https://identity.example.test"),
    ]);
    assert!(both.err().unwrap().contains("not both"));
    assert!(config(&[("LIMINA_AUTH_FAILURE_LIMIT", "ten")]).is_err());
    assert!(config(&[("LIMINA_AUTH_FAILURE_WINDOW_SECONDS", "")]).is_err());
    assert!(config(&[("FORWARDED_ALLOW_IPS", "proxy.example.com")]).is_err());
    let oidc = config(&[
        ("LIMINA_OIDC_ISSUER", " https://identity.example.test "),
        ("LIMINA_OIDC_AUDIENCE", "limina-api"),
        ("LIMINA_OIDC_ALGORITHMS", "RS256, ES256,"),
        ("LIMINA_OIDC_LEEWAY_SECONDS", " 45 "),
    ])
    .unwrap()
    .oidc
    .unwrap();
    assert_eq!(oidc.issuer, "https://identity.example.test");
    assert_eq!(oidc.algorithms, ["RS256", "ES256"]);
    assert_eq!((oidc.leeway_seconds, oidc.admin_value.as_str()), (45, "admin"));
    let limits =
        config(&[("LIMINA_AUTH_FAILURE_LIMIT", "3"), ("LIMINA_GLOBAL_AUTH_FAILURE_LIMIT", "7")])
            .unwrap()
            .limits;
    assert_eq!(
        (limits.client.limit, limits.global.limit, limits.client.window_seconds),
        (3, 7, 60)
    );
}

#[test]
fn non_local_binds_need_authentication() {
    let none = config(&[]).unwrap();
    for host in ["127.0.0.1", "localhost", "::1"] {
        assert!(none.check_bind(host).is_ok());
    }
    assert!(none.check_bind("0.0.0.0").is_err());
    assert!(config(&[("LIMINA_API_TOKEN", "t")]).unwrap().check_bind("0.0.0.0").is_ok());
    assert!(config(&[("LIMINA_ALLOW_INSECURE_NO_AUTH", "1")]).unwrap().check_bind("::").is_ok());
}

// ─── The request entry point ──────────────────────────────────────────────────

#[tokio::test]
async fn actor_hints_over_200_characters_are_rejected_before_authentication() {
    let auth =
        Auth::new(Arc::new(LocalAuthenticator::open()), None, AuthOptions::default()).unwrap();
    let long = "a".repeat(201);
    let error = auth.authenticate(&credentials(None, Some(&long))).await.unwrap_err();
    assert_eq!((error.http_status(), error.code()), (422, "invalid_request"));
    assert_eq!(error.details["errors"][0]["loc"], json!(["header", "x-limina-actor"]));
    let fits = auth.authenticate(&credentials(None, Some(&"a".repeat(200)))).await.unwrap();
    assert_eq!(fits.principal.actor().len(), 200);
}

#[tokio::test]
async fn token_mode_routes_need_the_right_token() {
    let auth =
        auth(&[("LIMINA_API_TOKEN", "project-token"), ("LIMINA_ADMIN_API_TOKEN", "admin-token")])
            .await
            .unwrap();
    let mut client = TestClient::new(app(&auth));
    assert_eq!(client.get("/livez").await.status, StatusCode::OK);
    assert_eq!(client.get("/auth/config").await.json(), json!({"mode": "token"}));
    assert_eq!(client.get("/auth/login").await.status, StatusCode::NOT_FOUND);

    let anonymous = client.get("/v1/me").await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.json()["error"]["code"], "authentication_required");

    let project = [("authorization", "Bearer project-token"), ("x-limina-actor", "researcher")];
    let me = client.send(Method::GET, "/v1/me", &project).await.json();
    assert_eq!(
        (me["subject"].as_str(), me["auth_mode"].as_str()),
        (Some("local:researcher"), Some("local"))
    );

    // test_cloud_api.py: runtime administration requires the distinct admin token.
    let path = "/v1/runtime/engines/codex/auth";
    let denied = client.send(Method::GET, path, &project).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.json()["error"]["message"], "Instance administrator access is required.");
    let admin = client.send(Method::GET, path, &[("authorization", "Bearer admin-token")]).await;
    assert_eq!(admin.status, StatusCode::OK);
}

#[tokio::test]
async fn caller_without_the_layer_is_a_wiring_error() {
    async fn me(Caller(principal): Caller) -> String {
        principal.subject
    }
    let mut client = TestClient::new(Router::new().route("/v1/me", get(me)));
    let response = client.get("/v1/me").await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.json()["error"]["code"], "transport_error");
}

#[tokio::test]
async fn route_groups_can_require_a_caller() {
    let auth = auth(&[("LIMINA_API_TOKEN", "project-token")]).await.unwrap();
    let guarded = Router::new()
        .route("/v1/ping", get(|| async { "pong" }))
        .route_layer(axum::middleware::from_extractor::<Caller>());
    let app =
        Router::new().merge(guarded).route("/livez", get(|| async { "ok" })).layer(auth.layer());
    let mut client = TestClient::new(app);
    assert_eq!(client.get("/v1/ping").await.status, StatusCode::UNAUTHORIZED);
    let bearer = [("authorization", "Bearer project-token")];
    assert_eq!(client.send(Method::GET, "/v1/ping", &bearer).await.status, StatusCode::OK);
    assert_eq!(client.get("/livez").await.status, StatusCode::OK);
}
