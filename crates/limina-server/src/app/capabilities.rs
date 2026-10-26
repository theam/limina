//! Capability tokens for the internal agent API.
//!
//! Each managed turn gets a fresh random token (`LIMINA_INTERNAL_TOKEN` in the provider's
//! environment) bound to its project and to the runtime's actor. The token lives in memory for
//! the duration of the turn only, so a restarted runtime invalidates every token it issued.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use limina_core::{LiminaError, Result, internal};

/// The longest agent lane accepted in `X-Limina-Agent-Lane`.
pub const LANE_LIMIT: usize = 64;

const INVALID_CAPABILITY: &str = "The internal project capability is invalid or expired.";
const INVALID_LANE: &str = "The internal agent lane is invalid.";
const MISSING_CAPABILITY: &str = "An internal project capability is required.";

/// Issued tokens: token → (project slug, actor).
#[derive(Debug, Default)]
pub(crate) struct Capabilities {
    tokens: Mutex<HashMap<String, (String, String)>>,
}

impl Capabilities {
    fn tokens(&self) -> MutexGuard<'_, HashMap<String, (String, String)>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new token for `slug`, acting as `actor`. Like Python's `secrets.token_urlsafe(32)`.
    pub(crate) fn issue(&self, slug: &str, actor: &str) -> Result<String> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(internal)?;
        let token = URL_SAFE_NO_PAD.encode(bytes);
        self.tokens().insert(token.clone(), (slug.to_owned(), actor.to_owned()));
        Ok(token)
    }

    pub(crate) fn revoke(&self, token: &str) {
        self.tokens().remove(token);
    }

    pub(crate) fn len(&self) -> usize {
        self.tokens().len()
    }

    /// The actor a token acts as on `slug`, with the agent lane appended (`<actor>:<lane>`).
    pub(crate) fn actor(&self, token: &str, slug: &str, lane: Option<&str>) -> Result<String> {
        let actor = match self.tokens().get(token) {
            Some((owner, actor)) if owner == slug => actor.clone(),
            _ => return Err(LiminaError::authentication_with(INVALID_CAPABILITY)),
        };
        match lane.filter(|lane| !lane.is_empty()) {
            None => Ok(actor),
            Some(lane) if valid_lane(lane) => Ok(format!("{actor}:{lane}")),
            Some(_) => Err(LiminaError::authentication_with(INVALID_LANE)),
        }
    }

    /// [`Self::actor`] from an `Authorization` header value (`Bearer <token>`).
    pub(crate) fn authorize(
        &self,
        authorization: Option<&str>,
        slug: &str,
        lane: Option<&str>,
    ) -> Result<String> {
        let Some(token) = authorization.and_then(|value| value.strip_prefix("Bearer ")) else {
            return Err(LiminaError::authentication_with(MISSING_CAPABILITY));
        };
        self.actor(token, slug, lane)
    }
}

/// Python: letters, digits (`str.isalnum`), `-` and `_`, at most 64 characters.
fn valid_lane(lane: &str) -> bool {
    lane.chars().count() <= LANE_LIMIT
        && lane.chars().all(|character| character.is_alphanumeric() || "-_".contains(character))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_project_scoped_and_lanes_validated() {
        let capabilities = Capabilities::default();
        let token = capabilities.issue("alpha", "limina:1:abc").unwrap();
        assert_eq!(token.len(), 43);
        assert_eq!(capabilities.actor(&token, "alpha", None).unwrap(), "limina:1:abc");
        assert_eq!(capabilities.actor(&token, "alpha", Some("")).unwrap(), "limina:1:abc");
        assert_eq!(
            capabilities.actor(&token, "alpha", Some("lane_2-b")).unwrap(),
            "limina:1:abc:lane_2-b"
        );
        let wrong_project = capabilities.actor(&token, "beta", None).unwrap_err();
        assert_eq!(wrong_project.message, INVALID_CAPABILITY);
        assert_eq!(wrong_project.http_status(), 401);
        for lane in ["has space", "a:b", &"x".repeat(65)] {
            let error = capabilities.actor(&token, "alpha", Some(lane)).unwrap_err();
            assert_eq!(error.message, INVALID_LANE, "{lane}");
        }
        assert!(capabilities.actor(&token, "alpha", Some(&"x".repeat(64))).is_ok());
        assert_eq!(
            capabilities.authorize(Some(&format!("Bearer {token}")), "alpha", None).unwrap(),
            "limina:1:abc"
        );
        assert_eq!(
            capabilities.authorize(Some(&token), "alpha", None).unwrap_err().message,
            MISSING_CAPABILITY
        );
        capabilities.revoke(&token);
        assert_eq!(capabilities.len(), 0);
        assert!(capabilities.actor(&token, "alpha", None).is_err());
    }
}
