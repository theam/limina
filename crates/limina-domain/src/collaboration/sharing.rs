//! Sharing a project by email.
//!
//! A project is visible only to its members (see [`limina_core::principal`] for the rule and its
//! one exception). Owners share it by email with someone in their organization:
//!
//! - When the identity provider's directory knows the person, they become a member at once,
//!   keyed by their subject ([`CollaborationService::share_with_person`]).
//! - Otherwise ([`CollaborationService::share_with_email`]) the member already known by that
//!   email gets the role, or a pending invitation (`project_invitations`) is stored for the
//!   email, optionally restricted to an organization.
//! - The first time someone authenticates with that email *verified* (and, when the invitation
//!   names one, in that organization), the invitation becomes their membership
//!   ([`CollaborationService::accept_invitations`]). Unverified emails never bind.
//!
//! Limina sends no email: the project simply appears in the person's project list. Pending
//! invitations never count as owners, so a project always keeps an active owner.

use std::collections::BTreeMap;

use limina_core::time::serde_iso;
use limina_core::{LiminaError, Principal, Result, Timestamp, normalize_email, now};
use limina_store::models::{ProjectInvitation, ProjectMember};
use limina_store::{Tx, args};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{CollaborationService, Role, lock_project, project, record, require_another_owner};
use super::{stored_role, validation::py_strip};

/// The longest email the API stores (`VARCHAR(320)`).
pub const EMAIL_LIMIT: usize = 320;
/// The longest display name a membership stores (`VARCHAR(240)`).
const DISPLAY_NAME_LIMIT: usize = 240;

/// Someone the identity provider's directory found by email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPerson {
    /// The identity provider's user ID: the subject their sessions carry.
    pub subject: String,
    pub display_name: String,
    pub email: String,
}

