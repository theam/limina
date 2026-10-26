//! Failed-authentication rate limits. Ports the limiter test of
//! `tests/test_cloud_enterprise_hardening.py` and, at this layer, the API's rate-limit test from
//! `tests/test_cloud_api.py`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod auth_support;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use auth_support::TestClient;
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::routing::get;
use axum::{Json, Router};
use limina_server::auth::rate_limit::Clock;
use limina_server::auth::{
    Auth, AuthLimits, AuthOptions, Authenticator, Caller, FailureRateLimiter, LocalAuthenticator,
    RateLimitSettings, RequestCredentials, TrustedProxies,
};
use serde_json::{Value, json};

fn manual_clock() -> (Arc<AtomicU64>, Clock) {
    let now = Arc::new(AtomicU64::new(0));
    let reader = now.clone();
    (now, Arc::new(move || reader.load(Ordering::SeqCst) as f64 / 1000.0))
}

fn token_auth(limits: AuthLimits) -> Auth {
    let local = LocalAuthenticator::new(Some("project-token".into()), None).unwrap();
    Auth::new(Arc::new(local), None, AuthOptions { limits, ..AuthOptions::default() }).unwrap()
}

fn from(client: [u8; 4], bearer: &str) -> RequestCredentials {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap());
    RequestCredentials::from_headers(
        &Method::GET,
        &Uri::from_static("/healthz"),
        &headers,
        Some(IpAddr::from(client)),
        &TrustedProxies::default(),
    )
}

// ─── test_cloud_enterprise_hardening.py ───────────────────────────────────────

#[test]
fn failure_limiter_is_bounded_and_recovers_after_the_window() {
    let (now, clock) = manual_clock();
    let settings = RateLimitSettings { limit: 2, window_seconds: 10, max_keys: 2 };
    let limiter = FailureRateLimiter::with_clock(settings, clock).unwrap();
    limiter.failure("client");
    limiter.failure("client");
    let error = limiter.check("client").unwrap_err();
    assert_eq!((error.http_status(), error.code()), (429, "rate_limited"));
    now.store(11_000, Ordering::SeqCst);
    limiter.check("client").unwrap();
}

#[test]
fn retry_after_rounds_half_to_even_and_is_at_least_one_second() {
    let (now, clock) = manual_clock();
    let limiter = FailureRateLimiter::with_clock(RateLimitSettings::new(1, 10), clock).unwrap();
    limiter.failure("client");
    let retry = |at_millis: u64| {
        now.store(at_millis, Ordering::SeqCst);
        limiter.check("client").unwrap_err().details["retry_after_seconds"].as_u64().unwrap()
    };
    assert_eq!(retry(0), 10);
    assert_eq!(retry(7_500), 2); // 2.5 → 2, like Python's round()
    assert_eq!(retry(6_500), 4); // 3.5 → 4
    assert_eq!(retry(9_900), 1); // 0.1 → at least 1
}

// ─── test_cloud_api.py: test_failed_authentication_is_rate_limited ────────────

#[tokio::test]
async fn failed_authentication_is_rate_limited() {
    async fn health(Caller(_principal): Caller) -> Json<Value> {
        Json(json!({"ok": true}))
    }
    let auth = token_auth(AuthLimits::default());
    let app = Router::new().route("/healthz", get(health)).layer(auth.layer());
    let mut client = TestClient::new(app);
    let invalid = [("authorization", "Bearer wrong")];
    for _ in 0..10 {
        let response = client.send(Method::GET, "/healthz", &invalid).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
    let limited = client.send(Method::GET, "/healthz", &invalid).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    let retry_after = limited.json()["error"]["details"]["retry_after_seconds"].as_u64().unwrap();
    assert!(retry_after >= 1);
    assert_eq!(limited.headers[RETRY_AFTER], retry_after.to_string().as_str());
    // The client is refused before its credentials are even looked at.
    let valid = [("authorization", "Bearer project-token")];
    let still_limited = client.send(Method::GET, "/healthz", &valid).await;
    assert_eq!(still_limited.status, StatusCode::TOO_MANY_REQUESTS);
}

// ─── Keys, success and the global ceiling ─────────────────────────────────────

#[tokio::test]
async fn each_client_address_has_its_own_budget_and_success_clears_it() {
    let limits = AuthLimits { client: RateLimitSettings::new(2, 60), ..AuthLimits::default() };
    let auth = token_auth(limits);
    let first = [192, 0, 2, 1];
    for _ in 0..2 {
        assert_eq!(auth.authenticate(&from(first, "wrong")).await.unwrap_err().http_status(), 401);
    }
    assert_eq!(auth.authenticate(&from(first, "wrong")).await.unwrap_err().http_status(), 429);
    assert!(auth.authenticate(&from([192, 0, 2, 2], "project-token")).await.is_ok());

    let third = [192, 0, 2, 3];
    assert!(auth.authenticate(&from(third, "wrong")).await.is_err());
    assert!(auth.authenticate(&from(third, "project-token")).await.is_ok());
    assert!(auth.authenticate(&from(third, "wrong")).await.is_err());
    assert_eq!(auth.authenticate(&from(third, "wrong")).await.unwrap_err().http_status(), 401);
}

#[tokio::test]
async fn the_global_ceiling_spans_clients_and_transports() {
    let limits = AuthLimits { global: RateLimitSettings::new(3, 60), ..AuthLimits::default() };
    let auth = token_auth(limits);
    for client in 1..=3 {
        assert!(auth.authenticate(&from([198, 51, 100, client], "wrong")).await.is_err());
    }
    let limited = auth.authenticate(&from([198, 51, 100, 4], "project-token")).await.unwrap_err();
    assert_eq!(limited.http_status(), 429);
    // WebSocket and MCP share the same ceiling through the transport authenticator.
    let transport = auth.transport().authenticate(Some("project-token"), None).await.unwrap_err();
    assert_eq!(transport.code(), "rate_limited");
}

#[tokio::test]
async fn trusted_proxies_key_failures_by_the_forwarded_client() {
    let limits = AuthLimits { client: RateLimitSettings::new(1, 60), ..AuthLimits::default() };
    let auth = token_auth(limits);
    let via_proxy = |client: &'static str| {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer wrong"));
        headers.insert("x-forwarded-for", HeaderValue::from_static(client));
        RequestCredentials::from_headers(
            &Method::GET,
            &Uri::from_static("/healthz"),
            &headers,
            Some(IpAddr::from([127, 0, 0, 1])),
            &TrustedProxies::default(),
        )
    };
    assert_eq!(auth.authenticate(&via_proxy("203.0.113.1")).await.unwrap_err().http_status(), 401);
    assert_eq!(auth.authenticate(&via_proxy("203.0.113.1")).await.unwrap_err().http_status(), 429);
    assert_eq!(auth.authenticate(&via_proxy("203.0.113.2")).await.unwrap_err().http_status(), 401);
}
