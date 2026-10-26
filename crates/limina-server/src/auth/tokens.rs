//! Static API tokens for a trusted local instance and for automation next to an identity provider.

use async_trait::async_trait;
use limina_core::{LiminaError, Principal, Result};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::authenticator::Authenticator;
use super::config::AuthConfigError;

/// Compares two credentials in constant time.
///
/// Both sides are hashed first, so the comparison leaks neither the content nor the length of the
/// configured token. Arbitrary Unicode input never fails.
pub fn tokens_equal(left: &str, right: &str) -> bool {
    let left = Sha256::digest(left.as_bytes());
    let right = Sha256::digest(right.as_bytes());
    left.as_slice().ct_eq(right.as_slice()).into()
}

/// Shared-token authentication for a local instance (`LIMINA_API_TOKEN`, `LIMINA_ADMIN_API_TOKEN`).
///
/// | Configured            | No bearer            | Project token     | Admin token        |
/// |-----------------------|----------------------|-------------------|--------------------|
/// | nothing (open mode)   | local admin          | local admin       | –                  |
/// | project token         | 401                  | local user        | local admin        |
/// | admin token only      | 401                  | –                 | local admin        |
///
/// Every principal is a project administrator (`project_admin`); only the admin token (or open
/// mode) makes it an instance administrator. The display name comes from the actor hint.
#[derive(Clone)]
pub struct LocalAuthenticator {
    token: Option<String>,
    admin_token: Option<String>,
}

impl std::fmt::Debug for LocalAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAuthenticator")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("admin_token", &self.admin_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl LocalAuthenticator {
    pub fn new(
        token: Option<String>,
        admin_token: Option<String>,
    ) -> std::result::Result<Self, AuthConfigError> {
        if let (Some(token), Some(admin)) = (non_empty(&token), non_empty(&admin_token))
            && tokens_equal(token, admin)
        {
            return Err(AuthConfigError::new(
                "LIMINA_API_TOKEN and LIMINA_ADMIN_API_TOKEN must be different.",
            ));
        }
        Ok(Self { token, admin_token })
    }

    /// No token at all: every caller is the local administrator.
    pub fn open() -> Self {
        Self { token: None, admin_token: None }
    }
}

#[async_trait]
impl Authenticator for LocalAuthenticator {
    fn mode(&self) -> &str {
        "local"
    }

    fn requires_credentials(&self) -> bool {
        non_empty(&self.token).is_some() || non_empty(&self.admin_token).is_some()
    }

    async fn authenticate(
        &self,
        bearer_token: Option<&str>,
        actor_hint: Option<&str>,
    ) -> Result<Principal> {
        let actor =
            |default: &'static str| actor_hint.filter(|hint| !hint.is_empty()).unwrap_or(default);
        if let (Some(admin), Some(bearer)) =
            (non_empty(&self.admin_token), bearer_token.filter(|bearer| !bearer.is_empty()))
            && tokens_equal(bearer, admin)
        {
            return Ok(Principal::local(actor("local-admin"), true));
        }
        if let Some(token) = &self.token {
            return match bearer_token {
                Some(bearer) if tokens_equal(bearer, token) => {
                    Ok(Principal::local(actor("local-user"), false))
                }
                _ => Err(LiminaError::authentication()),
            };
        }
        if self.admin_token.is_some() {
            return Err(LiminaError::authentication());
        }
        Ok(Principal::local(actor("local-admin"), true))
    }
}

/// Static automation tokens next to an identity provider, or `None` when neither is configured
/// (a chain built from it never grants anonymous access).
pub fn machine_token_authenticator(
    token: Option<&str>,
    admin_token: Option<&str>,
) -> std::result::Result<Option<LocalAuthenticator>, AuthConfigError> {
    let token = token.filter(|value| !value.is_empty()).map(str::to_owned);
    let admin_token = admin_token.filter(|value| !value.is_empty()).map(str::to_owned);
    if token.is_none() && admin_token.is_none() {
        return Ok(None);
    }
    LocalAuthenticator::new(token, admin_token).map(Some)
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}