/// One person's access to a project: a member, or a pending invitation for an email.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    /// `None` while the invitation is pending.
    pub subject: Option<String>,
    /// The member's name; the email while pending.
    pub display_name: String,
    pub email: Option<String>,
    pub role: Role,
    pub pending: bool,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl Share {
    fn member(row: ProjectMember) -> Result<Self> {
        Ok(Self {
            role: stored_role(&row.role)?,
            subject: Some(row.subject),
            display_name: row.display_name,
            email: row.email,
            pending: false,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    fn invitation(row: ProjectInvitation) -> Result<Self> {
        Ok(Self {
            role: stored_role(&row.role)?,
            subject: None,
            display_name: row.email.clone(),
            email: Some(row.email),
            pending: true,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// An owner of a project, as the people it is shared with see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectOwnerView {
    pub subject: String,
    pub display_name: String,
    pub email: Option<String>,
}

/// A project someone else shared with the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedProject {
    pub slug: String,
    pub role: Role,
    pub owners: Vec<ProjectOwnerView>,
    /// When the caller became a member.
    #[serde(with = "serde_iso")]
    pub shared_at: Timestamp,
}

/// A shareable email: trimmed, lower-cased and shaped like `local@domain.tld`.
pub fn parse_email(value: &str) -> Result<String> {
    let email = normalize_email(value);
    let invalid = || LiminaError::invariant("Enter a valid email address.").with("email", value);
    if email.chars().count() > EMAIL_LIMIT
        || email.chars().any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(invalid());
    }
    let Some((local, domain)) = email.split_once('@') else { return Err(invalid()) };
    let domain_ok = domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && !domain.contains('@');
    if local.is_empty() || !domain_ok {
        return Err(invalid());
    }
    Ok(email)
}

impl CollaborationService {
    /// Everyone with access: members first (owners, editors, viewers, then by name), then the
    /// pending invitations.
    pub async fn sharing(&self, slug: &str) -> Result<Vec<Share>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let members: Vec<ProjectMember> = tx
            .fetch_all(
                "SELECT * FROM project_members WHERE challenge_id = $1",
                args![&challenge.id],
            )
            .await?;
        let invitations: Vec<ProjectInvitation> = tx
            .fetch_all(
                "SELECT * FROM project_invitations WHERE challenge_id = $1",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        let mut active = members.into_iter().map(Share::member).collect::<Result<Vec<_>>>()?;
        let mut pending =
            invitations.into_iter().map(Share::invitation).collect::<Result<Vec<_>>>()?;
        let order =
            |share: &Share| (std::cmp::Reverse(share.role), share.display_name.to_lowercase());
        active.sort_by_key(order);
        pending.sort_by_key(order);
        active.extend(pending);
        Ok(active)
    }

    /// Gives a person the directory found `role`: they become a member at once (or their role
    /// changes), and a pending invitation for their email is consumed. A project always keeps
    /// an owner.
    pub async fn share_with_person(
        &self,
        slug: &str,
        person: &DirectoryPerson,
        role: Role,
        sharer: &Principal,
    ) -> Result<Share> {
        let subject = py_strip(&person.subject);
        if subject.is_empty() {
            return Err(LiminaError::invariant("Member subject and display name are required."));
        }
        let email = parse_email(&person.email)?;
        let name = match py_strip(&person.display_name) {
            "" => email.as_str(),
            name => name,
        };
        let mut tx = self.db.begin_write().await?;
        let challenge = lock_project(&mut tx, slug).await?;
        let change = MemberUpsert {
            challenge_id: &challenge.id,
            subject,
            display_name: name,
            email: Some(&email),
            role,
            created_by: &sharer.subject,
            keep_higher_role: false,
        };
        let row = upsert_member(&mut tx, &change).await?;
        delete_invitation(&mut tx, &challenge.id, &email).await?;
        record(
            &mut tx,
            &challenge.id,
            "project.member_set",
            &sharer.actor(),
            json!({"subject": &row.subject, "role": &row.role}),
            None,
        )
        .await?;
        tx.commit().await?;
        Share::member(row)
    }

    /// Shares with an email nobody in the directory matches (or with no directory at all): the
    /// one member already known by that email gets `role`; otherwise a pending invitation is
    /// stored (or its role changed). It binds when someone authenticates with that email
    /// verified and, when `organization` is given, in that organization.
    pub async fn share_with_email(
        &self,
        slug: &str,
        email: &str,
        role: Role,
        organization: Option<&str>,
        sharer: &Principal,
    ) -> Result<Share> {
        let email = parse_email(email)?;
        let organization = organization.map(str::trim).filter(|value| !value.is_empty());
        let mut tx = self.db.begin_write().await?;
        let challenge = lock_project(&mut tx, slug).await?;
        let members: Vec<ProjectMember> = tx
            .fetch_all(
                "SELECT * FROM project_members WHERE challenge_id = $1",
                args![&challenge.id],
            )
            .await?;
        let known: Vec<ProjectMember> = members
            .into_iter()
            .filter(|member| member.email.as_deref().map(normalize_email).as_ref() == Some(&email))
            .collect();
        if known.len() > 1 {
            return Err(LiminaError::invariant(
                "Several members use this email address; change their roles one by one.",
            )
            .with("email", email));
        }
        if let Some(member) = known.into_iter().next() {
            let change = MemberUpsert {
                challenge_id: &challenge.id,
                subject: &member.subject,
                display_name: &member.display_name,
                email: member.email.as_deref(),
                role,
                created_by: &member.created_by,
                keep_higher_role: false,
            };
            let row = upsert_member(&mut tx, &change).await?;
            delete_invitation(&mut tx, &challenge.id, &email).await?;
            record(
                &mut tx,
                &challenge.id,
                "project.member_set",
                &sharer.actor(),
                json!({"subject": &row.subject, "role": &row.role}),
                None,
            )
            .await?;
            tx.commit().await?;
            return Share::member(row);
        }
        let changed = now();
        let existing: Option<ProjectInvitation> = tx
            .fetch_optional(
                "SELECT * FROM project_invitations WHERE challenge_id = $1 AND email = $2",
                args![&challenge.id, &email],
            )
            .await?;
        let row: ProjectInvitation = match existing {
            None => {
                tx.fetch_one(
                    "INSERT INTO project_invitations (challenge_id, email, role, organization_id, \
                     created_by, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6) \
                     RETURNING *",
                    args![
                        &challenge.id,
                        &email,
                        role.as_str(),
                        organization,
                        &sharer.subject,
                        changed
                    ],
                )
                .await?
            }
            Some(_) => {
                tx.fetch_one(
                    "UPDATE project_invitations SET role = $1, organization_id = $2, \
                     updated_at = $3 WHERE challenge_id = $4 AND email = $5 RETURNING *",
                    args![role.as_str(), organization, changed, &challenge.id, &email],
                )
                .await?
            }
        };
        record(
            &mut tx,
            &challenge.id,
            "project.invitation_set",
            &sharer.actor(),
            json!({"email": &row.email, "role": &row.role}),
            None,
        )
        .await?;
        tx.commit().await?;
        Share::invitation(row)
    }

    /// Withdraws the pending invitation for `email`.
    pub async fn cancel_invitation(&self, slug: &str, email: &str, actor: &str) -> Result<Share> {
        let email = normalize_email(email);
        let mut tx = self.db.begin_write().await?;
        let challenge = lock_project(&mut tx, slug).await?;
        let existing: Option<ProjectInvitation> = tx
            .fetch_optional(
                "SELECT * FROM project_invitations WHERE challenge_id = $1 AND email = $2",
                args![&challenge.id, &email],
            )
            .await?;
        let Some(existing) = existing else {
            return Err(LiminaError::not_found("No pending invitation exists for this email.")
                .with("email", email));
        };
        delete_invitation(&mut tx, &challenge.id, &email).await?;
        record(
            &mut tx,
            &challenge.id,
            "project.invitation_removed",
            actor,
            json!({"email": &email}),
            None,
        )
        .await?;
        tx.commit().await?;
        Share::invitation(existing)
    }

    /// Binds the pending invitations for the principal's verified email (in its organization,
    /// when an invitation names one): each becomes a membership keyed by the principal's subject.
    /// An existing membership keeps the higher of its role and the invited one. Returns how many
    /// invitations were accepted; a principal without a verified email accepts none.
    pub async fn accept_invitations(&self, principal: &Principal) -> Result<usize> {
        let Some(email) = principal.verified_email() else { return Ok(0) };
        let mut tx = self.db.begin().await?;
        let waiting: i64 = tx
            .scalar(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM project_invitations WHERE email = $1",
                args![&email],
            )
            .await?;
        tx.commit().await?;
        if waiting == 0 {
            return Ok(0);
        }
        let mut tx = self.db.begin_write().await?;
        let sql = format!(
            "SELECT * FROM project_invitations WHERE email = $1 ORDER BY created_at{}",
            tx.for_update()
        );
        let invitations: Vec<ProjectInvitation> = tx.fetch_all(&sql, args![&email]).await?;
        let display_name: String =
            principal.display_name.chars().take(DISPLAY_NAME_LIMIT).collect();
        let member_email = principal.email.as_deref().map(str::trim);
        let mut accepted = 0;
        for invitation in invitations {
            if let Some(required) = &invitation.organization_id
                && principal.organization_id.as_ref() != Some(required)
            {
                continue;
            }
            let change = MemberUpsert {
                challenge_id: &invitation.challenge_id,
                subject: &principal.subject,
                display_name: &display_name,
                email: member_email,
                role: stored_role(&invitation.role)?,
                created_by: &invitation.created_by,
                keep_higher_role: true,
            };
            let row = upsert_member(&mut tx, &change).await?;
            delete_invitation(&mut tx, &invitation.challenge_id, &email).await?;
            record(
                &mut tx,
                &invitation.challenge_id,
                "project.invitation_accepted",
                &principal.actor(),
                json!({"email": &email, "subject": &row.subject, "role": &row.role}),
                None,
            )
            .await?;
            accepted += 1;
        }
        tx.commit().await?;
        Ok(accepted)
    }

    /// The projects someone else shared with the principal (memberships another person granted),
    /// oldest project first, with their owners.
    pub async fn shared_projects(&self, principal: &Principal) -> Result<Vec<SharedProject>> {
        type Row = (String, String, Timestamp, Option<String>, Option<String>, Option<String>);
        let mut tx = self.db.begin().await?;
        let rows: Vec<Row> = tx
            .fetch_all(
                "SELECT challenges.slug, mine.role, mine.created_at, owners.subject, \
                 owners.display_name, owners.email FROM project_members mine \
                 JOIN challenges ON challenges.id = mine.challenge_id \
                 LEFT JOIN project_members owners ON owners.challenge_id = mine.challenge_id \
                 AND owners.role = 'OWNER' \
                 WHERE mine.subject = $1 AND mine.created_by <> $1 \
                 ORDER BY challenges.created_at, challenges.slug, owners.created_at",
                args![&principal.subject],
            )
            .await?;
        tx.commit().await?;
        let mut projects: Vec<SharedProject> = Vec::new();
        let mut positions: BTreeMap<String, usize> = BTreeMap::new();
        for (slug, role, shared_at, owner, owner_name, owner_email) in rows {
            let position = match positions.get(&slug) {
                Some(position) => *position,
                None => {
                    positions.insert(slug.clone(), projects.len());
                    projects.push(SharedProject {
                        slug,
                        role: stored_role(&role)?,
                        owners: Vec::new(),
                        shared_at,
                    });
                    projects.len() - 1
                }
            };
            if let (Some(subject), Some(display_name)) = (owner, owner_name) {
                projects[position].owners.push(ProjectOwnerView {
                    subject,
                    display_name,
                    email: owner_email,
                });
            }
        }
        Ok(projects)
    }
}

/// A membership to add or change.
struct MemberUpsert<'a> {
    challenge_id: &'a str,
    subject: &'a str,
    display_name: &'a str,
    email: Option<&'a str>,
    role: Role,
    /// Recorded when the membership is new.
    created_by: &'a str,
    /// Keep an existing membership's role when it is higher than `role`.
    keep_higher_role: bool,
}

/// Adds the member or changes their role, name and email. Demoting an owner needs another owner.
async fn upsert_member(tx: &mut Tx, change: &MemberUpsert<'_>) -> Result<ProjectMember> {
    let existing: Option<ProjectMember> = tx
        .fetch_optional(
            "SELECT * FROM project_members WHERE challenge_id = $1 AND subject = $2",
            args![change.challenge_id, change.subject],
        )
        .await?;
    let changed = now();
    let Some(existing) = existing else {
        return Ok(tx
            .fetch_one(
                "INSERT INTO project_members (challenge_id, subject, role, display_name, email, \
                 created_by, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
                 RETURNING *",
                args![
                    change.challenge_id,
                    change.subject,
                    change.role.as_str(),
                    change.display_name,
                    change.email,
                    change.created_by,
                    changed
                ],
            )
            .await?);
    };
    let current = stored_role(&existing.role)?;
    let role = if change.keep_higher_role { current.max(change.role) } else { change.role };
    if current == Role::Owner && role != Role::Owner {
        require_another_owner(tx, change.challenge_id).await?;
    }
    Ok(tx
        .fetch_one(
            "UPDATE project_members SET role = $1, display_name = $2, email = $3, \
             updated_at = $4 WHERE challenge_id = $5 AND subject = $6 RETURNING *",
            args![
                role.as_str(),
                change.display_name,
                change.email,
                changed,
                change.challenge_id,
                change.subject
            ],
        )
        .await?)
}

async fn delete_invitation(tx: &mut Tx, challenge_id: &str, email: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM project_invitations WHERE challenge_id = $1 AND email = $2",
        args![challenge_id, email],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_normalized_and_checked() {
        assert_eq!(parse_email("  Ana.Perez@Example.COM ").unwrap(), "ana.perez@example.com");
        assert_eq!(parse_email("a+tag@sub.example.test").unwrap(), "a+tag@sub.example.test");
        for bad in [
            "",
            "ana",
            "@example.com",
            "ana@",
            "ana@localhost",
            "a b@example.com",
            "ana@example..com",
            "ana@.example.com",
            "ana@example.com.",
            "a@b@example.com",
        ] {
            let error = parse_email(bad).unwrap_err();
            assert_eq!(error.message, "Enter a valid email address.", "{bad}");
        }
        assert!(parse_email(&format!("{}@example.com", "a".repeat(320))).is_err());
    }
}
