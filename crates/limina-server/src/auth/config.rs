//! Authentication configuration from the environment.
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `LIMINA_API_TOKEN` | – | Project token (local token mode; machine token next to WorkOS) |
//! | `LIMINA_ADMIN_API_TOKEN` | – | Instance-administrator token; must differ from the project token |
//! | `LIMINA_ALLOW_INSECURE_NO_AUTH` | off | `1`/`true`/`yes`: allow running without a project token |
//! | `LIMINA_OIDC_ISSUER`, `LIMINA_OIDC_AUDIENCE` | – | OIDC mode (both required, issuer HTTPS) |
//! | `LIMINA_OIDC_JWKS_URL` | discovered | Key set URL (HTTPS) |
//! | `LIMINA_OIDC_ALGORITHMS` | `RS256` | Comma-separated public-key algorithms |
//! | `LIMINA_OIDC_ADMIN_CLAIM`, `LIMINA_OIDC_ADMIN_VALUE` | –, `admin` | Instance-admin mapping |
//! | `LIMINA_OIDC_LEEWAY_SECONDS` | 30 | Clock skew, 0–300 |
//! | `WORKOS_CLIENT_ID` | – | WorkOS mode (`client_…`) |
//! | `WORKOS_API_KEY` | – | Required with WorkOS |
//! | `WORKOS_COOKIE_PASSWORD` | – | ≥ 32 characters; seals the cookies |
//! | `WORKOS_API_HOSTNAME` | `api.workos.com` | Hostname, or HTTPS URL (HTTP only for loopback) |
//! | `WORKOS_REDIRECT_URI` | `<public URL>/auth/callback` | AuthKit redirect URI |
//! | `LIMINA_PUBLIC_URL` | request origin | Console URL: redirect URI, write origin, `Secure` cookies |
//! | `LIMINA_WORKOS_ISSUER` | discovered | Comma-separated pinned issuers |
//! | `LIMINA_WORKOS_ORGANIZATION_ID` | – | Only this organization's members |
//! | `LIMINA_WORKOS_REQUIRED_PERMISSION` | – | Permission every user needs |
//! | `LIMINA_WORKOS_ADMIN_PERMISSION` | `limina:admin` | Permission for instance administrators |
//! | `LIMINA_WORKOS_LEEWAY_SECONDS` | 30 | Clock skew, 0–300 |
//! | `LIMINA_AUTH_FAILURE_LIMIT` | 10 | Failed attempts per client address per window |
//! | `LIMINA_GLOBAL_AUTH_FAILURE_LIMIT` | 1000 | Failed attempts across all transports per window |
//! | `LIMINA_AUTH_FAILURE_WINDOW_SECONDS` | 60 | The sliding window of both limits |
//! | `FORWARDED_ALLOW_IPS` | `127.0.0.1` | Proxies whose `X-Forwarded-Proto`/`-For` are believed (`*` = any) |
//!
//! WorkOS and OIDC are mutually exclusive. With WorkOS, the static tokens stay available as machine
//! credentials; with OIDC they are ignored.

use std::time::Duration;

use super::oidc::OidcSettings;
use super::proxy::TrustedProxies;
use super::rate_limit::RateLimitSettings;
use super::workos::WorkOsSettings;

/// A configuration the runtime must refuse to start with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct AuthConfigError {
    message: String,
}

impl AuthConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Reads configuration values by name (the process environment, or a map in tests).
pub(crate) struct Lookup<'a>(&'a dyn Fn(&str) -> Option<String>);

impl Lookup<'_> {
    pub(crate) fn get(&self, name: &str) -> Option<String> {
        (self.0)(name)
    }

    /// The value with surrounding whitespace removed; empty when unset.
    pub(crate) fn trimmed(&self, name: &str) -> String {
        self.get(name).map(|value| value.trim().to_owned()).unwrap_or_default()
    }

    /// The value verbatim, or `None` when unset or empty.
    pub(crate) fn non_empty(&self, name: &str) -> Option<String> {
        self.get(name).filter(|value| !value.is_empty())
    }

    pub(crate) fn integer(&self, name: &str, default: i64) -> Result<i64, AuthConfigError> {
        match self.get(name) {
            None => Ok(default),
            Some(value) => value
                .trim()
                .parse()
                .map_err(|_| AuthConfigError::new(format!("{name} must be an integer."))),
        }
    }

    fn flag(&self, name: &str) -> bool {
        matches!(self.get(name).unwrap_or_default().to_lowercase().as_str(), "1" | "true" | "yes")
    }
}

