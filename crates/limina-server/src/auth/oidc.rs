//! Signed JWT access tokens from any OpenID Connect provider, verified with discovery and JWKS.

use std::str::FromStr;

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, AlgorithmFamily};
use limina_core::{LiminaError, Principal, Result};
use serde_json::{Map, Value};

use super::authenticator::Authenticator;
use super::config::{AuthConfigError, Lookup};
use super::jwks::{Jwks, JwksPolicy, JwtRules, py_str, truthy, verify_jwt};

/// `LIMINA_OIDC_*` configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcSettings {
    /// `LIMINA_OIDC_ISSUER`: HTTPS, compared exactly with discovery and the `iss` claim.
    pub issuer: String,
    /// `LIMINA_OIDC_AUDIENCE`: required in every token's `aud`.
    pub audience: String,
    /// `LIMINA_OIDC_JWKS_URL`: skips discovery when set.
    pub jwks_url: Option<String>,
    /// `LIMINA_OIDC_ALGORITHMS` (comma-separated, default `RS256`).
    pub algorithms: Vec<String>,
    /// `LIMINA_OIDC_ADMIN_CLAIM`: the claim that marks instance administrators.
    pub admin_claim: Option<String>,
    /// `LIMINA_OIDC_ADMIN_VALUE` (default `admin`): the value, or list member, that does.
    pub admin_value: String,
    /// `LIMINA_OIDC_LEEWAY_SECONDS` (default 30, 0–300): clock-skew tolerance.
    pub leeway_seconds: i64,
    pub jwks_policy: JwksPolicy,
}

impl OidcSettings {
    pub fn new(issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            jwks_url: None,
            algorithms: vec!["RS256".to_owned()],
            admin_claim: None,
            admin_value: "admin".to_owned(),
            leeway_seconds: 30,
            jwks_policy: JwksPolicy::OIDC,
        }
    }

    /// `None` unless `LIMINA_OIDC_ISSUER` or `LIMINA_OIDC_AUDIENCE` is set; both are required.
    pub(crate) fn from_lookup(
        lookup: &Lookup<'_>,
    ) -> std::result::Result<Option<Self>, AuthConfigError> {
        let issuer = lookup.trimmed("LIMINA_OIDC_ISSUER");
        let audience = lookup.trimmed("LIMINA_OIDC_AUDIENCE");
        if issuer.is_empty() && audience.is_empty() {
            return Ok(None);
        }
        if issuer.is_empty() || audience.is_empty() {
            return Err(AuthConfigError::new(
                "LIMINA_OIDC_ISSUER and LIMINA_OIDC_AUDIENCE must be set together.",
            ));
        }
        let algorithms = lookup.get("LIMINA_OIDC_ALGORITHMS").unwrap_or_else(|| "RS256".to_owned());
        let mut settings = Self::new(issuer, audience);
        settings.algorithms = algorithms
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect();
        settings.jwks_url = lookup.non_empty("LIMINA_OIDC_JWKS_URL");
        settings.admin_claim = lookup.non_empty("LIMINA_OIDC_ADMIN_CLAIM");
        if let Some(value) = lookup.get("LIMINA_OIDC_ADMIN_VALUE") {
            settings.admin_value = value;
        }
        settings.leeway_seconds = lookup.integer("LIMINA_OIDC_LEEWAY_SECONDS", 30)?;
        Ok(Some(settings))
    }
}

/// Validates signed JWT access tokens with the provider's published keys.
///
/// The identity always comes from the signed claims: `sub` is the subject, the display name is
/// `name`, `preferred_username`, `email` or the subject, in that order, and the email counts as
/// verified only when `email_verified` is true. The actor hint is ignored.
pub struct OidcAuthenticator {
    issuer: Vec<String>,
    audience: String,
    algorithms: Vec<Algorithm>,
    admin_claim: Option<String>,
    admin_value: String,
    leeway_seconds: u64,
    jwks: Jwks,
}

impl OidcAuthenticator {
    /// Validates the settings and, unless `jwks_url` is set, discovers the key set from
    /// `{issuer}/.well-known/openid-configuration`. Both endpoints must use HTTPS.
    pub async fn connect(
        settings: OidcSettings,
        http: &reqwest::Client,
    ) -> std::result::Result<Self, AuthConfigError> {
        let issuer = validated_issuer(&settings)?;
        validated_rules(&settings)?;
        let jwks_url = match &settings.jwks_url {
            Some(url) => url.clone(),
            None => discover_jwks_url(&issuer, http).await?,
        };
        if !jwks_url.starts_with("https://") {
            return Err(AuthConfigError::new("The OIDC JWKS endpoint must use HTTPS."));
        }
        let jwks = Jwks::remote(jwks_url, http.clone(), settings.jwks_policy);
        Self::with_jwks(settings, jwks)
    }

