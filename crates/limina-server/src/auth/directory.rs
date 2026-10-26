//! The WorkOS User Management directory, used to share projects by email.
//!
//! Two read-only calls with the secret API key (`Authorization: Bearer <WORKOS_API_KEY>`):
//!
//! 1. `GET /user_management/users?email=<email>&limit=10` lists the users with that email. Only
//!    a user whose `email` matches and whose `email_verified` is `true` counts.
//! 2. With a required organization, `GET /user_management/organization_memberships?user_id=<id>
//!    &organization_id=<org>` must return an `active` membership (WorkOS lists only active
//!    memberships by default; the status is checked as well).
//!
//! The person found is shared with by their WorkOS user ID, the subject their sessions carry.
//! Nobody found is not an error: the caller stores a pending invitation instead.

use async_trait::async_trait;
use limina_core::{LiminaError, Result, normalize_email};
use limina_domain::collaboration::DirectoryPerson;
use serde_json::Value;

use super::jwks::{py_str, truthy};
use super::workos::WorkOsSettings;
use crate::app::PeopleDirectory;

/// How many users with one email are considered (WorkOS allows 1–100 per page).
const USER_PAGE: &str = "10";

/// The User Management directory of one WorkOS environment.
pub struct WorkOsDirectory {
    api_base_url: String,
    api_key: String,
    organization_id: Option<String>,
    http: reqwest::Client,
}

impl std::fmt::Debug for WorkOsDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkOsDirectory")
            .field("api_base_url", &self.api_base_url)
            .field("organization_id", &self.organization_id)
            .finish_non_exhaustive()
    }
}

impl WorkOsDirectory {
    /// The directory of the environment `settings` signs people in with; the pinned
    /// organization (`LIMINA_WORKOS_ORGANIZATION_ID`) applies to every lookup.
    pub fn new(settings: &WorkOsSettings, http: reqwest::Client) -> Self {
        Self {
            api_base_url: settings.api_base_url.clone(),
            api_key: settings.api_key.clone(),
            organization_id: settings.organization_id.clone(),
            http,
        }
    }

    /// The `data` items of a list endpoint.
    async fn list(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let url = url::Url::parse_with_params(&format!("{}{path}", self.api_base_url), query)
            .map_err(limina_core::internal)?;
        let response =
            self.http.get(url).bearer_auth(&self.api_key).send().await.map_err(|error| {
                tracing::warn!(%error, "the WorkOS directory could not be reached");
                LiminaError::transport("The WorkOS directory could not be reached.")
            })?;
        let status = response.status();
        if !status.is_success() {
            tracing::warn!(%status, path, "the WorkOS directory refused a lookup");
            return Err(LiminaError::transport("The WorkOS directory refused the lookup.")
                .with("status", status.as_u16()));
        }
        let payload: Value = response.json().await.map_err(|_| {
            LiminaError::transport("The WorkOS directory returned an unreadable answer.")
        })?;
        Ok(payload.get("data").and_then(Value::as_array).cloned().unwrap_or_default())
    }

    async fn is_active_member(&self, user_id: &str, organization: &str) -> Result<bool> {
        let memberships = self
            .list(
                "/user_management/organization_memberships",
                &[("user_id", user_id), ("organization_id", organization)],
            )
            .await?;
        Ok(memberships.iter().any(|membership| {
            membership.get("organization_id").and_then(Value::as_str) == Some(organization)
                && membership.get("status").and_then(Value::as_str).is_none_or(|s| s == "active")
        }))
    }
}

#[async_trait]
impl PeopleDirectory for WorkOsDirectory {
    fn organization(&self) -> Option<&str> {
        self.organization_id.as_deref()
    }

    async fn find_by_email(
        &self,
        email: &str,
        organization: Option<&str>,
    ) -> Result<Option<DirectoryPerson>> {
        let email = normalize_email(email);
        let users =
            self.list("/user_management/users", &[("email", &email), ("limit", USER_PAGE)]).await?;
        for user in users {
            let id = user.get("id").and_then(Value::as_str).unwrap_or_default().trim();
            let user_email = user.get("email").and_then(Value::as_str).map(normalize_email);
            let verified = user.get("email_verified") == Some(&Value::Bool(true));
            if id.is_empty() || user_email.as_deref() != Some(email.as_str()) || !verified {
                continue;
            }
            if let Some(organization) = organization
                && !self.is_active_member(id, organization).await?
            {
                continue;
            }
            return Ok(Some(DirectoryPerson {
                subject: id.to_owned(),
                display_name: display_name(&user).unwrap_or_else(|| email.clone()),
                email,
            }));
        }
        Ok(None)
    }
}

/// `first_name last_name`, or WorkOS's `name`, as sign-in names people.
fn display_name(user: &Value) -> Option<String> {
    let parts: Vec<String> = ["first_name", "last_name"]
        .into_iter()
        .filter_map(|field| user.get(field).filter(|value| truthy(Some(value))))
        .map(|value| py_str(value).trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect();
    let name = if parts.is_empty() {
        user.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_owned()
    } else {
        parts.join(" ")
    };
    Some(name.chars().take(200).collect::<String>()).filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_come_from_the_profile() {
        assert_eq!(
            display_name(&json!({"first_name": "Ada", "last_name": "Lovelace"})).as_deref(),
            Some("Ada Lovelace")
        );
        assert_eq!(
            display_name(&json!({"first_name": "Ada", "last_name": null})).as_deref(),
            Some("Ada")
        );
        assert_eq!(
            display_name(&json!({"name": " Grace Hopper "})).as_deref(),
            Some("Grace Hopper")
        );
        assert_eq!(display_name(&json!({"first_name": ""})), None);
    }
}
