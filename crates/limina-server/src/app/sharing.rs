//! Sharing projects by email, on top of the collaboration service's memberships and pending
//! invitations ([`limina_domain::collaboration::sharing`]).
//!
//! Owners share a project by typing an email and a role. With a people directory (WorkOS User
//! Management, see [`crate::auth::directory`]) the person is looked up first: when they exist,
//! their email is verified and they belong to the required organization, they become a member
//! at once. Otherwise, and in modes without a directory (OIDC), the member already known by that
//! email gets the role, or a pending invitation waits for the first sign-in with that verified
//! email. The required organization is the runtime's pinned one (`LIMINA_WORKOS_ORGANIZATION_ID`)
//! or, without a pin, the sharer's session organization.
//!
//! Limina sends no email; the project appears in the person's project list.

use async_trait::async_trait;
use limina_core::{Principal, Result};
use limina_domain::collaboration::{DirectoryPerson, Role, Share, SharedProject, parse_email};

use super::operations::ProjectOperations;

/// Looks people up by email in the identity provider's directory.
#[async_trait]
pub trait PeopleDirectory: Send + Sync {
    /// The organization everyone must belong to, when the runtime is pinned to one.
    fn organization(&self) -> Option<&str>;

    /// The person with this (normalized) email, when the provider verified it and, with
    /// `organization`, they have an active membership in it. People outside the organization
    /// are not found, so their existence is never revealed.
    async fn find_by_email(
        &self,
        email: &str,
        organization: Option<&str>,
    ) -> Result<Option<DirectoryPerson>>;
}

impl ProjectOperations {
    /// Everyone with access to the project, members first, then pending invitations (viewers).
    pub async fn sharing(&self, slug: &str, principal: &Principal) -> Result<Vec<Share>> {
        self.collaboration().require_role(slug, principal, Role::Viewer).await?;
        self.collaboration().sharing(slug).await
    }

    /// Shares the project with the person who has `email` (owners); see the module docs.
    pub async fn share_project(
        &self,
        slug: &str,
        email: &str,
        role: &str,
        directory: Option<&dyn PeopleDirectory>,
        principal: &Principal,
    ) -> Result<Share> {
        let collaboration = self.collaboration();
        collaboration.require_role(slug, principal, Role::Owner).await?;
        let role = Role::parse(role)?;
        let email = parse_email(email)?;
        let organization = directory
            .and_then(|directory| directory.organization())
            .map(str::to_owned)
            .or_else(|| principal.organization_id.clone());
        let organization = organization.as_deref();
        let person = match directory {
            Some(directory) => directory.find_by_email(&email, organization).await?,
            None => None,
        };
        let shared = match person {
            Some(person) => collaboration.share_with_person(slug, &person, role, principal).await?,
            None => {
                collaboration.share_with_email(slug, &email, role, organization, principal).await?
            }
        };
        self.notifier().notify(slug);
        Ok(shared)
    }

    /// Withdraws a pending invitation (owners).
    pub async fn cancel_invitation(
        &self,
        slug: &str,
        email: &str,
        principal: &Principal,
    ) -> Result<Share> {
        let collaboration = self.collaboration();
        collaboration.require_role(slug, principal, Role::Owner).await?;
        let cancelled = collaboration.cancel_invitation(slug, email, &principal.actor()).await?;
        self.notifier().notify(slug);
        Ok(cancelled)
    }

    /// The projects other people shared with the caller, with their owners. Invitations waiting
    /// for the caller's verified email are accepted first.
    pub async fn shared_projects(&self, principal: &Principal) -> Result<Vec<SharedProject>> {
        self.collaboration().accept_invitations(principal).await?;
        self.collaboration().shared_projects(principal).await
    }
}
