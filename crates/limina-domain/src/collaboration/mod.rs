//! Team access and the read/write surfaces around the research core: project roles and members,
//! sharing by email ([`sharing`]), kickoff drafts, preflight, live attachment tickets, guidance
//! history, runtime runs and analytics. The knowledge surfaces (search, graph, relations,
//! comments, tags, saved views, sources) are in [`crate::knowledge`].
//!
//! Projects are private to their members; [`Principal::acts_on_every_project`] is the one rule
//! that decides who may act on a project without a membership, and every role check here goes
//! through [`CollaborationService::role_for`].

pub mod paging;
mod runs;
pub mod sharing;
pub mod validation;
mod views;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use limina_core::{LiminaError, Principal, Result, internal, now};
use limina_store::models::{Artifact, Challenge, CoordinatorState, LiveTicket, ProjectMember};
use limina_store::{Db, Tx, args};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub use paging::Page;
pub use sharing::{DirectoryPerson, ProjectOwnerView, Share, SharedProject, parse_email};
pub use views::*;

use crate::common::record_event;
use crate::engines::Engine;
use validation::{py_strip, required};

/// How long a live ticket stays valid when the caller does not say.
pub const DEFAULT_LIVE_TICKET_TTL_SECONDS: i64 = 60;

/// A project role. Roles are ordered: `Viewer < Editor < Owner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Role {
    Viewer,
    Editor,
    Owner,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "VIEWER",
            Self::Editor => "EDITOR",
            Self::Owner => "OWNER",
        }
    }

    pub const fn level(self) -> u8 {
        match self {
            Self::Viewer => 1,
            Self::Editor => 2,
            Self::Owner => 3,
        }
    }

    /// Parses a role as the API accepts it (any case).
    pub fn parse(value: &str) -> Result<Self> {
        Self::from_stored(&value.to_uppercase())
            .ok_or_else(|| LiminaError::invariant("Project role must be OWNER, EDITOR, or VIEWER."))
    }

    fn from_stored(value: &str) -> Option<Self> {
        match value {
            "VIEWER" => Some(Self::Viewer),
            "EDITOR" => Some(Self::Editor),
            "OWNER" => Some(Self::Owner),
            _ => None,
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A role read back from the database; only validated roles are ever written.
fn stored_role(value: &str) -> Result<Role> {
    Role::from_stored(value).ok_or_else(|| internal(format!("unknown project role '{value}'")))
}

/// Owns team access and the read/write surfaces around the research core.
#[derive(Debug, Clone)]
pub struct CollaborationService {
    pub(crate) db: Db,
}

impl CollaborationService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Makes the principal the project's first owner when it has no members yet.
    pub async fn add_creator(&self, slug: &str, principal: &Principal) -> Result<()> {
        let mut tx = self.db.begin_write().await?;
        let challenge = lock_project(&mut tx, slug).await?;
        let members: i64 = tx
            .scalar(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM project_members WHERE challenge_id = $1",
                args![&challenge.id],
            )
            .await?;
        if members == 0 {
            tx.execute(
                "INSERT INTO project_members (challenge_id, subject, role, display_name, email, \
                 created_by, created_at, updated_at) VALUES ($1, $2, 'OWNER', $3, $4, $5, $6, $6)",
                args![
                    &challenge.id,
                    &principal.subject,
                    &principal.display_name,
                    principal.email.as_deref(),
                    &principal.subject,
                    now()
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The principal's role on a project: its membership, or owner for a principal that acts on
    /// every project ([`Principal::acts_on_every_project`]). A non-member first accepts the
    /// invitations waiting for its verified email; without a membership it is refused.
    pub async fn role_for(&self, slug: &str, principal: &Principal) -> Result<Role> {
        if principal.acts_on_every_project() {
            return Ok(Role::Owner);
        }
        if let Some(role) = self.member_role(slug, principal).await? {
            return Ok(role);
        }
        if self.accept_invitations(principal).await? > 0
            && let Some(role) = self.member_role(slug, principal).await?
        {
            return Ok(role);
        }
        Err(LiminaError::authorization().with("project", slug))
    }

    /// The principal's membership role, `None` when it is not a member.
    async fn member_role(&self, slug: &str, principal: &Principal) -> Result<Option<Role>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let role: Option<String> = tx
            .scalar_optional(
                "SELECT role FROM project_members WHERE challenge_id = $1 AND subject = $2",
                args![&challenge.id, &principal.subject],
            )
            .await?;
        tx.commit().await?;
        role.as_deref().map(stored_role).transpose()
    }

    /// The principal's role, or a permission error when it is below `minimum`.
    pub async fn require_role(
        &self,
        slug: &str,
        principal: &Principal,
        minimum: Role,
    ) -> Result<Role> {
        let role = self.role_for(slug, principal).await?;
        if role.level() < minimum.level() {
            return Err(LiminaError::authorization_with(format!(
                "Project role {minimum} or higher is required."
            ))
            .with("project", slug)
            .with("required_role", minimum.as_str())
            .with("actual_role", role.as_str()));
        }
        Ok(role)
    }

    /// Each project the principal belongs to, with its role, after accepting the invitations
    /// waiting for its verified email. `None` means owner of every project
    /// ([`Principal::acts_on_every_project`]).
    pub async fn project_roles(
        &self,
        principal: &Principal,
    ) -> Result<Option<BTreeMap<String, Role>>> {
        if principal.acts_on_every_project() {
            return Ok(None);
        }
        self.accept_invitations(principal).await?;
        let mut tx = self.db.begin().await?;
        let rows: Vec<(String, String)> = tx
            .fetch_all(
                "SELECT challenges.slug, project_members.role FROM challenges \
                 JOIN project_members ON project_members.challenge_id = challenges.id \
                 WHERE project_members.subject = $1",
                args![&principal.subject],
            )
            .await?;
        tx.commit().await?;
        let roles = rows
            .into_iter()
            .map(|(slug, role)| Ok((slug, stored_role(&role)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Some(roles))
    }

    /// The slugs of the projects the principal can see. `None` means every project.
    pub async fn visible_project_slugs(
        &self,
        principal: &Principal,
    ) -> Result<Option<BTreeSet<String>>> {
        Ok(self.project_roles(principal).await?.map(|roles| roles.into_keys().collect()))
    }

    pub async fn list_members(&self, slug: &str) -> Result<Vec<Member>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let rows: Vec<ProjectMember> = tx
            .fetch_all(
                "SELECT * FROM project_members WHERE challenge_id = $1 \
                 ORDER BY role DESC, display_name",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        rows.into_iter().map(member).collect()
    }

    /// Adds a member or changes one, by subject. A project always keeps at least one owner. A
    /// pending invitation for the member's email is withdrawn.
    pub async fn set_member(
        &self,
        slug: &str,
        change: &MemberChange,
        actor: &str,
    ) -> Result<Member> {
        let role = Role::parse(&change.role)?;
        let subject = py_strip(&change.subject);
        let display_name = py_strip(&change.display_name);
        if subject.is_empty() || display_name.is_empty() {
            return Err(LiminaError::invariant("Member subject and display name are required."));
        }
        let email = change.email.as_deref().filter(|email| !email.is_empty()).map(py_strip);
        let mut tx = self.db.begin_write().await?;
        let challenge = lock_project(&mut tx, slug).await?;
        let existing: Option<ProjectMember> = tx
            .fetch_optional(
                "SELECT * FROM project_members WHERE challenge_id = $1 AND subject = $2",
                args![&challenge.id, subject],
            )
            .await?;
        let changed = now();
        let row: ProjectMember = match existing {
            None => {
                tx.fetch_one(
                    "INSERT INTO project_members (challenge_id, subject, role, display_name, \
                     email, created_by, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $7) RETURNING *",
                    args![
                        &challenge.id,
                        subject,
                        role.as_str(),
                        display_name,
                        email,
                        actor,
                        changed
                    ],
                )
                .await?
            }
            Some(existing) => {
                if existing.role == Role::Owner.as_str() && role != Role::Owner {
                    require_another_owner(&mut tx, &challenge.id).await?;
                }
                tx.fetch_one(
                    "UPDATE project_members SET role = $1, display_name = $2, email = $3, \
                     updated_at = $4 WHERE challenge_id = $5 AND subject = $6 RETURNING *",
                    args![role.as_str(), display_name, email, changed, &challenge.id, subject],
                )
                .await?
            }
        };
        // The member's role is now explicit; an invitation for the same email would otherwise
        // raise it again when it is accepted.
        if let Some(email) = email {
            tx.execute(
                "DELETE FROM project_invitations WHERE challenge_id = $1 AND email = $2",
                args![&challenge.id, limina_core::normalize_email(email)],
            )
            .await?;
        }
        record(
            &mut tx,
            &challenge.id,
            "project.member_set",
            actor,
            json!({"subject": &row.subject, "role": &row.role}),
            None,
        )
        .await?;
        tx.commit().await?;
        member(row)
    }

    /// Removes a member. The last owner cannot be removed.
    pub async fn remove_member(&self, slug: &str, subject: &str, actor: &str) -> Result<Member> {
        let mut tx = self.db.begin_write().await?;
        let challenge = lock_project(&mut tx, slug).await?;
        let existing: Option<ProjectMember> = tx
            .fetch_optional(
                "SELECT * FROM project_members WHERE challenge_id = $1 AND subject = $2",
                args![&challenge.id, subject],
            )
            .await?;
        let Some(existing) = existing else {
            return Err(
                LiminaError::not_found("Project member does not exist.").with("subject", subject)
            );
        };
        if existing.role == Role::Owner.as_str() {
            require_another_owner(&mut tx, &challenge.id).await?;
        }
        tx.execute(
            "DELETE FROM project_members WHERE challenge_id = $1 AND subject = $2",
            args![&challenge.id, subject],
        )
        .await?;
        record(
            &mut tx,
            &challenge.id,
            "project.member_removed",
            actor,
            json!({"subject": subject}),
            None,
        )
        .await?;
        tx.commit().await?;
        member(existing)
    }

    /// Changes draft kickoff fields or model defaults for future turns on a non-archived project.
    pub async fn update_draft(
        &self,
        slug: &str,
        changes: &DraftChanges,
        actor: &str,
    ) -> Result<DraftProject> {
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let coordinator: Option<CoordinatorState> = tx
            .fetch_optional(
                "SELECT * FROM coordinator_states WHERE challenge_id = $1",
                args![&challenge.id],
            )
            .await?;
        let kickoff_change = changes.name.is_some()
            || changes.mission.is_some()
            || changes.context.is_some()
            || changes.success_criteria.is_some()
            || changes.runtime.is_some();
        let Some(coordinator) =
            coordinator.filter(|state| !kickoff_change || state.status == "CREATED")
        else {
            return Err(LiminaError::invariant(
                "Project kickoff fields can only change before the first start.",
            ));
        };
        if challenge.status == "ARCHIVED" {
            return Err(LiminaError::invariant("Archived projects are read-only."));
        }
        let engine_changed =
            changes.runtime.as_deref().is_some_and(|engine| engine != challenge.runtime_engine);
        let name = match &changes.name {
            Some(name) => required("name", name)?,
            None => challenge.name,
        };
        let (objective, current_objective) = match &changes.mission {
            Some(mission) => {
                let objective = required("mission", mission)?;
                (objective.clone(), objective)
            }
            None => (challenge.objective, coordinator.current_objective),
        };
        let context = match &changes.context {
            Some(context) => py_strip(context).to_owned(),
            None => challenge.context,
        };
        let success_criteria = match &changes.success_criteria {
            Some(criteria) => required("success criteria", criteria)?,
            None => challenge.success_criteria,
        };
        let runtime_engine = match &changes.runtime {
            Some(runtime) => Engine::parse(runtime)
                .map_err(|error| LiminaError::invariant(error.message))?
                .id()
                .to_owned(),
            None => challenge.runtime_engine,
        };
        // Provider switches clear provider-specific defaults unless supplied together.
        let model = match &changes.model {
            Some(model) => crate::engines::model_choice(Some(model))?,
            None if engine_changed => None,
            None => challenge.model.clone(),
        };
        let model_changed = model != challenge.model;
        let engine = Engine::parse(&runtime_engine)?;
        let reasoning_effort = match &changes.reasoning_effort {
            Some(effort) => engine.reasoning_choice(Some(effort))?,
            None if engine_changed || model_changed => None,
            None => challenge.reasoning_effort,
        };
        let updated = now();
        let challenge: Challenge = tx
            .fetch_one(
                "UPDATE challenges SET name = $1, objective = $2, context = $3, \
                 success_criteria = $4, runtime_engine = $5, model = $6, reasoning_effort = $7, \
                 version = version + 1, updated_at = $8 WHERE id = $9 RETURNING *",
                args![
                    name,
                    objective,
                    context,
                    success_criteria,
                    runtime_engine,
                    model.as_deref(),
                    reasoning_effort.as_deref(),
                    updated,
                    &challenge.id
                ],
            )
            .await?;
        let coordinator: CoordinatorState = tx
            .fetch_one(
                "UPDATE coordinator_states SET current_objective = $1, version = version + 1, \
                 updated_at = $2 WHERE challenge_id = $3 RETURNING *",
                args![current_objective, updated, &challenge.id],
            )
            .await?;
        record(&mut tx, &challenge.id, "project.draft_updated", actor, json!({}), None).await?;
        tx.commit().await?;
        Ok(DraftProject::from_rows(challenge, coordinator))
    }

    /// Kickoff readiness. `engine_status` probes the project's engine; it runs after the database
    /// work, since probing credentials may start a provider process.
    pub async fn preflight<F, Fut>(&self, slug: &str, engine_status: F) -> Result<Preflight>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<EngineReadiness>>,
    {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let sources: i64 = tx
            .scalar(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM project_sources \
                 WHERE challenge_id = $1 AND status = 'ACTIVE'",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        let has_mission = !py_strip(&challenge.objective).is_empty();
        let has_success_criteria = !py_strip(&challenge.success_criteria).is_empty();
        let engine = engine_status(challenge.runtime_engine).await?;
        let engine_ready = engine.installed && engine.credentials;
        let check = |name: &str, status, detail: String| PreflightCheck {
            name: name.to_owned(),
            status,
            detail,
        };
        let checks = vec![
            if has_mission {
                check("mission", CheckStatus::Pass, "Mission is defined.".to_owned())
            } else {
                check("mission", CheckStatus::Fail, "Mission is empty.".to_owned())
            },
            if has_success_criteria {
                check(
                    "success_criteria",
                    CheckStatus::Pass,
                    "Success criteria are defined.".to_owned(),
                )
            } else {
                check(
                    "success_criteria",
                    CheckStatus::Fail,
                    "Success criteria are empty.".to_owned(),
                )
            },
            if engine_ready {
                check(
                    "runtime",
                    CheckStatus::Pass,
                    format!("{} is ready. {}", engine.label, engine.detail),
                )
            } else {
                check("runtime", CheckStatus::Warn, engine.detail)
            },
            check(
                "sources",
                if sources > 0 { CheckStatus::Pass } else { CheckStatus::Info },
                format!("{sources} source(s) registered."),
            ),
        ];
        let ready = !checks.iter().any(|item| item.status == CheckStatus::Fail);
        Ok(Preflight { ready, checks })
    }

    /// Issues a single-use ticket that lets a browser attach to the live channel with `role`.
    pub async fn issue_live_ticket(
        &self,
        slug: &str,
        principal: &Principal,
        role: Role,
    ) -> Result<LiveTicketGrant> {
        self.issue_live_ticket_for(slug, principal, role, DEFAULT_LIVE_TICKET_TTL_SECONDS).await
    }

    /// [`Self::issue_live_ticket`] with a lifetime, clamped to 15–120 seconds. Only the ticket's
    /// SHA-256 digest is stored.
    pub async fn issue_live_ticket_for(
        &self,
        slug: &str,
        principal: &Principal,
        role: Role,
        ttl_seconds: i64,
    ) -> Result<LiveTicketGrant> {
        let ttl_seconds = ttl_seconds.clamp(15, 120);
        let ticket = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
        let expires_at = now() + chrono::Duration::seconds(ttl_seconds);
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        tx.execute(
            "INSERT INTO live_tickets (token_hash, challenge_id, subject, display_name, role, \
             instance_admin, expires_at, used_at, created_at, project_admin) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, $8, $9)",
            args![
                sha256_hex(&ticket),
                &challenge.id,
                &principal.subject,
                &principal.display_name,
                role.as_str(),
                principal.instance_admin,
                expires_at,
                now(),
                principal.project_admin
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(LiveTicketGrant { ticket, expires_at })
    }

    /// Redeems a live ticket once, returning who it was issued to and with which role. The
    /// principal keeps the issuer's administration flags, so a trusted local operator still acts
    /// on the project through its ticket.
    pub async fn consume_live_ticket(&self, slug: &str, ticket: &str) -> Result<(Principal, Role)> {
        let token_hash = sha256_hex(ticket);
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let sql = format!(
            "SELECT * FROM live_tickets WHERE token_hash = $1 AND challenge_id = $2{}",
            tx.for_update()
        );
        let item: Option<LiveTicket> =
            tx.fetch_optional(&sql, args![&token_hash, &challenge.id]).await?;
        let Some(item) = item.filter(|item| item.used_at.is_none() && item.expires_at > now())
        else {
            return Err(LiminaError::authorization_with(
                "The live attachment ticket is invalid or expired.",
            ));
        };
        let role = stored_role(&item.role)?;
        tx.execute(
            "UPDATE live_tickets SET used_at = $1 WHERE token_hash = $2",
            args![now(), &token_hash],
        )
        .await?;
        tx.commit().await?;
        let principal = Principal {
            subject: item.subject,
            display_name: item.display_name,
            email: None,
            email_verified: false,
            organization_id: None,
            instance_admin: item.instance_admin,
            project_admin: item.project_admin,
            auth_mode: "live-ticket".to_owned(),
        };
        Ok((principal, role))
    }
}

fn member(row: ProjectMember) -> Result<Member> {
    let role = stored_role(&row.role)?;
    Ok(Member::from_row(row, role))
}

async fn require_another_owner(tx: &mut Tx, challenge_id: &str) -> Result<()> {
    let owners: i64 = tx
        .scalar(
            "SELECT CAST(COUNT(*) AS BIGINT) FROM project_members \
             WHERE challenge_id = $1 AND role = 'OWNER'",
            args![challenge_id],
        )
        .await?;
    if owners == 1 {
        return Err(LiminaError::invariant("A project must retain at least one owner."));
    }
    Ok(())
}

/// The project with this slug (case-insensitive), or `Project '<slug>' does not exist.`
pub(crate) async fn project(tx: &mut Tx, slug: &str) -> Result<Challenge> {
    find_project(tx, slug, false).await
}

/// [`project`], locking the row for the rest of the transaction on PostgreSQL.
pub(crate) async fn lock_project(tx: &mut Tx, slug: &str) -> Result<Challenge> {
    find_project(tx, slug, true).await
}

async fn find_project(tx: &mut Tx, slug: &str, lock: bool) -> Result<Challenge> {
    let sql = format!(
        "SELECT * FROM challenges WHERE slug = $1{}",
        if lock { tx.for_update() } else { "" }
    );
    let challenge: Option<Challenge> = tx.fetch_optional(&sql, args![slug.to_lowercase()]).await?;
    challenge.ok_or_else(|| {
        LiminaError::not_found(format!("Project '{slug}' does not exist.")).with("project", slug)
    })
}

/// A knowledge artifact by display ID (case-insensitive).
pub(crate) async fn knowledge_artifact(
    tx: &mut Tx,
    challenge_id: &str,
    artifact_id: &str,
) -> Result<Artifact> {
    let artifact: Option<Artifact> = tx
        .fetch_optional(
            "SELECT * FROM artifacts WHERE challenge_id = $1 AND artifact_id = $2",
            args![challenge_id, artifact_id.to_uppercase()],
        )
        .await?;
    artifact.ok_or_else(|| {
        LiminaError::not_found(format!("Knowledge artifact '{artifact_id}' does not exist."))
    })
}

/// Records an event that belongs to no client command (a random command ID, like the Python
/// runtime's `secrets.token_hex(16)`).
pub(crate) async fn record(
    tx: &mut Tx,
    challenge_id: &str,
    event_type: &str,
    actor: &str,
    payload: Value,
    artifact_id: Option<&str>,
) -> Result<()> {
    let command_id: String =
        random_bytes::<16>()?.iter().map(|byte| format!("{byte:02x}")).collect();
    record_event(tx, challenge_id, event_type, actor, &command_id, payload, artifact_id).await?;
    Ok(())
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(internal)?;
    Ok(bytes)
}

fn sha256_hex(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_order_and_parse_like_python() {
        assert!(Role::Viewer < Role::Editor && Role::Editor < Role::Owner);
        assert_eq!(Role::parse("owner").unwrap(), Role::Owner);
        assert_eq!(
            Role::parse("v\u{131}ewer").unwrap(),
            Role::Viewer,
            "Python upper() maps ı to I"
        );
        assert_eq!(
            Role::parse("admin").unwrap_err().message,
            "Project role must be OWNER, EDITOR, or VIEWER."
        );
        assert_eq!(serde_json::to_value(Role::Editor).unwrap(), "EDITOR");
    }

    #[test]
    fn tickets_hash_with_sha256() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(URL_SAFE_NO_PAD.encode(random_bytes::<32>().unwrap()).len(), 43);
    }
}
