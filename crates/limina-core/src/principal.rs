//! The authenticated caller of a command, and the rule for which projects it may act on.
//!
//! # Project access
//!
//! Projects are private to their members: a principal sees and opens a project only when it is a
//! member (`OWNER`, `EDITOR` or `VIEWER`), which it becomes by creating the project or when an
//! owner shares it. The single exception is a trusted single-operator principal
//! ([`Principal::acts_on_every_project`]): the CLI in embedded mode, the localhost open mode and
//! static API tokens, which own every project as before.
//!
//! [`Principal::instance_admin`] administers the runtime itself (shared engine credentials, the
//! runtime's Codex account). It grants no project access: in the multi-user sign-in modes (OIDC
//! and WorkOS) an administrator sees only the projects they are a member of, like everyone else.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub subject: String,
    pub display_name: String,
    pub email: Option<String>,
    /// Whether the identity provider verified `email`. Pending project invitations bind only to
    /// a verified email.
    #[serde(default)]
    pub email_verified: bool,
    /// The organization the sign-in session belongs to (WorkOS `org_id`), when it has one.
    #[serde(default)]
    pub organization_id: Option<String>,
    /// Allowed to administer the runtime (engines, the runtime's Codex account). Grants no
    /// project access by itself.
    pub instance_admin: bool,
    /// Trusted local operator: owns every project without a membership (see
    /// [`Self::acts_on_every_project`]).
    pub project_admin: bool,
    /// How the principal signed in: `local`, `token`, `oidc` or `workos`.
    pub auth_mode: String,
}

impl Principal {
    pub fn new(subject: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            display_name: display_name.into(),
            email: None,
            email_verified: false,
            organization_id: None,
            instance_admin: false,
            project_admin: false,
            auth_mode: "oidc".to_owned(),
        }
    }

    /// The name recorded on events and artifacts (at most 200 characters).
    pub fn actor(&self) -> String {
        self.display_name.chars().take(200).collect()
    }

    /// A trusted local operator (CLI in embedded mode, localhost without tokens, static tokens).
    pub fn local(actor: &str, instance_admin: bool) -> Self {
        let name = match actor.trim() {
            "" => "local-admin",
            name => name,
        };
        Self {
            subject: format!("local:{name}"),
            display_name: name.to_owned(),
            email: None,
            email_verified: false,
            organization_id: None,
            instance_admin,
            project_admin: true,
            auth_mode: "local".to_owned(),
        }
    }

    /// The project-access rule (see the module docs): only trusted single-operator principals act
    /// on every project; everyone else, instance administrators included, needs a membership.
    pub fn acts_on_every_project(&self) -> bool {
        self.project_admin
    }

    /// The email pending invitations may bind to: trimmed and lower-cased, and only when the
    /// identity provider verified it.
    pub fn verified_email(&self) -> Option<String> {
        if !self.email_verified {
            return None;
        }
        self.email.as_deref().map(normalize_email).filter(|email| !email.is_empty())
    }

    #[must_use]
    pub fn with_email(mut self, email: Option<String>) -> Self {
        self.email = email;
        self
    }

    /// Marks the email as verified (or not) by the identity provider.
    #[must_use]
    pub fn email_verified(mut self, verified: bool) -> Self {
        self.email_verified = verified;
        self
    }

    #[must_use]
    pub fn organization(mut self, organization_id: Option<String>) -> Self {
        self.organization_id = organization_id;
        self
    }

    #[must_use]
    pub fn admin(mut self, instance_admin: bool) -> Self {
        self.instance_admin = instance_admin;
        self
    }

    #[must_use]
    pub fn mode(mut self, auth_mode: &str) -> Self {
        auth_mode.clone_into(&mut self.auth_mode);
        self
    }
}

/// An email as Limina compares and stores it: trimmed and lower-cased.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_operators_act_on_every_project() {
        assert!(Principal::local("ada", false).acts_on_every_project());
        assert!(Principal::local("", true).acts_on_every_project());
        let admin = Principal::new("user_01", "Ada").admin(true).mode("workos");
        assert!(admin.instance_admin && !admin.acts_on_every_project());
        assert!(!Principal::new("user_02", "Grace").acts_on_every_project());
    }

    #[test]
    fn invitations_bind_only_to_verified_emails() {
        let person = Principal::new("user_01", "Ada").with_email(Some(" Ada@Example.COM ".into()));
        assert_eq!(person.verified_email(), None);
        assert_eq!(
            person.email_verified(true).verified_email().as_deref(),
            Some("ada@example.com")
        );
        assert_eq!(Principal::new("x", "X").email_verified(true).verified_email(), None);
    }
}