    /// Validates the settings and verifies tokens with the given key source (no discovery, and no
    /// HTTPS check on the key source, which the caller chose).
    pub fn with_jwks(
        settings: OidcSettings,
        jwks: Jwks,
    ) -> std::result::Result<Self, AuthConfigError> {
        let issuer = validated_issuer(&settings)?;
        let (algorithms, leeway_seconds) = validated_rules(&settings)?;
        Ok(Self {
            issuer: vec![issuer],
            audience: settings.audience,
            algorithms,
            admin_claim: settings.admin_claim,
            admin_value: settings.admin_value,
            leeway_seconds,
            jwks,
        })
    }

    pub fn jwks(&self) -> &Jwks {
        &self.jwks
    }

    fn is_admin(&self, claims: &Map<String, Value>) -> bool {
        let Some(claim) = self.admin_claim.as_deref().filter(|claim| !claim.is_empty()) else {
            return false;
        };
        match claims.get(claim) {
            Some(Value::Array(items)) => items.iter().any(|item| py_str(item) == self.admin_value),
            value => py_str(value.unwrap_or(&Value::Null)) == self.admin_value,
        }
    }
}

#[async_trait]
impl Authenticator for OidcAuthenticator {
    fn mode(&self) -> &str {
        "oidc"
    }

    async fn authenticate(
        &self,
        bearer_token: Option<&str>,
        _actor_hint: Option<&str>,
    ) -> Result<Principal> {
        let Some(token) = bearer_token.filter(|token| !token.is_empty()) else {
            return Err(LiminaError::authentication());
        };
        let rules = JwtRules {
            algorithms: &self.algorithms,
            issuers: &self.issuer,
            audience: Some(&self.audience),
            leeway_seconds: self.leeway_seconds,
        };
        let claims = verify_jwt(&self.jwks, token, &rules).await.map_err(|error| {
            tracing::debug!(%error, "OIDC bearer token rejected");
            LiminaError::authentication_with("The bearer token is invalid or expired.")
        })?;
        let subject = claims.get("sub").map(py_str).unwrap_or_default().trim().to_owned();
        if subject.is_empty() {
            return Err(LiminaError::authentication_with("The bearer token has no subject."));
        }
        let display_name = ["name", "preferred_username", "email"]
            .into_iter()
            .map(|claim| claims.get(claim))
            .find(|value| truthy(*value))
            .flatten()
            .map_or_else(|| subject.clone(), py_str)
            .trim()
            .to_owned();
        let email = claims
            .get("email")
            .filter(|value| truthy(Some(value)))
            .map(|value| py_str(value).trim().to_owned());
        Ok(Principal::new(subject, display_name)
            .with_email(email)
            .email_verified(email_verified(&claims))
            .admin(self.is_admin(&claims))
            .mode("oidc"))
    }
}