/// The failure limits applied in front of every authenticator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthLimits {
    /// Per client address (`LIMINA_AUTH_FAILURE_LIMIT`, default 10 per 60 s).
    pub client: RateLimitSettings,
    /// Across all transports (`LIMINA_GLOBAL_AUTH_FAILURE_LIMIT`, default 1000 per 60 s).
    pub global: RateLimitSettings,
}

impl Default for AuthLimits {
    fn default() -> Self {
        Self { client: RateLimitSettings::new(10, 60), global: RateLimitSettings::new(1000, 60) }
    }
}

/// Request-handling options of an [`Auth`](super::Auth) stack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthOptions {
    pub limits: AuthLimits,
    pub trusted_proxies: TrustedProxies,
}

/// Everything [`Auth::from_config`](super::Auth::from_config) needs. Fields are public so the
/// CLI can override them with flags (`--token`, `--admin-token`).
#[derive(Clone)]
pub struct AuthConfig {
    pub api_token: Option<String>,
    pub admin_api_token: Option<String>,
    pub allow_insecure_no_auth: bool,
    pub oidc: Option<OidcSettings>,
    pub workos: Option<WorkOsSettings>,
    pub limits: AuthLimits,
    pub trusted_proxies: TrustedProxies,
    /// Timeout of every call to the identity provider (discovery, keys, code exchange).
    pub http_timeout: Duration,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("api_token", &self.api_token.as_ref().map(|_| "<redacted>"))
            .field("admin_api_token", &self.admin_api_token.as_ref().map(|_| "<redacted>"))
            .field("allow_insecure_no_auth", &self.allow_insecure_no_auth)
            .field("oidc", &self.oidc)
            .field("workos", &self.workos)
            .field("limits", &self.limits)
            .field("trusted_proxies", &self.trusted_proxies)
            .field("http_timeout", &self.http_timeout)
            .finish()
    }
}

impl Default for AuthConfig {
    /// No credentials and no identity provider: refused unless `allow_insecure_no_auth` is set.
    fn default() -> Self {
        Self {
            api_token: None,
            admin_api_token: None,
            allow_insecure_no_auth: false,
            oidc: None,
            workos: None,
            limits: AuthLimits::default(),
            trusted_proxies: TrustedProxies::default(),
            http_timeout: Duration::from_secs(10),
        }
    }
}

impl AuthConfig {
    /// Reads the process environment.
    pub fn from_env() -> Result<Self, AuthConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads configuration through `lookup` (variable name → value).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, AuthConfigError> {
        let lookup = Lookup(&lookup);
        let workos = WorkOsSettings::from_lookup(&lookup)?;
        if workos.is_some() && !lookup.trimmed("LIMINA_OIDC_ISSUER").is_empty() {
            return Err(AuthConfigError::new(
                "Configure either WorkOS or LIMINA_OIDC_ISSUER, not both.",
            ));
        }
        // With WorkOS, the OIDC variables are not read at all.
        let oidc = if workos.is_some() { None } else { OidcSettings::from_lookup(&lookup)? };
        let window = lookup.integer("LIMINA_AUTH_FAILURE_WINDOW_SECONDS", 60)?;
        Ok(Self {
            api_token: lookup.non_empty("LIMINA_API_TOKEN"),
            admin_api_token: lookup.non_empty("LIMINA_ADMIN_API_TOKEN"),
            allow_insecure_no_auth: lookup.flag("LIMINA_ALLOW_INSECURE_NO_AUTH"),
            oidc,
            workos,
            limits: AuthLimits {
                client: RateLimitSettings::new(
                    lookup.integer("LIMINA_AUTH_FAILURE_LIMIT", 10)?,
                    window,
                ),
                global: RateLimitSettings::new(
                    lookup.integer("LIMINA_GLOBAL_AUTH_FAILURE_LIMIT", 1000)?,
                    window,
                ),
            },
            trusted_proxies: match lookup.get("FORWARDED_ALLOW_IPS") {
                Some(value) => TrustedProxies::parse(&value)?,
                None => TrustedProxies::default(),
            },
            http_timeout: Duration::from_secs(10),
        })
    }

    /// Refuses to listen on a non-loopback host without a token, WorkOS, OIDC or the explicit
    /// insecure opt-in.
    pub fn check_bind(&self, host: &str) -> Result<(), AuthConfigError> {
        let local = matches!(host, "127.0.0.1" | "localhost" | "::1");
        let configured = self.api_token.is_some()
            || self.admin_api_token.is_some()
            || self.oidc.is_some()
            || self.workos.is_some()
            || self.allow_insecure_no_auth;
        if local || configured {
            return Ok(());
        }
        Err(AuthConfigError::new(
            "a shared token, WorkOS, or OIDC configuration is required for a non-local bind",
        ))
    }
}
