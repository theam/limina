//! OIDC bearer tokens verified against a JWKS endpoint. Ports `tests/test_cloud_auth.py`
//! (`OidcAuthenticationTests`) and covers key rotation and the refresh cooldown.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod auth_support;

use std::time::Duration;

use auth_support::{FakeIdp, KEYS, now};
use limina_server::auth::{Authenticator, Jwks, JwksPolicy, OidcAuthenticator, OidcSettings};
use serde_json::{Value, json};

const ISSUER: &str = "https://identity.example.test";

fn settings() -> OidcSettings {
    let mut settings = OidcSettings::new(ISSUER, "limina-api");
    settings.admin_claim = Some("roles".into());
    settings.admin_value = "limina-admin".into();
    settings
}

/// The Python test's authenticator, with keys served by the fake provider.
fn authenticator(idp: &FakeIdp, policy: JwksPolicy) -> OidcAuthenticator {
    let jwks = Jwks::remote(format!("{}/jwks", idp.base_url), reqwest::Client::new(), policy);
    OidcAuthenticator::with_jwks(settings(), jwks).unwrap()
}

fn claims(overrides: Value) -> Value {
    let now = now();
    let mut claims = json!({
        "iss": ISSUER,
        "aud": "limina-api",
        "sub": "user-123",
        "name": "Ada Investigator",
        "email": "ada@example.test",
        "roles": ["limina-admin"],
        "iat": now,
        "exp": now + 300,
    });
    for (name, value) in overrides.as_object().unwrap() {
        if value.is_null() {
            claims.as_object_mut().unwrap().remove(name);
        } else {
            claims[name] = value.clone();
        }
    }
    claims
}

fn token(overrides: Value) -> String {
    KEYS[0].sign(&claims(overrides))
}

// ─── test_cloud_auth.py: OidcAuthenticationTests ──────────────────────────────

#[tokio::test]
async fn signed_oidc_claims_define_the_principal() {
    let idp = FakeIdp::start().await;
    let oidc = authenticator(&idp, JwksPolicy::OIDC);
    let principal =
        oidc.authenticate(Some(&token(json!({}))), Some("untrusted-header-value")).await.unwrap();
    assert_eq!(principal.subject, "user-123");
    assert_eq!(principal.actor(), "Ada Investigator");
    assert_eq!(principal.email.as_deref(), Some("ada@example.test"));
    assert!(principal.instance_admin);
    assert!(!principal.project_admin);
    assert_eq!(principal.auth_mode, "oidc");
}

#[tokio::test]
async fn wrong_audience_and_expired_tokens_are_rejected() {
    let idp = FakeIdp::start().await;
    let oidc = authenticator(&idp, JwksPolicy::OIDC);
    for overrides in [json!({"aud": "another-api"}), json!({"exp": now() - 60})] {
        let error = oidc.authenticate(Some(&token(overrides)), None).await.unwrap_err();
        assert_eq!(error.code(), "authentication_required");
        assert_eq!(error.message, "The bearer token is invalid or expired.");
    }
}

// ─── Claims and token shape ───────────────────────────────────────────────────

#[tokio::test]
async fn every_registered_claim_is_checked() {
    let idp = FakeIdp::start().await;
    let oidc = authenticator(&idp, JwksPolicy::OIDC);
    assert!(
        oidc.authenticate(Some(&token(json!({"aud": ["other", "limina-api"]}))), None)
            .await
            .is_ok()
    );
    for overrides in [
        json!({"iss": "https://other.example.test"}),
        json!({"iat": null}),
        json!({"iat": now() + 600}),
        json!({"nbf": now() + 600}),
        json!({"aud": null}),
        json!({"sub": null}),
        json!({"exp": null}),
    ] {
        assert!(
            oidc.authenticate(Some(&token(overrides.clone())), None).await.is_err(),
            "{overrides}"
        );
    }
    // Within the 30-second leeway.
    assert!(oidc.authenticate(Some(&token(json!({"exp": now() - 10}))), None).await.is_ok());
    assert!(oidc.authenticate(None, None).await.is_err());
    assert!(oidc.authenticate(Some("not-a-jwt"), None).await.is_err());
}