/// The standard `email_verified` claim: `true` (or the string `"true"` some providers send).
/// Anything else, including a missing claim, leaves the email unverified.
fn email_verified(claims: &Map<String, Value>) -> bool {
    match claims.get("email_verified") {
        Some(Value::Bool(verified)) => *verified,
        Some(Value::String(text)) => text.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn validated_issuer(settings: &OidcSettings) -> std::result::Result<String, AuthConfigError> {
    let issuer = settings.issuer.trim_end_matches('/').to_owned();
    if !issuer.starts_with("https://") {
        return Err(AuthConfigError::new("LIMINA_OIDC_ISSUER must use HTTPS."));
    }
    Ok(issuer)
}

/// Signature algorithms (asymmetric only: the keys come from a public key set) and the leeway.
fn validated_rules(
    settings: &OidcSettings,
) -> std::result::Result<(Vec<Algorithm>, u64), AuthConfigError> {
    let unsigned =
        || AuthConfigError::new("LIMINA_OIDC_ALGORITHMS must contain signed JWT algorithms.");
    if settings.algorithms.is_empty()
        || settings.algorithms.iter().any(|item| item.eq_ignore_ascii_case("none"))
    {
        return Err(unsigned());
    }
    let mut algorithms = Vec::new();
    for name in &settings.algorithms {
        let algorithm = Algorithm::from_str(name).map_err(|_| {
            AuthConfigError::new(format!(
                "LIMINA_OIDC_ALGORITHMS contains an unsupported algorithm: {name}."
            ))
        })?;
        if algorithm.family() == AlgorithmFamily::Hmac {
            return Err(AuthConfigError::new(
                "LIMINA_OIDC_ALGORITHMS must use public-key algorithms (RS*, PS*, ES*, EdDSA).",
            ));
        }
        algorithms.push(algorithm);
    }
    let leeway =
        u64::try_from(settings.leeway_seconds).ok().filter(|leeway| *leeway <= 300).ok_or_else(
            || AuthConfigError::new("OIDC clock-skew leeway must be between 0 and 300 seconds."),
        )?;
    Ok((algorithms, leeway))
}

async fn discover_jwks_url(
    issuer: &str,
    http: &reqwest::Client,
) -> std::result::Result<String, AuthConfigError> {
    let discovery_url = format!("{issuer}/.well-known/openid-configuration");
    let unavailable = || {
        AuthConfigError::new(format!("Cannot load OIDC discovery metadata from {discovery_url}."))
    };
    let response = http.get(&discovery_url).send().await.map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let metadata: Value = response.json().await.map_err(|_| unavailable())?;
    jwks_url_from_discovery(issuer, &metadata)
}

/// The `jwks_uri` of discovery metadata whose `issuer` matches exactly.
pub(crate) fn jwks_url_from_discovery(
    issuer: &str,
    metadata: &Value,
) -> std::result::Result<String, AuthConfigError> {
    if metadata.get("issuer").and_then(Value::as_str) != Some(issuer) {
        return Err(AuthConfigError::new(
            "OIDC discovery issuer does not exactly match LIMINA_OIDC_ISSUER.",
        ));
    }
    match metadata.get("jwks_uri").and_then(Value::as_str) {
        Some(url) if url.starts_with("https://") => Ok(url.to_owned()),
        _ => Err(AuthConfigError::new("OIDC discovery metadata must provide an HTTPS jwks_uri.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ISSUER: &str = "https://identity.example.test";

    #[test]
    fn discovery_must_match_the_issuer_and_publish_https_keys() {
        let good = json!({"issuer": ISSUER, "jwks_uri": "https://identity.example.test/jwks"});
        assert_eq!(
            jwks_url_from_discovery(ISSUER, &good).unwrap(),
            "https://identity.example.test/jwks"
        );
        let other_issuer = json!({"issuer": "https://other.example.test", "jwks_uri": "https://x"});
        assert!(jwks_url_from_discovery(ISSUER, &other_issuer).is_err());
        let insecure = json!({"issuer": ISSUER, "jwks_uri": "http://identity.example.test/jwks"});
        assert!(jwks_url_from_discovery(ISSUER, &insecure).is_err());
        assert!(jwks_url_from_discovery(ISSUER, &json!([])).is_err());
    }

    #[test]
    fn settings_reject_unsafe_configuration() {
        let jwks = || Jwks::remote("https://x/jwks", reqwest::Client::new(), JwksPolicy::OIDC);
        let check = |change: fn(&mut OidcSettings)| {
            let mut settings = OidcSettings::new(ISSUER, "limina-api");
            change(&mut settings);
            OidcAuthenticator::with_jwks(settings, jwks()).err().map(|error| error.to_string())
        };
        assert_eq!(check(|_| {}), None);
        assert_eq!(check(|s| s.issuer = "https://identity.example.test/".into()), None);
        assert!(check(|s| s.issuer = "http://identity.example.test".into()).is_some());
        assert!(check(|s| s.algorithms = vec![]).is_some());
        assert!(check(|s| s.algorithms = vec!["RS256".into(), "None".into()]).is_some());
        assert!(check(|s| s.algorithms = vec!["HS256".into()]).is_some());
        assert!(check(|s| s.algorithms = vec!["XX999".into()]).is_some());
        assert!(check(|s| s.leeway_seconds = 301).is_some());
        assert!(check(|s| s.leeway_seconds = -1).is_some());
        assert_eq!(check(|s| s.algorithms = vec!["RS256".into(), "ES256".into()]), None);
    }
}
