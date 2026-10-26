//! Signing keys published as a JSON Web Key Set, and JWT verification against them.
//!
//! The cache keeps the last successful key set for `max_age`. A token whose `kid` is unknown
//! triggers one refresh (key rotation), but never more than once per `refresh_cooldown`, so a
//! stream of forged `kid`s cannot turn the runtime into a request amplifier against the identity
//! provider. When a refresh fails, keys that were already known keep working until the next
//! successful fetch.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use jsonwebtoken::jwk::{Jwk, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::{Map, Value};

/// How long a key set stays fresh, and how often an unknown `kid` may force a refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JwksPolicy {
    pub max_age: Duration,
    pub refresh_cooldown: Duration,
}

impl JwksPolicy {
    /// Generic OIDC providers: five minutes (the Python runtime's PyJWKClient default).
    pub const OIDC: Self =
        Self { max_age: Duration::from_secs(300), refresh_cooldown: Duration::from_secs(10) };
    /// WorkOS: one hour, as the Python runtime configured it.
    pub const WORKOS: Self =
        Self { max_age: Duration::from_secs(3600), refresh_cooldown: Duration::from_secs(10) };
}

/// A remote JSON Web Key Set with caching. Cheap to clone.
#[derive(Clone)]
pub struct Jwks(Arc<JwksInner>);

struct JwksInner {
    url: String,
    http: reqwest::Client,
    policy: JwksPolicy,
    keys: RwLock<KeySet>,
    /// Serializes fetches and remembers when the last one started.
    fetch: tokio::sync::Mutex<Option<Instant>>,
}

#[derive(Default)]
struct KeySet {
    keys: HashMap<String, Arc<SigningKey>>,
    fetched_at: Option<Instant>,
}

pub(crate) struct SigningKey {
    key: DecodingKey,
    /// The JWK's own `alg`, when it declares one.
    algorithm: Option<Algorithm>,
}

impl std::fmt::Debug for Jwks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jwks").field("url", &self.0.url).finish_non_exhaustive()
    }
}

impl Jwks {
    /// Keys fetched from `url` with `http`. The caller validates the URL (HTTPS in production).
    pub fn remote(url: impl Into<String>, http: reqwest::Client, policy: JwksPolicy) -> Self {
        Self(Arc::new(JwksInner {
            url: url.into(),
            http,
            policy,
            keys: RwLock::new(KeySet::default()),
            fetch: tokio::sync::Mutex::new(None),
        }))
    }

    pub fn url(&self) -> &str {
        &self.0.url
    }

    pub(crate) async fn signing_key(&self, kid: &str) -> Result<Arc<SigningKey>, String> {
        let inner = &self.0;
        if let Some(key) = self.fresh_key(kid) {
            return Ok(key);
        }
        let mut last_attempt = inner.fetch.lock().await;
        if let Some(key) = self.fresh_key(kid) {
            return Ok(key); // another request refreshed the set while this one waited
        }
        let cooled_down =
            last_attempt.is_none_or(|at| at.elapsed() >= inner.policy.refresh_cooldown);
        if cooled_down {
            *last_attempt = Some(Instant::now());
            match self.fetch().await {
                Ok(keys) => {
                    let mut set = inner.keys.write().unwrap_or_else(PoisonError::into_inner);
                    *set = KeySet { keys, fetched_at: Some(Instant::now()) };
                }
                Err(error) => tracing::warn!(url = %inner.url, %error, "JWKS refresh failed"),
            }
        }
        let set = inner.keys.read().unwrap_or_else(PoisonError::into_inner);
        set.keys.get(kid).cloned().ok_or_else(|| {
            if set.fetched_at.is_none() {
                "the signing keys are unavailable".to_owned()
            } else {
                format!("no signing key matches kid {kid:?}")
            }
        })
    }

    fn fresh_key(&self, kid: &str) -> Option<Arc<SigningKey>> {
        let set = self.0.keys.read().unwrap_or_else(PoisonError::into_inner);
        let fresh = set.fetched_at.is_some_and(|at| at.elapsed() < self.0.policy.max_age);
        if fresh { set.keys.get(kid).cloned() } else { None }
    }

    async fn fetch(&self) -> Result<HashMap<String, Arc<SigningKey>>, String> {
        let response =
            self.0.http.get(&self.0.url).send().await.map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let document: Value = response.json().await.map_err(|error| error.to_string())?;
        let keys = parse_key_set(&document);
        if keys.is_empty() {
            tracing::warn!(url = %self.0.url, "the JWKS endpoint did not contain any signing keys");
        }
        Ok(keys)
    }
}

/// Usable signing keys by `kid`: keys without a `kid`, encryption keys and keys this library cannot
/// load are skipped (as PyJWT's `PyJWKClient` does); the first key wins for a repeated `kid`.
fn parse_key_set(document: &Value) -> HashMap<String, Arc<SigningKey>> {
    let mut keys = HashMap::new();
    let entries = document.get("keys").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    for entry in entries {
        let Ok(jwk) = serde_json::from_value::<Jwk>(entry.clone()) else { continue };
        let Some(kid) = jwk.common.key_id.clone().filter(|kid| !kid.is_empty()) else { continue };
        if !matches!(jwk.common.public_key_use, None | Some(PublicKeyUse::Signature)) {
            continue;
        }
        let Ok(key) = DecodingKey::from_jwk(&jwk) else { continue };
        let algorithm = entry.get("alg").and_then(Value::as_str);
        let algorithm = match algorithm.map(Algorithm::from_str) {
            None => None,
            Some(Ok(algorithm)) => Some(algorithm),
            Some(Err(_)) => continue, // e.g. an encryption algorithm
        };
        keys.entry(kid).or_insert_with(|| Arc::new(SigningKey { key, algorithm }));
    }
    keys
}