#[tokio::test]
async fn tokens_need_a_published_key_id_and_an_allowed_algorithm() {
    let idp = FakeIdp::start().await;
    let oidc = authenticator(&idp, JwksPolicy::OIDC);
    let without_kid = KEYS[0]
        .sign_with(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256), &claims(json!({})));
    assert!(oidc.authenticate(Some(&without_kid), None).await.is_err());
    assert!(oidc.authenticate(Some(&token(json!({}))), None).await.is_ok());
    // A key the provider does not publish.
    assert!(oidc.authenticate(Some(&KEYS[1].sign(&claims(json!({})))), None).await.is_err());
    // An algorithm outside LIMINA_OIDC_ALGORITHMS (HS256 with the public modulus as secret).
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    header.kid = Some(KEYS[0].kid.clone());
    let secret =
        jsonwebtoken::EncodingKey::from_secret(KEYS[0].jwk()["n"].as_str().unwrap().as_bytes());
    let confused = jsonwebtoken::encode(&header, &claims(json!({})), &secret).unwrap();
    assert!(oidc.authenticate(Some(&confused), None).await.is_err());
}

#[tokio::test]
async fn names_fall_back_and_admin_claims_match_strings_or_lists() {
    let idp = FakeIdp::start().await;
    let oidc = authenticator(&idp, JwksPolicy::OIDC);
    let principal = |overrides: Value| {
        let token = token(overrides);
        let oidc = &oidc;
        async move { oidc.authenticate(Some(&token), None).await.unwrap() }
    };
    let preferred = principal(json!({"name": null, "preferred_username": "ada"})).await;
    assert_eq!(preferred.display_name, "ada");
    let by_email = principal(json!({"name": "", "roles": "limina-admin"})).await;
    assert_eq!(by_email.display_name, "ada@example.test");
    assert!(by_email.instance_admin);
    let bare = principal(json!({"name": null, "email": null, "roles": ["reader"]})).await;
    assert_eq!((bare.display_name.as_str(), bare.email.as_deref()), ("user-123", None));
    assert!(!bare.instance_admin);
    assert!(!principal(json!({"roles": null})).await.instance_admin);
}

// ─── Key rotation and caching ─────────────────────────────────────────────────

#[tokio::test]
async fn unknown_key_ids_refresh_the_key_set() {
    let idp = FakeIdp::start().await;
    let policy = JwksPolicy { refresh_cooldown: Duration::ZERO, ..JwksPolicy::OIDC };
    let oidc = authenticator(&idp, policy);
    assert!(oidc.authenticate(Some(&token(json!({}))), None).await.is_ok());
    assert!(oidc.authenticate(Some(&token(json!({}))), None).await.is_ok());
    assert_eq!(idp.gets(), ["/jwks"]); // cached

    idp.with(|state| state.published = vec![1]); // the provider rotates its key
    let rotated = KEYS[1].sign(&claims(json!({})));
    assert!(oidc.authenticate(Some(&rotated), None).await.is_ok());
    assert_eq!(idp.gets().len(), 2);
    // The retired key is gone from the refreshed set.
    assert!(oidc.authenticate(Some(&token(json!({}))), None).await.is_err());
}

#[tokio::test]
async fn forged_key_ids_cannot_hammer_the_provider() {
    let idp = FakeIdp::start().await;
    let policy = JwksPolicy { refresh_cooldown: Duration::from_secs(3600), ..JwksPolicy::OIDC };
    let oidc = authenticator(&idp, policy);
    assert!(oidc.authenticate(Some(&token(json!({}))), None).await.is_ok());
    idp.with(|state| state.published = vec![0, 1]);
    for _ in 0..5 {
        let unknown = KEYS[1].sign(&claims(json!({})));
        assert!(oidc.authenticate(Some(&unknown), None).await.is_err());
    }
    assert_eq!(idp.gets().len(), 1, "an unknown kid refreshes at most once per cooldown");
}

#[tokio::test]
async fn known_keys_survive_an_unreachable_provider() {
    let idp = FakeIdp::start().await;
    let policy = JwksPolicy { max_age: Duration::ZERO, refresh_cooldown: Duration::ZERO };
    let oidc = authenticator(&idp, policy);
    assert!(oidc.authenticate(Some(&token(json!({}))), None).await.is_ok());
    let jwks_url = format!("{}/jwks", idp.base_url);
    drop(idp); // the key set is now stale and cannot be refreshed
    assert!(oidc.authenticate(Some(&token(json!({}))), None).await.is_ok());

    let unreachable = Jwks::remote(jwks_url, reqwest::Client::new(), JwksPolicy::OIDC);
    let cold = OidcAuthenticator::with_jwks(settings(), unreachable).unwrap();
    let error = cold.authenticate(Some(&token(json!({}))), None).await.unwrap_err();
    assert_eq!(error.http_status(), 401);
}
