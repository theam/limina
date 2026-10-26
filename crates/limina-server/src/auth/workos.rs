//! WorkOS AuthKit sign-in for the web console, hosted by the runtime itself.
//!
//! This is the server-side AuthKit pattern:
//!
//! 1. `/auth/login` redirects the browser to AuthKit's hosted sign-in with `state` and PKCE.
//! 2. `/auth/callback` exchanges the code with the WorkOS API (client secret + verifier), checks
//!    the access token, and stores the session in a sealed, HttpOnly, SameSite=Lax cookie.
//! 3. Every API request with that cookie verifies the WorkOS access token locally against the
//!    client's JWKS with the User Management issuer pinned. Expired tokens are refreshed
//!    server-side with the sealed refresh token and the cookie is rotated. The browser never sees
//!    a token.
//!
//! Authorization comes from the access token: an optional organization pin, an optional required
//! permission, and a permission that grants instance administration (the runtime, not other
//! people's projects). Project roles stay in Limina's own membership table, keyed by the WorkOS
//! user id; owners share projects by email through the User Management directory
//! ([`super::directory`]).
//!
//! The same verifier accepts WorkOS access tokens as bearer credentials, so API and MCP clients
//! that hold one can call the runtime directly.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use jsonwebtoken::Algorithm;
use limina_core::{LiminaError, Principal, Result};
use serde_json::{Map, Value, json};
use subtle::ConstantTimeEq;
use url::form_urlencoded;

use super::authenticator::Authenticator;
use super::config::{AuthConfigError, Lookup};
use super::jwks::{
    Jwks, JwksPolicy, JwtRules, TokenError, py_str, truthy, unix_now, unverified_claims, verify_jwt,
};
use super::seal::{SessionSealer, pkce_pair, sha256_hex, token_urlsafe};

pub use super::cookies::{FLOW_COOKIE, FLOW_TTL_SECONDS, SESSION_COOKIE, SESSION_TTL_SECONDS};

pub const DEFAULT_ADMIN_PERMISSION: &str = "limina:admin";
/// Parallel requests carrying the same expired session share one refresh for this long.
const REFRESH_REUSE: Duration = Duration::from_secs(60);
/// After a failed discovery, the API-host issuer is used and discovery is retried this much later.
const DISCOVERY_RETRY: Duration = Duration::from_secs(60);

/// `WORKOS_*` and `LIMINA_WORKOS_*` configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkOsSettings {
    /// `WORKOS_CLIENT_ID` (`client_…`). Setting it enables WorkOS sign-in.
    pub client_id: String,
    /// `WORKOS_API_KEY`: the secret key, used only for the code exchange and refreshes.
    pub api_key: String,
    /// `WORKOS_COOKIE_PASSWORD`: at least 32 characters; seals both cookies.
    pub cookie_password: String,
    /// From `WORKOS_API_HOSTNAME` (default `api.workos.com`), see [`api_base_url`].
    pub api_base_url: String,
    /// `WORKOS_REDIRECT_URI`; defaults to `<public URL>/auth/callback`.
    pub redirect_uri: Option<String>,
    /// `LIMINA_PUBLIC_URL`: the URL people open. Sets the redirect URI, the origin allowed to make
    /// cookie-authenticated writes, and `Secure` cookies when it is HTTPS.
    pub public_url: Option<String>,
    /// `LIMINA_WORKOS_ISSUER` (comma-separated): pinned issuers; skips discovery.
    pub issuers: Vec<String>,
    /// `LIMINA_WORKOS_ORGANIZATION_ID`: only members of this organization may sign in.
    pub organization_id: Option<String>,
    /// `LIMINA_WORKOS_REQUIRED_PERMISSION`: a permission every user needs.
    pub required_permission: Option<String>,
    /// `LIMINA_WORKOS_ADMIN_PERMISSION` (default `limina:admin`; empty disables it).
    pub admin_permission: String,
    /// `LIMINA_WORKOS_LEEWAY_SECONDS` (default 30, 0–300).
    pub leeway_seconds: i64,
    pub jwks_policy: JwksPolicy,
}

