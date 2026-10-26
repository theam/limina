//! WorkOS AuthKit sign-in. Ports all of `tests/test_cloud_workos_auth.py` against a fake WorkOS
//! API on a loopback port and RSA keys generated for the test run.
//!
//! The Python browser-flow tests call `/v1/me` and `POST /v1/projects` of the full API. Here the
//! same assertions run against a router made of the `/auth/*` routes and two stand-in routes that
//! extract a [`Caller`] (`/v1/me` answers like the Python route; `POST /v1/projects` returns 201).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod auth_support;

use std::collections::HashMap;
use std::sync::Arc;

use auth_support::{CLIENT_ID, FakeIdp, PASSWORD, TestClient, TestResponse, query};
use axum::http::{Method, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use limina_server::auth::workos::SESSION_COOKIE;
use limina_server::auth::{
    Auth, AuthConfig, AuthOptions, Authenticator, Caller, ChainedAuthenticator, LocalAuthenticator,
    SessionSealer, WorkOsAuth, WorkOsSettings, safe_return_path,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn settings(idp: &FakeIdp) -> WorkOsSettings {
    let mut settings = WorkOsSettings::new(CLIENT_ID, "sk_test", PASSWORD).unwrap();
    settings.api_base_url.clone_from(&idp.base_url);
    settings
}

/// The Python tests' `make_auth`: a WorkOS client whose API is the fake provider.
fn make_auth(idp: &FakeIdp, change: impl FnOnce(&mut WorkOsSettings)) -> Arc<WorkOsAuth> {
    let mut settings = settings(idp);
    change(&mut settings);
    Arc::new(WorkOsAuth::new(settings, reqwest::Client::new()).unwrap())
}

#[tokio::test]
async fn sharing_needs_explicit_verification_of_the_email_source() {
    let idp = FakeIdp::start().await;
    let auth = make_auth(&idp, |_| {});
    let claims = json!({"sub":"user_reader","email":"reader@example.test"});
    assert!(!auth.principal(claims.as_object().unwrap(), None).unwrap().email_verified);
    let claims = json!({"sub":"user_reader","email":"reader@example.test","email_verified":true});
    assert!(auth.principal(claims.as_object().unwrap(), None).unwrap().email_verified);
    let profile = json!({"email":"other@example.test","email_verified":false});
    let person = auth.principal(claims.as_object().unwrap(), Some(&profile)).unwrap();
    assert_eq!(person.verified_email(), None);
}

// ─── WorkOsPrimitivesTests ────────────────────────────────────────────────────

#[test]
fn sealed_payloads_are_confidential_and_tamper_evident() {
    let sealer = SessionSealer::new(PASSWORD, "session").unwrap();
    let sealed = sealer.seal(&json!({"access_token": "secret-value"})).unwrap();
    assert!(!sealed.contains("secret-value"));
    assert_eq!(
        Value::Object(sealer.unseal(&sealed).unwrap()),
        json!({"access_token": "secret-value"})
    );
    assert!(SessionSealer::new(&"q".repeat(40), "session").unwrap().unseal(&sealed).is_none());
    assert!(SessionSealer::new(PASSWORD, "login-flow").unwrap().unseal(&sealed).is_none());
    assert!(sealer.unseal(&format!("{}AAAA", &sealed[..sealed.len() - 4])).is_none());
    assert!(sealer.unseal("not-a-cookie").is_none());
    assert!(sealer.unseal("").is_none());
}

/// A cookie sealed by the Python runtime (`SessionSealer("p" * 40, purpose="session")`) opens
/// here, so sessions survive the switch to the Rust runtime.
#[test]
fn cookies_sealed_by_the_python_runtime_still_open() {
    const PYTHON_SEALED: &str = "Abtqty0q9gi7agZGMjaymOzY81cF0qaBy9nyeqKh7-K0Z_FwD8D61a8hg-lxa-Bz\
        LURy0VKgYYEWJtnPzSk3DLzqSi4iv0B2K0OaGT81Weuw_pGzCts4y-ZdOOWA-A48u1pDq2vo5czx0RApP4GQO6-b7WZ\
        q9cEJbee5AxW_apOcZAFMZZwPc5lrbYILZd5R7y8-ZCjRZ6-QUx72EUev2PCI3-jJB8neWcWIhqj64t3ARZKBroAvP-hNLRI";
    let sealer = SessionSealer::new(PASSWORD, "session").unwrap();
    let payload = sealer.unseal(PYTHON_SEALED).unwrap();
    assert_eq!(
        Value::Object(payload),
        json!({
            "access_token": "fake-access",
            "refresh_token": "fake-refresh",
            "user": {"id": "user_01", "email": "ada@example.com", "first_name": "Ada", "last_name": null},
        })
    );
    // The format: version byte, 12-byte nonce, AES-GCM ciphertext and tag, base64url unpadded.
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(PYTHON_SEALED).unwrap();
    assert_eq!(raw[0], 0x01);
    let ours = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(sealer.seal(&json!({"a": 1})).unwrap())
        .unwrap();
    assert_eq!((ours[0], ours.len()), (0x01, 1 + 12 + br#"{"a":1}"#.len() + 16));
}

#[test]
fn return_paths_stay_on_this_origin() {
    assert_eq!(safe_return_path(Some("/projects/demo?x=1")), "/projects/demo?x=1");
    for value in [
        None,
        Some(""),
        Some("https://evil.example"),
        Some("//evil.example"),
        Some("/\\evil"),
        Some("projects"),
    ] {
        assert_eq!(safe_return_path(value), "/", "{value:?}");
    }
}

#[test]
fn settings_reject_unsafe_configuration() {
    assert!(WorkOsSettings::new("nope", "sk", PASSWORD).is_err());
    assert!(WorkOsSettings::new(CLIENT_ID, "sk", "short").is_err());
    assert!(WorkOsSettings::new(CLIENT_ID, "", PASSWORD).is_err());
    let mut env: HashMap<&str, &str> = HashMap::from([
        ("WORKOS_CLIENT_ID", CLIENT_ID),
        ("WORKOS_API_KEY", "sk_test"),
        ("WORKOS_COOKIE_PASSWORD", PASSWORD),
        ("WORKOS_API_HOSTNAME", "http://auth.example.com"),
    ]);
    let load = |env: &HashMap<&str, &str>| {
        AuthConfig::from_lookup(|name| env.get(name).map(|value| (*value).to_owned()))
    };
    assert!(load(&env).is_err());
    env.insert("WORKOS_API_HOSTNAME", "http://127.0.0.1:9000");
    let settings = load(&env).unwrap().workos.unwrap();
    assert_eq!(settings.jwks_url(), format!("http://127.0.0.1:9000/sso/jwks/{CLIENT_ID}"));
    assert_eq!(
        settings.expected_issuers(),
        [format!("http://127.0.0.1:9000/user_management/{CLIENT_ID}")]
    );

    env.extend([
        ("LIMINA_WORKOS_ISSUER", " https://a.example.com/, https://b.example.com ,"),
        ("LIMINA_PUBLIC_URL", "https://limina.example.com/"),
        ("LIMINA_WORKOS_ADMIN_PERMISSION", " "),
        ("LIMINA_WORKOS_LEEWAY_SECONDS", "301"),
    ]);
    assert!(load(&env).is_err());
    env.insert("LIMINA_WORKOS_LEEWAY_SECONDS", "0");
    let settings = load(&env).unwrap().workos.unwrap();
    assert_eq!(settings.issuers, ["https://a.example.com", "https://b.example.com"]);
    assert_eq!(settings.public_url.as_deref(), Some("https://limina.example.com"));
    assert_eq!(settings.admin_permission, "");
    assert!(!format!("{settings:?}").contains("sk_test"));
}

#[tokio::test]
async fn token_policy_pins_issuer_organization_and_permissions() {
    let idp = FakeIdp::start().await;
    let auth = make_auth(&idp, |settings| {
        settings.organization_id = Some("org_01".into());
        settings.required_permission = Some("limina:access".into());
        settings.admin_permission = "limina:admin".into();
    });
    let token = |change: fn(auth_support::AccessToken) -> auth_support::AccessToken| {
        change(idp.token()).signed()
    };
    let admin = auth
        .authenticate(Some(&token(|t| t.permissions(&["limina:access", "limina:admin"]))), None)
        .await
        .unwrap();
    assert_eq!(
        (admin.subject.as_str(), admin.auth_mode.as_str(), admin.instance_admin),
        ("user_01", "workos", true)
    );
    let member =
        auth.authenticate(Some(&token(|t| t.permissions(&["limina:access"]))), None).await.unwrap();
    assert!(!member.instance_admin);
    assert!(!member.project_admin);

    let denied = auth.authenticate(Some(&token(|t| t.permissions(&[]))), None).await.unwrap_err();
    assert_eq!(
        (denied.http_status(), denied.details["reason"].as_str()),
        (403, Some("permission"))
    );
    assert_eq!(denied.details["required_permission"], "limina:access");
    let other_org = token(|t| t.permissions(&["limina:access"]).org(Some("org_other")));
    let denied = auth.authenticate(Some(&other_org), None).await.unwrap_err();
    assert_eq!(
        (denied.http_status(), denied.details["reason"].as_str()),
        (403, Some("organization"))
    );
    let other_issuer = token(|t| t.issuer("https://api.workos.com/other"));
    assert_eq!(auth.authenticate(Some(&other_issuer), None).await.unwrap_err().http_status(), 401);
    let expired = token(|t| t.expires_in(-120));
    assert_eq!(auth.authenticate(Some(&expired), None).await.unwrap_err().http_status(), 401);
}

#[tokio::test]
async fn issuer_and_keys_come_from_workos_discovery() {
    let idp = FakeIdp::start().await;
    let custom = "https://auth.example.com/user_management/client_default";
    let jwks_uri = format!("{}/custom/jwks", idp.base_url);
    idp.with(|state| state.discovery = Some(json!({"issuer": custom, "jwks_uri": jwks_uri})));
    let auth = make_auth(&idp, |_| {});
    assert_eq!(
        auth.authenticate(Some(&idp.token().issuer(custom).signed()), None).await.unwrap().subject,
        "user_01"
    );
    // The API-host issuer is not this client's issuer.
    assert!(auth.authenticate(Some(&idp.token().signed()), None).await.is_err());
    // Discovery is a GET (once), never a token exchange; keys come from the discovered URL.
    assert!(idp.requests().is_empty());
    let discovery = format!("/user_management/{CLIENT_ID}/.well-known/openid-configuration");
    assert_eq!(idp.gets(), [discovery, "/custom/jwks".to_owned()]);
}

#[tokio::test]
async fn explicit_issuer_skips_discovery_and_insecure_keys_are_ignored() {
    let idp = FakeIdp::start().await;
    idp.with(|state| {
        state.discovery = Some(
            json!({"issuer": "https://other.example.com", "jwks_uri": "http://evil.example/jwks"}),
        );
    });
    let pinned =
        make_auth(&idp, |settings| settings.issuers = vec!["https://pinned.example.com".into()]);
    assert_eq!(pinned.verification().await.issuers(), ["https://pinned.example.com"]);
    assert!(idp.gets().is_empty());
    let insecure = make_auth(&idp, |_| {});
    assert_eq!(insecure.verification().await.issuers(), [idp.issuer()]);
    assert_eq!(
        insecure.verification().await.jwks().url(),
        format!("{}/sso/jwks/{CLIENT_ID}", idp.base_url)
    );
}

#[tokio::test]
async fn machine_tokens_chain_with_workos_without_anonymous_access() {
    let idp = FakeIdp::start().await;
    let machine = LocalAuthenticator::new(Some("machine-token".into()), None).unwrap();
    let chain =
        ChainedAuthenticator::new("workos", vec![Arc::new(machine), make_auth(&idp, |_| {})])
            .unwrap();
    assert_eq!(chain.authenticate(Some("machine-token"), None).await.unwrap().auth_mode, "local");
    assert_eq!(
        chain.authenticate(Some(&idp.token().signed()), None).await.unwrap().auth_mode,
        "workos"
    );
    for value in [None, Some(""), Some("wrong")] {
        let error = chain.authenticate(value, None).await.unwrap_err();
        assert_eq!(error.code(), "authentication_required", "{value:?}");
    }
}

// ─── WorkOsBrowserFlowTests ───────────────────────────────────────────────────

struct Browser {
    idp: FakeIdp,
    workos: Arc<WorkOsAuth>,
    client: TestClient,
}

/// The stand-ins for the API routes the Python tests call.
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
    async fn create_project(Caller(principal): Caller) -> (StatusCode, Json<Value>) {
        (StatusCode::CREATED, Json(json!({"owner": principal.subject})))
    }
    Router::new()
        .route("/v1/me", get(me))
        .route("/v1/projects", post(create_project))
        .merge(auth.router())
        .layer(auth.layer())
}

impl Browser {
    /// The Python `setUp`: WorkOS requiring `limina:access`, no machine tokens.
    async fn start() -> Self {
        Self::with_settings(|settings| settings.required_permission = Some("limina:access".into()))
            .await
    }

    async fn with_settings(change: impl FnOnce(&mut WorkOsSettings)) -> Self {
        let idp = FakeIdp::start().await;
        let workos = make_auth(&idp, change);
        let auth = Auth::with_workos(workos.clone(), None, None, AuthOptions::default()).unwrap();
        Self { idp, workos, client: TestClient::new(app(&auth)) }
    }

    async fn sign_in(&mut self, return_to: &str) -> TestResponse {
        let start = self.client.get(&format!("/auth/login?return_to={return_to}")).await;
        assert_eq!(start.status, StatusCode::SEE_OTHER);
        let state = query(start.location())["state"].clone();
        self.client.get(&format!("/auth/callback?code=code_01&state={state}")).await
    }

    async fn me(&mut self) -> TestResponse {
        self.client.get("/v1/me").await
    }
}

#[tokio::test]
async fn config_reports_workos_and_nothing_is_anonymous() {
    let mut browser = Browser::start().await;
    assert_eq!(browser.client.get("/auth/config").await.json(), json!({"mode": "workos"}));
    assert_eq!(browser.me().await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(browser.client.get("/auth/unknown").await.status, StatusCode::NOT_FOUND);
    assert_eq!(browser.client.get("/auth/logout").await.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn login_redirects_to_authkit_with_state_and_pkce() {
    let mut browser = Browser::start().await;
    let response = browser.client.get("/auth/login?return_to=//evil.example").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    let location = response.location().to_owned();
    let target = location.split('?').next().unwrap();
    assert_eq!(target, format!("{}/user_management/authorize", browser.idp.base_url));
    let params = query(&location);
    assert_eq!(params["client_id"], CLIENT_ID);
    assert_eq!(params["provider"], "authkit");
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["redirect_uri"], "http://testserver/auth/callback");
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(!params["state"].is_empty());
    let cookie = &response.set_cookies()[0];
    assert!(cookie.starts_with("limina_auth_flow="));
    for attribute in ["HttpOnly", "SameSite=lax", "Path=/auth", "Max-Age=600"] {
        assert!(cookie.contains(attribute), "{cookie}");
    }
    assert!(!cookie.contains("Secure"));

    // The verifier sent at the exchange matches the challenge, and the unsafe return path fell
    // back to the console root.
    let callback =
        browser.client.get(&format!("/auth/callback?code=code_01&state={}", params["state"])).await;
    assert_eq!(callback.location(), "/");
    let verifier = browser.idp.requests()[0].body["code_verifier"].as_str().unwrap().to_owned();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    assert_eq!(challenge, params["code_challenge"]);
}

#[tokio::test]
async fn login_moves_to_the_public_url_before_setting_cookies() {
    let mut browser = Browser::with_settings(|settings| {
        settings.public_url = Some("http://localhost:7433".into())
    })
    .await;
    let response = browser.client.get("/auth/login?return_to=/projects").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.location(), "http://localhost:7433/auth/login?return_to=%2Fprojects");
    assert!(response.set_cookies().is_empty());
}

#[tokio::test]
async fn https_public_urls_get_secure_cookies() {
    let mut browser = Browser::with_settings(|settings| {
        settings.public_url = Some("https://limina.example.com".into());
        settings.organization_id = Some("org_01".into());
    })
    .await;
    // A trusted local proxy terminates TLS for the public host. Forwarding headers from any
    // other peer are ignored, which sends the browser to the public URL first.
    let forwarded = [("host", "limina.example.com"), ("x-forwarded-proto", "https")];
    browser.client.peer = Some("203.0.113.7:50000".parse().unwrap());
    let untrusted = browser.client.send(Method::GET, "/auth/login", &forwarded).await;
    assert_eq!(untrusted.location(), "https://limina.example.com/auth/login");
    browser.client.peer = Some("127.0.0.1:50000".parse().unwrap());
    let start = browser.client.send(Method::GET, "/auth/login", &forwarded).await;
    assert_eq!(start.status, StatusCode::SEE_OTHER, "{:?}", start.headers);
    let params = query(start.location());
    assert_eq!(params["redirect_uri"], "https://limina.example.com/auth/callback");
    assert_eq!(params["organization_id"], "org_01");
    assert!(start.set_cookies()[0].ends_with("; Secure"));
}

#[tokio::test]
async fn callback_creates_a_session_the_api_accepts() {
    let mut browser = Browser::start().await;
    let callback = browser.sign_in("/projects/demo").await;
    assert_eq!(callback.status, StatusCode::SEE_OTHER);
    assert_eq!(callback.location(), "/projects/demo");
    let exchange = &browser.idp.requests()[0];
    assert_eq!(exchange.path, "/user_management/authenticate");
    assert_eq!(exchange.body["grant_type"], "authorization_code");
    assert_eq!(exchange.body["client_id"], CLIENT_ID);
    assert_eq!(exchange.body["client_secret"], "sk_test");
    assert_eq!(exchange.body["code"], "code_01");
    assert!(!exchange.body["code_verifier"].as_str().unwrap().is_empty());
    let cookies = callback.set_cookies().join("\n");
    assert!(!cookies.contains("sk_test"));
    assert!(cookies.contains("limina_session="));
    assert!(cookies.contains(
        "limina_auth_flow=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; Path=/auth"
    ));
    assert!(!browser.client.has_cookie("limina_auth_flow"));
    let me = browser.me().await.json();
    assert_eq!(
        (&me["subject"], &me["display_name"], &me["email"], &me["auth_mode"]),
        (&json!("user_01"), &json!("Ada Lovelace"), &json!("ada@example.com"), &json!("workos"))
    );
}

#[tokio::test]
async fn unverifiable_tokens_end_sign_in_with_a_clear_reason() {
    let mut browser = Browser::start().await;
    browser.idp.with(|state| state.issuer = "https://unexpected.example.com".into());
    let response = browser.sign_in("/projects/demo").await;
    assert_eq!(response.location(), "/signin?error=token");
    assert!(!browser.client.has_cookie(SESSION_COOKIE));
}

#[tokio::test]
async fn callback_rejects_forged_state_cancelled_and_unauthorized_sign_ins() {
    let mut browser = Browser::start().await;
    browser.client.get("/auth/login").await;
    let forged = browser.client.get("/auth/callback?code=x&state=forged").await;
    assert_eq!(forged.location(), "/signin?error=state");
    let cancelled = browser.client.get("/auth/callback?error=access_denied").await;
    assert_eq!(cancelled.location(), "/signin?error=cancelled");
    let missing = browser.client.get("/auth/callback?code=x").await;
    assert_eq!(missing.location(), "/signin?error=failed");
    browser.idp.with(|state| state.permissions.clear());
    let denied = browser.sign_in("/projects/demo").await;
    assert_eq!(denied.location(), "/signin?error=permission");
    assert!(!browser.client.has_cookie(SESSION_COOKIE));
    assert_eq!(browser.me().await.status, StatusCode::UNAUTHORIZED);
    browser.idp.with(|state| state.fail = true);
    assert_eq!(browser.sign_in("/").await.location(), "/signin?error=rejected");
    // Only the WorkOS exchanges reached the provider; the forged state never did.
    assert!(browser.idp.requests().iter().all(|request| request.body["code"] == "code_01"));
}

#[tokio::test]
async fn session_writes_require_the_console_origin() {
    let mut browser = Browser::start().await;
    browser.sign_in("/projects/demo").await;
    let blocked = browser.client.send(Method::POST, "/v1/projects", &[]).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert_eq!(blocked.json()["error"]["details"]["reason"], "origin");
    let forged = browser
        .client
        .send(Method::POST, "/v1/projects", &[("origin", "https://evil.example")])
        .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    let created =
        browser.client.send(Method::POST, "/v1/projects", &[("origin", "http://testserver")]).await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.json()["owner"], "user_01");
}

#[tokio::test]
async fn expired_sessions_refresh_once_and_rotate_the_cookie() {
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
    let first = browser.me().await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(first.set_cookies().iter().any(|cookie| cookie.starts_with("limina_session=")));
    assert_eq!(first.json()["display_name"], "Ada Lovelace"); // the refreshed profile
    browser.client.set_cookie(SESSION_COOKIE, &stale);
    assert_eq!(browser.me().await.status, StatusCode::OK);
    let refreshes: Vec<Value> = browser
        .idp
        .requests()
        .into_iter()
        .filter(|request| request.body["grant_type"] == "refresh_token")
        .map(|request| request.body["refresh_token"].clone())
        .collect();
    assert_eq!(refreshes, [json!("refresh_old")]);

    browser.idp.with(|state| state.fail = true);
    let dead = browser
        .workos
        .sessions()
        .seal(&json!({
            "access_token": browser.idp.token().expires_in(-120).signed(),
            "refresh_token": "revoked",
            "user": {},
        }))
        .unwrap();
    browser.client.set_cookie(SESSION_COOKIE, &dead);
    let expired = browser.me().await;
    assert_eq!(expired.status, StatusCode::UNAUTHORIZED);
    assert_eq!(expired.json()["error"]["message"], "Your session expired. Please sign in again.");
}

#[tokio::test]
async fn parallel_requests_with_one_expired_session_share_one_refresh() {
    let browser = Browser::start().await;
    let stale = browser
        .workos
        .sessions()
        .seal(&json!({
            "access_token": browser.idp.token().permissions(&["limina:access"]).expires_in(-120).signed(),
            "refresh_token": "refresh_parallel",
            "user": {"id": "user_01"},
        }))
        .unwrap();
    let resolutions =
        futures::future::join_all((0..8).map(|_| browser.workos.resolve(&stale))).await;
    for resolved in resolutions {
        let resolved = resolved.unwrap();
        assert_eq!(resolved.session_id, "session_01");
        assert!(resolved.refreshed_cookie.is_some());
    }
    assert_eq!(browser.idp.requests().len(), 1);
    // A forged or foreign cookie is an invalid session, not a server error.
    let foreign = SessionSealer::new(&"q".repeat(40), "session").unwrap().seal(&json!({})).unwrap();
    assert_eq!(browser.workos.resolve(&foreign).await.unwrap_err().http_status(), 401);
}

#[tokio::test]
async fn logout_ends_the_workos_session_and_clears_the_cookie() {
    let mut browser = Browser::start().await;
    browser.sign_in("/projects/demo").await;
    let blocked = browser.client.send(Method::POST, "/auth/logout", &[]).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert!(browser.client.has_cookie(SESSION_COOKIE));
    let response =
        browser.client.send(Method::POST, "/auth/logout", &[("origin", "http://testserver")]).await;
    let redirect = response.json()["redirect"].as_str().unwrap().to_owned();
    assert!(
        redirect.starts_with(&format!("{}/user_management/sessions/logout?", browser.idp.base_url))
    );
    let params = query(&redirect);
    assert_eq!(params["session_id"], "session_01");
    assert_eq!(params["return_to"], "http://testserver/signin?signed_out=1");
    assert!(!browser.client.has_cookie(SESSION_COOKIE));
    assert_eq!(browser.me().await.status, StatusCode::UNAUTHORIZED);
    // Without a session there is nothing to end at WorkOS.
    let again =
        browser.client.send(Method::POST, "/auth/logout", &[("origin", "http://testserver")]).await;
    assert_eq!(again.json(), json!({"redirect": "/signin?signed_out=1"}));
}

#[tokio::test]
async fn workos_bearer_tokens_work_for_api_clients() {
    let mut browser = Browser::start().await;
    let token = browser.idp.token().permissions(&["limina:access"]).signed();
    let me = browser
        .client
        .send(Method::GET, "/v1/me", &[("authorization", &format!("Bearer {token}"))])
        .await;
    assert_eq!(me.json()["subject"], "user_01");
    let denied_token = browser.idp.token().permissions(&[]).signed();
    let denied = browser
        .client
        .send(Method::GET, "/v1/me", &[("authorization", &format!("Bearer {denied_token}"))])
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    // A bearer token is not ambient, so writes need no Origin.
    let created = browser
        .client
        .send(Method::POST, "/v1/projects", &[("authorization", &format!("Bearer {token}"))])
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
}

#[tokio::test]
async fn permission_denials_do_not_count_as_failed_sign_ins() {
    let browser = Browser::start().await;
    let auth = Auth::with_workos(
        browser.workos.clone(),
        Some("machine-token"),
        None,
        AuthOptions::default(),
    )
    .unwrap();
    let denied = browser.idp.token().permissions(&[]).signed();
    let request = |bearer: &str| {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", format!("Bearer {bearer}").parse().unwrap());
        limina_server::auth::RequestCredentials::from_headers(
            &Method::GET,
            &axum::http::Uri::from_static("/v1/me"),
            &headers,
            None,
            &limina_server::auth::TrustedProxies::default(),
        )
    };
    for _ in 0..12 {
        assert_eq!(auth.authenticate(&request(&denied)).await.unwrap_err().http_status(), 403);
    }
    assert_eq!(
        auth.authenticate(&request("machine-token")).await.unwrap().principal.auth_mode,
        "local"
    );
}