/// Why a token was rejected. Only `Expired` tells a session it may refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// A correctly signed token past its `exp` (beyond the leeway).
    Expired,
    Invalid(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => f.write_str("the token has expired"),
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

/// The checks a token must pass besides its signature.
pub(crate) struct JwtRules<'a> {
    pub algorithms: &'a [Algorithm],
    pub issuers: &'a [String],
    /// Required and checked when set; ignored otherwise (WorkOS access tokens carry no audience).
    pub audience: Option<&'a str>,
    pub leeway_seconds: u64,
}

/// Verifies the signature (key chosen by `kid`), algorithm, issuer, audience, `exp`, `nbf` and
/// `iat`, and requires `exp`, `iat`, `iss` and `sub`. Returns the claims.
pub(crate) async fn verify_jwt(
    jwks: &Jwks,
    token: &str,
    rules: &JwtRules<'_>,
) -> Result<Map<String, Value>, TokenError> {
    let invalid = |reason: &str| TokenError::Invalid(reason.to_owned());
    let header = jsonwebtoken::decode_header(token).map_err(|_| invalid("malformed token"))?;
    if !rules.algorithms.contains(&header.alg) {
        return Err(invalid("the token algorithm is not allowed"));
    }
    let kid = header.kid.as_deref().ok_or_else(|| invalid("the token has no key id"))?;
    let key = jwks.signing_key(kid).await.map_err(TokenError::Invalid)?;
    if key.algorithm.is_some_and(|algorithm| algorithm != header.alg) {
        return Err(invalid("the token algorithm does not match its key"));
    }

    let mut validation = Validation::new(header.alg);
    validation.leeway = rules.leeway_seconds;
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.set_issuer(rules.issuers);
    let mut required = vec!["exp", "iss", "sub"];
    match rules.audience {
        Some(audience) => {
            validation.set_audience(&[audience]);
            required.push("aud");
        }
        None => validation.validate_aud = false,
    }
    validation.set_required_spec_claims(&required);

    let claims = jsonwebtoken::decode::<Map<String, Value>>(token, &key.key, &validation)
        .map_err(|error| match error.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => TokenError::Expired,
            _ => TokenError::Invalid(error.to_string()),
        })?
        .claims;

    // jsonwebtoken does not look at `iat`; PyJWT required it and rejected one from the future.
    let issued_at = claims.get("iat").and_then(Value::as_f64).ok_or_else(|| invalid("no iat"))?;
    #[allow(clippy::cast_precision_loss)]
    if issued_at > (unix_now() + rules.leeway_seconds) as f64 {
        return Err(invalid("the token is not yet valid (iat)"));
    }
    Ok(claims)
}

/// Claims decoded without verification. Only for logging and for reading a session id that is
/// sent back to its own issuer.
pub(crate) fn unverified_claims(token: &str) -> Option<Map<String, Value>> {
    jsonwebtoken::dangerous::insecure_decode::<Map<String, Value>>(token)
        .ok()
        .map(|data| data.claims)
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

/// Python truthiness of a JSON value: `null`, `false`, `0`, `""`, `[]` and `{}` are false.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(fields)) => !fields.is_empty(),
    }
}

/// Python `str()` of a JSON value, for claims compared or displayed as text.
pub(crate) fn py_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_sets_keep_only_identified_signing_keys() {
        let rsa = |kid: Option<&str>, extra: Value| {
            let mut key = json!({"kty": "RSA", "n": "sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1WlUzewbgBHod5pcM9H95GQRV3JDXboIRROSBigeC5yjU1hGzHHyXss8UDprecbAYxknTcQkhslANGRUZmdTOQ5qTRsLAt6BTYuyvVRdhS8exSZEy_c4gs_7svlJJQ4H9_NxsiIoLwAEk7-Q3UXERGYw_75IDrGA84-lA_-Ct4eTlXHBIY2EaV7t7LjJaynVJCpkv4LKjTTAumiGUIuQhrNhZLuF_RJLqHpM2kgWFLU7-VTdL1VbC2tejvcI2BlMkEpk1BzBZI0KQB0GaDWFLN-aEAw3vRw", "e": "AQAB"});
            if let Some(kid) = kid {
                key["kid"] = json!(kid);
            }
            for (name, value) in extra.as_object().unwrap() {
                key[name] = value.clone();
            }
            key
        };
        let document = json!({"keys": [
            rsa(Some("a"), json!({"use": "sig", "alg": "RS256"})),
            rsa(None, json!({})),
            rsa(Some("enc"), json!({"use": "enc"})),
            rsa(Some("oaep"), json!({"alg": "RSA-OAEP"})),
            {"kty": "unknown", "kid": "odd"},
            rsa(Some("b"), json!({})),
        ]});
        let keys = parse_key_set(&document);
        let mut kids: Vec<_> = keys.keys().cloned().collect();
        kids.sort();
        assert_eq!(kids, ["a", "b"]);
        assert_eq!(keys["a"].algorithm, Some(Algorithm::RS256));
        assert_eq!(keys["b"].algorithm, None);
    }

    #[test]
    fn python_truthiness_and_str() {
        assert!(!truthy(Some(&json!(""))));
        assert!(!truthy(Some(&json!(0))));
        assert!(!truthy(Some(&json!([]))));
        assert!(truthy(Some(&json!("x"))));
        assert_eq!(py_str(&json!(null)), "None");
        assert_eq!(py_str(&json!(true)), "True");
        assert_eq!(py_str(&json!(7)), "7");
    }
}