impl std::fmt::Debug for WorkOsSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkOsSettings")
            .field("client_id", &self.client_id)
            .field("api_base_url", &self.api_base_url)
            .field("redirect_uri", &self.redirect_uri)
            .field("public_url", &self.public_url)
            .field("issuers", &self.issuers)
            .field("organization_id", &self.organization_id)
            .field("required_permission", &self.required_permission)
            .field("admin_permission", &self.admin_permission)
            .field("leeway_seconds", &self.leeway_seconds)
            .finish_non_exhaustive()
    }
}

impl WorkOsSettings {
    /// Validated settings with the defaults for everything else.
    pub fn new(
        client_id: impl Into<String>,
        api_key: impl Into<String>,
        cookie_password: impl Into<String>,
    ) -> std::result::Result<Self, AuthConfigError> {
        let settings = Self {
            client_id: client_id.into(),
            api_key: api_key.into(),
            cookie_password: cookie_password.into(),
            api_base_url: "https://api.workos.com".to_owned(),
            redirect_uri: None,
            public_url: None,
            issuers: Vec::new(),
            organization_id: None,
            required_permission: None,
            admin_permission: DEFAULT_ADMIN_PERMISSION.to_owned(),
            leeway_seconds: 30,
            jwks_policy: JwksPolicy::WORKOS,
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> std::result::Result<(), AuthConfigError> {
        if !self.client_id.starts_with("client_") {
            return Err(AuthConfigError::new(
                "WORKOS_CLIENT_ID must be a WorkOS client id (client_…).",
            ));
        }
        if self.api_key.is_empty() {
            return Err(AuthConfigError::new(
                "WORKOS_API_KEY is required when WORKOS_CLIENT_ID is set.",
            ));
        }
        if self.cookie_password.chars().count() < 32 {
            return Err(AuthConfigError::new(
                "WORKOS_COOKIE_PASSWORD must be at least 32 characters.",
            ));
        }
        if !(0..=300).contains(&self.leeway_seconds) {
            return Err(AuthConfigError::new(
                "WorkOS clock-skew leeway must be between 0 and 300 seconds.",
            ));
        }
        Ok(())
    }

    /// The issuer when discovery is unavailable: the pinned issuers, or the API host's User
    /// Management issuer for this client.
    pub fn expected_issuers(&self) -> Vec<String> {
        if self.issuers.is_empty() {
            vec![format!("{}/user_management/{}", self.api_base_url, self.client_id)]
        } else {
            self.issuers.clone()
        }
    }

    pub fn discovery_url(&self) -> String {
        format!(
            "{}/user_management/{}/.well-known/openid-configuration",
            self.api_base_url, self.client_id
        )
    }

    pub fn jwks_url(&self) -> String {
        format!("{}/sso/jwks/{}", self.api_base_url, self.client_id)
    }

    /// `None` unless `WORKOS_CLIENT_ID` is set.
    pub(crate) fn from_lookup(
        lookup: &Lookup<'_>,
    ) -> std::result::Result<Option<Self>, AuthConfigError> {
        let client_id = lookup.trimmed("WORKOS_CLIENT_ID");
        if client_id.is_empty() {
            return Ok(None);
        }
        let issuers = lookup
            .get("LIMINA_WORKOS_ISSUER")
            .unwrap_or_default()
            .split(',')
            .map(|item| item.trim().trim_end_matches('/').to_owned())
            .filter(|item| !item.is_empty())
            .collect();
        let hostname =
            lookup.get("WORKOS_API_HOSTNAME").unwrap_or_else(|| "api.workos.com".to_owned());
        let settings = Self {
            client_id,
            api_key: lookup.trimmed("WORKOS_API_KEY"),
            cookie_password: lookup.get("WORKOS_COOKIE_PASSWORD").unwrap_or_default(),
            api_base_url: api_base_url(&hostname)?,
            redirect_uri: Some(lookup.trimmed("WORKOS_REDIRECT_URI")).filter(|v| !v.is_empty()),
            public_url: Some(lookup.trimmed("LIMINA_PUBLIC_URL").trim_end_matches('/').to_owned())
                .filter(|value| !value.is_empty()),
            issuers,
            organization_id: Some(lookup.trimmed("LIMINA_WORKOS_ORGANIZATION_ID"))
                .filter(|value| !value.is_empty()),
            required_permission: Some(lookup.trimmed("LIMINA_WORKOS_REQUIRED_PERMISSION"))
                .filter(|value| !value.is_empty()),
            admin_permission: lookup
                .get("LIMINA_WORKOS_ADMIN_PERMISSION")
                .map_or_else(|| DEFAULT_ADMIN_PERMISSION.to_owned(), |v| v.trim().to_owned()),
            leeway_seconds: lookup.integer("LIMINA_WORKOS_LEEWAY_SECONDS", 30)?,
            jwks_policy: JwksPolicy::WORKOS,
        };
        settings.validate()?;
        Ok(Some(settings))
    }
}

/// The WorkOS API base URL from `WORKOS_API_HOSTNAME`: a bare hostname means HTTPS; a full URL
/// must be HTTPS, except plain HTTP to a loopback host (local test servers). Only the scheme and
/// `host[:port]` are kept.
pub fn api_base_url(value: &str) -> std::result::Result<String, AuthConfigError> {
    let raw = value.trim().trim_end_matches('/');
    let Some((scheme, rest)) = raw.split_once("://") else {
        if raw.is_empty() || raw.contains('/') {
            return Err(AuthConfigError::new(
                "WORKOS_API_HOSTNAME must be a hostname such as api.workos.com.",
            ));
        }
        return Ok(format!("https://{raw}"));
    };
    let scheme = scheme.to_ascii_lowercase();
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let loopback = || {
        url::Url::parse(raw).ok().and_then(|url| url.host_str().map(is_loopback)).unwrap_or(false)
    };
    if scheme == "https" || (scheme == "http" && loopback()) {
        return Ok(format!("{scheme}://{netloc}"));
    }
    Err(AuthConfigError::new(
        "WORKOS_API_HOSTNAME must use HTTPS (plain HTTP is allowed for loopback).",
    ))
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Only same-origin absolute paths; never a protocol-relative or external URL.
pub fn safe_return_path(value: Option<&str>) -> String {
    match value {
        Some(path) if path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') => {
            path.to_owned()
        }
        _ => "/".to_owned(),
    }
}

/// A sealed session cookie turned into a principal.
#[derive(Debug, Clone)]
pub struct ResolvedSession {
    pub principal: Principal,
    /// The WorkOS session id (`sid`), empty when the token has none.
    pub session_id: String,
    /// A new sealed value when the access token was refreshed; the response must set it.
    pub refreshed_cookie: Option<String>,
}

/// The pinned issuer(s) and signing keys for this client.
pub struct Verification {
    issuers: Vec<String>,
    jwks: Jwks,
}

impl Verification {
    pub fn issuers(&self) -> &[String] {
        &self.issuers
    }

    pub fn jwks(&self) -> &Jwks {
        &self.jwks
    }
}

#[derive(Default)]
struct DiscoveryState {
    failed_at: Option<Instant>,
    /// Used while discovery fails, so the fallback keys stay cached between requests.
    fallback: Option<Arc<Verification>>,
}

type RefreshCell = Arc<tokio::sync::OnceCell<Map<String, Value>>>;

/// AuthKit flow, token verification and sealed sessions for one WorkOS client.
pub struct WorkOsAuth {
    settings: WorkOsSettings,
    http: reqwest::Client,
    jwks_override: Option<Jwks>,
    verification: OnceLock<Arc<Verification>>,
    discovery: tokio::sync::Mutex<DiscoveryState>,
    sessions: SessionSealer,
    flows: SessionSealer,
    refreshes: Mutex<HashMap<String, (Instant, RefreshCell)>>,
}

impl WorkOsAuth {
    pub fn new(
        settings: WorkOsSettings,
        http: reqwest::Client,
    ) -> std::result::Result<Self, AuthConfigError> {
        settings.validate()?;
        let sessions = SessionSealer::new(&settings.cookie_password, "session")?;
        let flows = SessionSealer::new(&settings.cookie_password, "login-flow")?;
        Ok(Self {
            settings,
            http,
            jwks_override: None,
            verification: OnceLock::new(),
            discovery: tokio::sync::Mutex::new(DiscoveryState::default()),
            sessions,
            flows,
            refreshes: Mutex::new(HashMap::new()),
        })
    }

    /// Verifies tokens with these keys instead of the discovered (or default) key set.
    #[must_use]
    pub fn with_jwks(mut self, jwks: Jwks) -> Self {
        self.jwks_override = Some(jwks);
        self
    }

    pub fn settings(&self) -> &WorkOsSettings {
        &self.settings
    }

    /// The environment's User Management directory, for sharing projects by email.
    pub fn directory(&self) -> super::directory::WorkOsDirectory {
        super::directory::WorkOsDirectory::new(&self.settings, self.http.clone())
    }

    /// The sealer for `limina_session` values.
    pub fn sessions(&self) -> &SessionSealer {
        &self.sessions
    }

    // ─── Tokens ─────────────────────────────────────────────────────────────

    /// The pinned issuer(s) and signing keys for this client.
    ///
    /// WorkOS publishes both in the client's OpenID configuration. Environments with a custom
    /// authentication domain issue tokens from that domain, so the issuer cannot be derived from
    /// the API host alone. An explicit `LIMINA_WORKOS_ISSUER` skips discovery. When discovery
    /// fails, the API-host issuer and key set are used and discovery is retried a minute later.
    pub async fn verification(&self) -> Arc<Verification> {
        if let Some(verification) = self.verification.get() {
            return verification.clone();
        }
        let mut state = self.discovery.lock().await;
        if let Some(verification) = self.verification.get() {
            return verification.clone();
        }
        if !self.settings.issuers.is_empty() {
            let pinned =
                self.verification_for(self.settings.issuers.clone(), self.settings.jwks_url());
            return self.verification.get_or_init(|| pinned).clone();
        }
        if state.failed_at.is_none_or(|at| at.elapsed() > DISCOVERY_RETRY) {
            if let Some((issuer, jwks_uri)) = self.discover().await {
                let discovered = self.verification_for(vec![issuer], jwks_uri);
                return self.verification.get_or_init(|| discovered).clone();
            }
            state.failed_at = Some(Instant::now());
        }
        let fallback = state.fallback.get_or_insert_with(|| {
            self.verification_for(self.settings.expected_issuers(), self.settings.jwks_url())
        });
        fallback.clone()
    }

    fn verification_for(&self, issuers: Vec<String>, jwks_url: String) -> Arc<Verification> {
        let jwks = self.jwks_override.clone().unwrap_or_else(|| {
            Jwks::remote(jwks_url, self.http.clone(), self.settings.jwks_policy)
        });
        Arc::new(Verification { issuers, jwks })
    }

    /// Verifies signature (RS256), issuer and lifetime. `TokenError::Expired` means a refresh may
    /// help.
    pub async fn verify_access_token(
        &self,
        token: &str,
    ) -> std::result::Result<Map<String, Value>, TokenError> {
        let verification = self.verification().await;
        let rules = JwtRules {
            algorithms: &[Algorithm::RS256],
            issuers: &verification.issuers,
            audience: None,
            leeway_seconds: self.settings.leeway_seconds.unsigned_abs(),
        };
        verify_jwt(&verification.jwks, token, &rules).await
    }

    /// Applies the configured organization and permission policy to verified claims.
    pub fn principal(
        &self,
        claims: &Map<String, Value>,
        user: Option<&Value>,
    ) -> Result<Principal> {
        let subject = claims.get("sub").map(py_str).unwrap_or_default().trim().to_owned();
        if subject.is_empty() {
            return Err(LiminaError::authentication_with("The WorkOS token has no subject."));
        }
        if let Some(expected) = &self.settings.organization_id
            && claims.get("org_id").and_then(Value::as_str) != Some(expected.as_str())
        {
            return Err(LiminaError::authorization_with(
                "Your account is not part of the organization this Limina runtime serves.",
            )
            .with("reason", "organization"));
        }
        let permissions: Vec<&str> = claims
            .get("permissions")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if let Some(required) = &self.settings.required_permission
            && !permissions.contains(&required.as_str())
        {
            return Err(LiminaError::authorization_with(
                "Your account has not been granted access to this Limina runtime.",
            )
            .with("reason", "permission")
            .with("required_permission", required.as_str()));
        }
        let empty = Map::new();
        let profile = user.and_then(Value::as_object).unwrap_or(&empty);
        let name = ["first_name", "last_name"]
            .into_iter()
            .filter_map(|field| profile.get(field).filter(|value| truthy(Some(value))))
            .map(|value| py_str(value).trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        // Pending invitations bind only when the source of this email explicitly verifies it.
        let (email, email_verified) =
            [(profile.get("email"), profile), (claims.get("email"), claims)]
                .into_iter()
                .find(|(value, _)| truthy(*value))
                .map(|(value, source)| {
                    let email = value.map(|value| py_str(value).trim().to_owned());
                    (email, source.get("email_verified") == Some(&Value::Bool(true)))
                })
                .unwrap_or((None, false));
        let email = email.filter(|email| !email.is_empty());
        let display_name = [Some(name.as_str()), email.as_deref(), Some(subject.as_str())]
            .into_iter()
            .flatten()
            .find(|candidate| !candidate.is_empty())
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect::<String>();
        let admin_permission = self.settings.admin_permission.as_str();
        let organization = claims.get("org_id").and_then(Value::as_str).map(str::to_owned);
        Ok(Principal::new(subject, display_name)
            .with_email(email)
            .email_verified(email_verified)
            .organization(organization)
            .admin(!admin_permission.is_empty() && permissions.contains(&admin_permission))
            .mode("workos"))
    }

    // ─── AuthKit flow ───────────────────────────────────────────────────────

    pub fn redirect_uri(&self, request_origin: &str) -> String {
        if let Some(uri) = &self.settings.redirect_uri {
            return uri.clone();
        }
        let base = self.settings.public_url.as_deref().unwrap_or(request_origin);
        format!("{base}/auth/callback")
    }

    /// The AuthKit authorization URL and the sealed flow cookie that goes with it.
    pub fn start_login(
        &self,
        request_origin: &str,
        return_to: Option<&str>,
    ) -> Result<(String, String)> {
        let state = token_urlsafe(24)?;
        let (verifier, challenge) = pkce_pair()?;
        let redirect_uri = self.redirect_uri(request_origin);
        let mut params = form_urlencoded::Serializer::new(String::new());
        params
            .append_pair("client_id", &self.settings.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("provider", "authkit")
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        if let Some(organization) = &self.settings.organization_id {
            params.append_pair("organization_id", organization);
        }
        let flow = self.flows.seal(&json!({
            "state": state,
            "verifier": verifier,
            "return_to": safe_return_path(return_to),
            "expires": unix_now() + FLOW_TTL_SECONDS,
        }))?;
        let url =
            format!("{}/user_management/authorize?{}", self.settings.api_base_url, params.finish());
        Ok((url, flow))
    }

    /// Exchanges the code; returns the sealed session cookie and the return path.
    ///
    /// Failures carry a `reason` detail the sign-in page shows: `state` (expired or forged flow),
    /// `unreachable`/`rejected` (WorkOS), `token` (unverifiable access token), or the policy
    /// reasons `organization` and `permission` (403).
    pub async fn finish_login(
        &self,
        code: &str,
        state: &str,
        flow_cookie: Option<&str>,
    ) -> Result<(String, String)> {
        let flow = self.flows.unseal(flow_cookie.unwrap_or_default());
        let flow = flow.filter(|flow| {
            let expires = flow.get("expires").and_then(Value::as_u64).unwrap_or(0);
            let expected = flow.get("state").map(py_str).unwrap_or_default();
            expires >= unix_now() && bool::from(expected.as_bytes().ct_eq(state.as_bytes()))
        });
        let Some(flow) = flow else {
            return Err(LiminaError::authentication_with(
                "The sign-in attempt expired. Please try again.",
            )
            .with("reason", "state"));
        };
        let verifier = flow.get("verifier").map(py_str).unwrap_or_default();
        let response = self
            .authenticate_grant(json!({
                "grant_type": "authorization_code",
                "code": code,
                "code_verifier": verifier,
            }))
            .await?;
        let access_token = response.get("access_token").map(py_str).unwrap_or_default();
        let claims = match self.verify_access_token(&access_token).await {
            Ok(claims) => claims,
            Err(error) => {
                let token_issuer = unverified_claims(&access_token)
                    .and_then(|claims| claims.get("iss").cloned())
                    .unwrap_or(Value::Null);
                let expected = self.verification().await.issuers.clone();
                tracing::warn!(
                    %error,
                    %token_issuer,
                    ?expected,
                    "WorkOS access token rejected; set LIMINA_WORKOS_ISSUER if this environment \
                     uses another issuer"
                );
                return Err(LiminaError::authentication_with(
                    "The WorkOS session could not be verified.",
                )
                .with("reason", "token"));
            }
        };
        self.principal(&claims, response.get("user"))?;
        let return_to = flow.get("return_to").and_then(Value::as_str);
        Ok((self.seal_session(&response)?, safe_return_path(return_to)))
    }

    /// Authenticates a sealed session cookie, refreshing it when the access token expired.
    pub async fn resolve(&self, cookie_value: &str) -> Result<ResolvedSession> {
        let invalid =
            || LiminaError::authentication_with("Your session is invalid. Please sign in again.");
        let mut session = self.sessions.unseal(cookie_value).ok_or_else(invalid)?;
        let access_token = session
            .get("access_token")
            .filter(|value| truthy(Some(value)))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?
            .to_owned();
        let mut refreshed = None;
        let claims = match self.verify_access_token(&access_token).await {
            Ok(claims) => claims,
            Err(TokenError::Expired) => {
                session = self.refresh(&session).await?;
                refreshed = Some(self.seal_session(&session)?);
                let renewed = session.get("access_token").map(py_str).unwrap_or_default();
                self.verify_access_token(&renewed).await.map_err(|_| invalid())?
            }
            Err(TokenError::Invalid(_)) => return Err(invalid()),
        };
        let principal = self.principal(&claims, session.get("user"))?;
        let session_id = claims.get("sid").map(py_str).unwrap_or_default();
        Ok(ResolvedSession { principal, session_id, refreshed_cookie: refreshed })
    }

    /// The WorkOS logout URL for the session, so the next sign-in cannot silently reuse it.
    pub fn logout_url(&self, cookie_value: Option<&str>, return_to: &str) -> Option<String> {
        let session = self.sessions.unseal(cookie_value.unwrap_or_default())?;
        let access_token = session.get("access_token").filter(|value| truthy(Some(value)))?;
        let claims = unverified_claims(access_token.as_str()?)?;
        let session_id = claims.get("sid").filter(|value| truthy(Some(value)))?;
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("session_id", &py_str(session_id))
            .append_pair("return_to", return_to)
            .finish();
        Some(format!("{}/user_management/sessions/logout?{query}", self.settings.api_base_url))
    }

    // ─── Internals ──────────────────────────────────────────────────────────

    fn seal_session(&self, response: &Map<String, Value>) -> Result<String> {
        let empty = Map::new();
        let user = response.get("user").and_then(Value::as_object).unwrap_or(&empty);
        let profile: Map<String, Value> =
            ["id", "email", "first_name", "last_name", "email_verified"]
                .into_iter()
                .map(|key| (key.to_owned(), user.get(key).cloned().unwrap_or(Value::Null)))
                .collect();
        self.sessions.seal(&json!({
            "access_token": response.get("access_token").cloned().unwrap_or(Value::Null),
            "refresh_token": response.get("refresh_token").cloned().unwrap_or(Value::Null),
            "user": profile,
        }))
    }

    /// Exchanges the session's refresh token. Requests carrying the same expired session share
    /// one exchange (and its result) for a minute, whatever their order.
    async fn refresh(&self, session: &Map<String, Value>) -> Result<Map<String, Value>> {
        let expired =
            || LiminaError::authentication_with("Your session expired. Please sign in again.");
        let refresh_token = session
            .get("refresh_token")
            .filter(|value| truthy(Some(value)))
            .map(py_str)
            .ok_or_else(expired)?;
        let cell = {
            let mut refreshes = self.refreshes.lock().unwrap_or_else(PoisonError::into_inner);
            refreshes.retain(|_, (created, _)| created.elapsed() < REFRESH_REUSE);
            refreshes
                .entry(sha256_hex(&refresh_token))
                .or_insert_with(|| (Instant::now(), RefreshCell::default()))
                .1
                .clone()
        };
        let response = cell
            .get_or_try_init(|| async {
                let mut response = self
                    .authenticate_grant(json!({
                        "grant_type": "refresh_token",
                        "refresh_token": refresh_token,
                    }))
                    .await
                    .map_err(|_| expired())?;
                if !response.contains_key("user") {
                    let user = session.get("user").cloned().unwrap_or(Value::Null);
                    response.insert("user".to_owned(), user);
                }
                Ok::<_, LiminaError>(response)
            })
            .await?;
        Ok(response.clone())
    }

    /// The issuer and key set URL from the client's OpenID configuration, when usable.
    async fn discover(&self) -> Option<(String, String)> {
        let response = self.http.get(self.settings.discovery_url()).send().await.ok()?;
        if response.status() != reqwest::StatusCode::OK {
            return None;
        }
        let metadata: Value = response.json().await.ok()?;
        let issuer = metadata.get("issuer")?.as_str()?;
        let jwks_uri = metadata.get("jwks_uri")?.as_str()?;
        api_base_url(jwks_uri).ok()?; // keys only over HTTPS (or a loopback test server)
        Some((issuer.trim_end_matches('/').to_owned(), jwks_uri.to_owned()))
    }

    /// `POST /user_management/authenticate` with the client credentials.
    async fn authenticate_grant(&self, grant: Value) -> Result<Map<String, Value>> {
        let mut body = Map::new();
        body.insert("client_id".to_owned(), json!(self.settings.client_id));
        body.insert("client_secret".to_owned(), json!(self.settings.api_key));
        if let Value::Object(grant) = grant {
            body.extend(grant);
        }
        let url = format!("{}/user_management/authenticate", self.settings.api_base_url);
        let response = self.http.post(url).json(&body).send().await.map_err(|error| {
            tracing::warn!(%error, "WorkOS could not be reached");
            LiminaError::authentication_with("WorkOS could not be reached.")
                .with("reason", "unreachable")
        })?;
        if response.status().as_u16() >= 400 {
            return Err(LiminaError::authentication_with("WorkOS rejected the sign-in.")
                .with("reason", "rejected"));
        }
        let incomplete = || {
            LiminaError::authentication_with("WorkOS returned an incomplete session.")
                .with("reason", "rejected")
        };
        let payload = match response.json::<Value>().await {
            Ok(Value::Object(payload)) => payload,
            _ => return Err(incomplete()),
        };
        if !truthy(payload.get("access_token")) || !truthy(payload.get("refresh_token")) {
            return Err(incomplete());
        }
        Ok(payload)
    }
}

#[async_trait]
impl Authenticator for WorkOsAuth {
    fn mode(&self) -> &str {
        "workos"
    }

    /// A WorkOS access token presented as a bearer credential. The actor hint is ignored.
    async fn authenticate(
        &self,
        bearer_token: Option<&str>,
        _actor_hint: Option<&str>,
    ) -> Result<Principal> {
        let Some(token) = bearer_token.filter(|token| !token.is_empty()) else {
            return Err(LiminaError::authentication());
        };
        let claims = self.verify_access_token(token).await.map_err(|error| {
            tracing::debug!(%error, "WorkOS bearer token rejected");
            LiminaError::authentication_with("The bearer token is invalid or expired.")
        })?;
        self.principal(&claims, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_hosts_must_be_https_or_loopback() {
        assert_eq!(api_base_url("api.workos.com").unwrap(), "https://api.workos.com");
        assert_eq!(
            api_base_url(" https://auth.example.com/ ").unwrap(),
            "https://auth.example.com"
        );
        assert_eq!(
            api_base_url("https://auth.example.com/sso/jwks/x").unwrap(),
            "https://auth.example.com"
        );
        assert_eq!(api_base_url("http://127.0.0.1:9000").unwrap(), "http://127.0.0.1:9000");
        assert_eq!(api_base_url("http://localhost:9000").unwrap(), "http://localhost:9000");
        assert_eq!(api_base_url("http://[::1]:9000").unwrap(), "http://[::1]:9000");
        for bad in
            ["", "api.workos.com/path", "http://auth.example.com", "http://10.0.0.1", "ftp://x"]
        {
            assert!(api_base_url(bad).is_err(), "{bad}");
        }
    }
}
