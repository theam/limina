//! The provider-neutral authenticator boundary shared by REST, WebSocket and MCP.

use std::sync::Arc;

use async_trait::async_trait;
use limina_core::{ErrorKind, LiminaError, Principal, Result};

use super::config::AuthConfigError;
use super::rate_limit::FailureRateLimiter;

/// Turns a bearer credential into a [`Principal`].
///
/// Implementations fail with `authentication_required` (401) when the credential is missing or not
/// theirs, and with `permission_denied` (403) when the credential is valid but its owner may not use
/// this runtime. A [`ChainedAuthenticator`] tries the next authenticator only after a 401.
#[async_trait]
pub trait Authenticator: Send + Sync {
    /// How principals authenticated here are reported (`local`, `oidc` or `workos`); `/healthz`
    /// shows it as `auth_mode`.
    fn mode(&self) -> &str;

    /// False only for the open local mode, where anonymous callers are signed in as the local
    /// administrator. Drives what `/auth/config` tells the console.
    fn requires_credentials(&self) -> bool {
        true
    }

    /// `actor_hint` is the caller-supplied display name (`X-Limina-Actor`). Only static tokens use
    /// it; identity providers always take the identity from signed claims.
    async fn authenticate(
        &self,
        bearer_token: Option<&str>,
        actor_hint: Option<&str>,
    ) -> Result<Principal>;
}

/// The key every transport shares in the global failure ceiling.
pub const ALL_TRANSPORTS: &str = "all-transports";

/// Transport-neutral brute-force guard used by REST, WebSocket and MCP: every failed
/// authentication counts against one global ceiling, whatever the caller's address.
pub struct RateLimitedAuthenticator {
    wrapped: Arc<dyn Authenticator>,
    limiter: FailureRateLimiter,
}

impl RateLimitedAuthenticator {
    pub fn new(wrapped: Arc<dyn Authenticator>, limiter: FailureRateLimiter) -> Self {
        Self { wrapped, limiter }
    }

    pub fn wrapped(&self) -> &Arc<dyn Authenticator> {
        &self.wrapped
    }
}

#[async_trait]
impl Authenticator for RateLimitedAuthenticator {
    fn mode(&self) -> &str {
        self.wrapped.mode()
    }

    fn requires_credentials(&self) -> bool {
        self.wrapped.requires_credentials()
    }

    async fn authenticate(
        &self,
        bearer_token: Option<&str>,
        actor_hint: Option<&str>,
    ) -> Result<Principal> {
        self.limiter.check(ALL_TRANSPORTS)?;
        let result = self.wrapped.authenticate(bearer_token, actor_hint).await;
        if is_authentication_failure(&result) {
            self.limiter.failure(ALL_TRANSPORTS);
        }
        result
    }
}

/// Accepts the first credential type that recognises a bearer token.
///
/// Used when an identity provider signs people in and optional static tokens remain available for
/// automation (CLI, MCP, CI). A chain never grants anonymous access.
pub struct ChainedAuthenticator {
    mode: String,
    authenticators: Vec<Arc<dyn Authenticator>>,
}

impl ChainedAuthenticator {
    pub fn new(
        mode: impl Into<String>,
        authenticators: Vec<Arc<dyn Authenticator>>,
    ) -> std::result::Result<Self, AuthConfigError> {
        if authenticators.is_empty() {
            return Err(AuthConfigError::new("At least one authenticator is required."));
        }
        Ok(Self { mode: mode.into(), authenticators })
    }
}

#[async_trait]
impl Authenticator for ChainedAuthenticator {
    fn mode(&self) -> &str {
        &self.mode
    }

    async fn authenticate(
        &self,
        bearer_token: Option<&str>,
        actor_hint: Option<&str>,
    ) -> Result<Principal> {
        let Some(token) = bearer_token.filter(|token| !token.is_empty()) else {
            return Err(LiminaError::authentication());
        };
        let mut failure = None;
        for authenticator in &self.authenticators {
            match authenticator.authenticate(Some(token), actor_hint).await {
                Err(error) if error.kind == ErrorKind::Authentication => failure = Some(error),
                outcome => return outcome,
            }
        }
        Err(failure.unwrap_or_else(LiminaError::authentication))
    }
}

pub(crate) fn is_authentication_failure<T>(result: &Result<T>) -> bool {
    matches!(result, Err(error) if error.kind == ErrorKind::Authentication)
}
