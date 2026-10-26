//! Project kickoff, lifecycle and resource commands of [`ChallengeService`].

use std::collections::{BTreeMap, HashMap};

use limina_core::{LiminaError, Result, new_uuid, now};
use limina_store::models::{Challenge, CoordinatorState, ProjectResource};
use limina_store::{Tx, args};
use serde_json::json;

use crate::command;
use crate::common::{find_challenge, record_event};
use crate::engines::Engine;
use crate::service::internal::{require_text, resource_name, resource_value, strip};
use crate::service::{ChallengeService, ChallengeView, ResourceView};

/// The coordinator's first next step on a new project.
pub const FIRST_NEXT_STEP: &str = "Frame the first falsifiable hypothesis.";

/// A new project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewChallenge {
    pub slug: String,
    pub name: String,
    pub objective: String,
    pub success_criteria: String,
    pub context: String,
    /// `codex` (the default) or `claude-code`.
    pub runtime_engine: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    /// Becomes the project's first `OWNER` member.
    pub owner: Option<ProjectOwner>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectOwner {
    pub subject: String,
    /// Defaults to the subject.
    pub display_name: Option<String>,
    pub email: Option<String>,
}

/// A lifecycle action on a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectAction {
    Start,
    Pause,
    Resume,
    Stop,
    Archive,
}

impl ProjectAction {
    /// Parses an action name (case-insensitive).
    pub fn parse(action: &str) -> Result<Self> {
        let action = action.to_lowercase();
        match action.as_str() {
            "start" => Ok(Self::Start),
            "pause" => Ok(Self::Pause),
            "resume" => Ok(Self::Resume),
            "stop" => Ok(Self::Stop),
            "archive" => Ok(Self::Archive),
            _ => Err(LiminaError::invariant(
                "Project action must be start, pause, resume, stop, or archive.",
            )
            .with("action", action)),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::Archive => "archive",
        }
    }

    const fn command_type(self) -> &'static str {
        match self {
            Self::Start => "project.start",
            Self::Pause => "project.pause",
            Self::Resume => "project.resume",
            Self::Stop => "project.stop",
            Self::Archive => "project.archive",
        }
    }

    const fn event_type(self) -> &'static str {
        match self {
            Self::Start => "project.started",
            Self::Pause => "project.paused",
            Self::Resume => "project.resumed",
            Self::Stop => "project.stopped",
            Self::Archive => "project.archived",
        }
    }

    /// The coordinator status this action moves `status` to, if the transition is allowed.
    /// Archiving is not a coordinator transition.
    pub fn transition(self, status: &str) -> Option<&'static str> {
        match (self, status) {
            (Self::Start, "CREATED" | "STOPPED")
            | (Self::Resume, "PAUSED" | "WAITING" | "STOPPED" | "FAILED") => Some("RUNNING"),
            (Self::Pause, "RUNNING" | "WAITING") => Some("PAUSED"),
            (Self::Stop, "CREATED" | "RUNNING" | "WAITING" | "PAUSED" | "FAILED") => {
                Some("STOPPED")
            }
            _ => None,
        }
    }

    /// True when the project is already where this action would take it.
    fn already_applied(self, status: &str) -> bool {
        matches!(
            (self, status),
            (Self::Start | Self::Resume, "RUNNING")
                | (Self::Pause, "PAUSED")
                | (Self::Stop, "STOPPED")
        )
    }
}

impl ChallengeService {
    pub async fn create_challenge(
        &self,
        new: NewChallenge,
        actor: &str,
        command_id: &str,
    ) -> Result<ChallengeView> {
        let slug = strip(&new.slug).to_lowercase();
        if !is_slug(&slug) {
            return Err(LiminaError::invariant(
                "Challenge slug must contain lowercase letters, numbers, and single hyphens.",
            )
            .with("slug", slug.as_str()));
        }
        require_text("name", &new.name)?;
        require_text("objective", &new.objective)?;
        require_text("success criteria", &new.success_criteria)?;
        let requested = new.runtime_engine.as_deref().unwrap_or("codex");
        let engine = Engine::parse(&strip(requested).to_lowercase()).map_err(|error| {
            LiminaError::invariant(error.message).with("runtime_engine", requested)
        })?;
        let model = crate::engines::model_choice(new.model.as_deref())?;
        let reasoning_effort = engine.reasoning_choice(new.reasoning_effort.as_deref())?;
        command!(&self.db, command_id, "challenge.create", actor, |tx| {
            let existing: Option<(String,)> = tx
                .fetch_optional("SELECT id FROM challenges WHERE slug = $1", args![slug.as_str()])
                .await?;
            if existing.is_some() {
                return Err(LiminaError::conflict(format!("Challenge '{slug}' already exists."))
                    .with("challenge", slug.as_str()));
            }
            let created_at = now();
            let challenge: Challenge = tx
                .fetch_one(
                    "INSERT INTO challenges (id, slug, name, objective, context, success_criteria, \
                     runtime_engine, model, reasoning_effort, status, version, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'ACTIVE', 1, $10, $10) RETURNING *",
                    args![
                        new_uuid(),
                        slug.as_str(),
                        strip(&new.name),
                        strip(&new.objective),
                        strip(&new.context),
                        strip(&new.success_criteria),
                        engine.id(),
                        model.as_deref(),
                        reasoning_effort.as_deref(),
                        created_at
                    ],
                )
                .await?;
            let coordinator: CoordinatorState = tx
                .fetch_one(
                    "INSERT INTO coordinator_states (challenge_id, status, current_objective, \
                     next_step, blocker, worker_id, continuation_id, inbox_cursor, version, \
                     heartbeat_at, wake_at, updated_at) \
                     VALUES ($1, 'CREATED', $2, $3, 'None', NULL, NULL, 0, 1, NULL, NULL, $4) \
                     RETURNING *",
                    args![&challenge.id, &challenge.objective, FIRST_NEXT_STEP, now()],
                )
                .await?;
            if let Some(owner) = &new.owner {
                let joined_at = now();
                tx.execute(
                    "INSERT INTO project_members (challenge_id, subject, role, display_name, \
                     email, created_by, created_at, updated_at) \
                     VALUES ($1, $2, 'OWNER', $3, $4, $2, $5, $5)",
                    args![
                        &challenge.id,
                        owner.subject.as_str(),
                        owner
                            .display_name
                            .as_deref()
                            .filter(|name| !name.is_empty())
                            .unwrap_or(&owner.subject),
                        owner.email.as_deref(),
                        joined_at
                    ],
                )
                .await?;
            }
            record_event(
                tx,
                &challenge.id,
                "challenge.created",
                actor,
                command_id,
                json!({"slug": slug, "name": challenge.name, "runtime_engine": engine.id()}),
                None,
            )
            .await?;
            Ok(ChallengeView::new(&challenge, Some(&coordinator)))
        })
    }

    /// Every project, oldest first; archived projects only when asked for.
    pub async fn list_projects(&self, include_archived: bool) -> Result<Vec<ChallengeView>> {
        let mut tx = self.db.begin().await?;
        let sql = if include_archived {
            "SELECT * FROM challenges ORDER BY created_at, slug"
        } else {
            "SELECT * FROM challenges WHERE status != 'ARCHIVED' ORDER BY created_at, slug"
        };
        let projects: Vec<Challenge> = tx.fetch_all(sql, args![]).await?;
        let coordinators: Vec<CoordinatorState> =
            tx.fetch_all("SELECT * FROM coordinator_states", args![]).await?;
        tx.commit().await?;
        let by_id: HashMap<&str, &CoordinatorState> =
            coordinators.iter().map(|item| (item.challenge_id.as_str(), item)).collect();
        Ok(projects
            .iter()
            .map(|project| ChallengeView::new(project, by_id.get(project.id.as_str()).copied()))
            .collect())
    }

    /// Starts, pauses, resumes, stops or archives a project. Repeating an action that already
    /// holds is a no-op that still succeeds.
    pub async fn change_project_state(
        &self,
        slug: &str,
        action: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<ChallengeView> {
        let action = ProjectAction::parse(action)?;
        command!(&self.db, command_id, action.command_type(), actor, |tx| {
            let mut challenge = find_challenge(tx, slug).await?;
            let sql = format!(
                "SELECT * FROM coordinator_states WHERE challenge_id = $1{}",
                tx.for_update()
            );
            let coordinator: Option<CoordinatorState> =
                tx.fetch_optional(&sql, args![&challenge.id]).await?;
            let Some(mut coordinator) = coordinator else {
                return Err(LiminaError::not_found(format!(
                    "Project '{slug}' has no runtime state."
                )));
            };
            if challenge.status == "ARCHIVED" {
                if action == ProjectAction::Archive {
                    return Ok(ChallengeView::new(&challenge, Some(&coordinator)));
                }
                return Err(LiminaError::invariant("An archived project cannot be restarted.")
                    .with("action", action.name())
                    .with("status", "ARCHIVED"));
            }
            let (target, coordinator_status) = if action == ProjectAction::Archive {
                if matches!(coordinator.status.as_str(), "RUNNING" | "WAITING") {
                    return Err(LiminaError::invariant(
                        "Pause or stop the project before archiving it.",
                    )
                    .with("status", coordinator.status.as_str()));
                }
                ("ARCHIVED", "STOPPED")
            } else {
                match action.transition(&coordinator.status) {
                    Some(target) => (target, target),
                    None if action.already_applied(&coordinator.status) => {
                        return Ok(ChallengeView::new(&challenge, Some(&coordinator)));
                    }
                    None => {
                        return Err(LiminaError::invariant(format!(
                            "Cannot {} a project while it is {}.",
                            action.name(),
                            coordinator.status
                        ))
                        .with("action", action.name())
                        .with("status", coordinator.status.as_str()));
                    }
                }
            };
            let changed_at = now();
            coordinator = tx
                .fetch_one(
                    "UPDATE coordinator_states SET status = $1, worker_id = NULL, wake_at = NULL, \
                     heartbeat_at = $2, updated_at = $2, version = $3 WHERE challenge_id = $4 \
                     RETURNING *",
                    args![coordinator_status, changed_at, coordinator.version + 1, &challenge.id],
                )
                .await?;
            let challenge_status = if action == ProjectAction::Archive {
                "ARCHIVED"
            } else {
                challenge.status.as_str()
            };
            challenge = tx
                .fetch_one(
                    "UPDATE challenges SET status = $1, updated_at = $2 WHERE id = $3 RETURNING *",
                    args![challenge_status.to_owned(), changed_at, &challenge.id],
                )
                .await?;
            record_event(
                tx,
                &challenge.id,
                action.event_type(),
                actor,
                command_id,
                json!({"action": action.name(), "status": target}),
                None,
            )
            .await?;
            Ok(ChallengeView::new(&challenge, Some(&coordinator)))
        })
    }

    /// Sets a project variable, visible to the project's provider processes and to its members.
    pub async fn set_variable(
        &self,
        slug: &str,
        name: &str,
        value: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<ResourceView> {
        let name = resource_name(name)?;
        resource_value("variable", value)?;
        self.set_resource(slug, &name, "VARIABLE", Some(value), None, actor, command_id).await
    }

    /// Sets a project secret, encrypted at rest and bound to this project and name. Its value is
    /// only ever returned to provider processes.
    pub async fn set_secret(
        &self,
        slug: &str,
        name: &str,
        value: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<ResourceView> {
        let name = resource_name(name)?;
        resource_value("secret", value)?;
        let cipher = self.secret_cipher.as_deref().ok_or_else(no_secret_provider)?;
        let ciphertext = cipher.encrypt(&slug.to_lowercase(), &name, value);
        self.set_resource(slug, &name, "SECRET", None, Some(&ciphertext), actor, command_id).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn set_resource(
        &self,
        slug: &str,
        name: &str,
        resource_type: &str,
        value: Option<&str>,
        secret_ciphertext: Option<&str>,
        actor: &str,
        command_id: &str,
    ) -> Result<ResourceView> {
        let lowered = resource_type.to_lowercase();
        let command_type = format!("resource.{lowered}.set");
        command!(&self.db, command_id, &command_type, actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let existing = find_resource(tx, &challenge.id, name, false).await?;
            let changed_at = now();
            let resource: ProjectResource = match existing {
                Some(existing) => {
                    tx.fetch_one(
                        "UPDATE project_resources SET resource_type = $1, value = $2, \
                         secret_ciphertext = $3, status = 'ACTIVE', created_by = $4, \
                         updated_at = $5 WHERE id = $6 RETURNING *",
                        args![
                            resource_type,
                            value,
                            secret_ciphertext,
                            actor,
                            changed_at,
                            &existing.id
                        ],
                    )
                    .await?
                }
                None => {
                    tx.fetch_one(
                        "INSERT INTO project_resources (id, challenge_id, name, resource_type, \
                         value, secret_ciphertext, status, created_by, created_at, updated_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, 'ACTIVE', $7, $8, $8) RETURNING *",
                        args![
                            new_uuid(),
                            &challenge.id,
                            name,
                            resource_type,
                            value,
                            secret_ciphertext,
                            actor,
                            changed_at
                        ],
                    )
                    .await?
                }
            };
            record_event(
                tx,
                &challenge.id,
                &format!("resource.{lowered}_set"),
                actor,
                command_id,
                json!({"resource_id": resource.id, "name": name, "type": resource_type}),
                None,
            )
            .await?;
            Ok(ResourceView::from(&resource))
        })
    }

    /// A project's resources by name; secret values are never included.
    pub async fn list_resources(&self, slug: &str, active_only: bool) -> Result<Vec<ResourceView>> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let sql = if active_only {
            "SELECT * FROM project_resources WHERE challenge_id = $1 AND status = 'ACTIVE' \
             ORDER BY name"
        } else {
            "SELECT * FROM project_resources WHERE challenge_id = $1 ORDER BY name"
        };
        let resources: Vec<ProjectResource> = tx.fetch_all(sql, args![&challenge.id]).await?;
        tx.commit().await?;
        Ok(resources.iter().map(ResourceView::from).collect())
    }

    /// The project's active variables and decrypted secrets, for its provider processes only.
    pub async fn resource_environment(&self, slug: &str) -> Result<BTreeMap<String, String>> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let resources: Vec<ProjectResource> = tx
            .fetch_all(
                "SELECT * FROM project_resources WHERE challenge_id = $1 AND status = 'ACTIVE' \
                 ORDER BY name",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        let mut environment = BTreeMap::new();
        for resource in resources {
            if resource.resource_type == "VARIABLE" {
                let value = resource.value.ok_or_else(|| {
                    LiminaError::invariant(format!("Variable '{}' has no value.", resource.name))
                })?;
                environment.insert(resource.name, value);
                continue;
            }
            let ciphertext = match resource.secret_ciphertext.as_deref() {
                Some(ciphertext)
                    if resource.resource_type == "SECRET" && !ciphertext.is_empty() =>
                {
                    ciphertext
                }
                _ => {
                    return Err(LiminaError::invariant(format!(
                        "Resource '{}' has invalid secret state.",
                        resource.name
                    )));
                }
            };
            let cipher = self.secret_cipher.as_deref().ok_or_else(no_secret_provider)?;
            let value = cipher.decrypt(&challenge.slug, &resource.name, ciphertext)?;
            environment.insert(resource.name, value);
        }
        Ok(environment)
    }

    /// Removes an active resource and wipes its stored value.
    pub async fn remove_resource(
        &self,
        slug: &str,
        name: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<ResourceView> {
        let name = resource_name(name)?;
        command!(&self.db, command_id, "resource.remove", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let resource =
                find_resource(tx, &challenge.id, &name, true).await?.ok_or_else(|| {
                    LiminaError::not_found(format!("Active resource '{name}' does not exist."))
                })?;
            let resource: ProjectResource = tx
                .fetch_one(
                    "UPDATE project_resources SET status = 'REMOVED', value = NULL, \
                     secret_ciphertext = NULL, updated_at = $1 WHERE id = $2 RETURNING *",
                    args![now(), &resource.id],
                )
                .await?;
            record_event(
                tx,
                &challenge.id,
                "resource.removed",
                actor,
                command_id,
                json!({"resource_id": resource.id, "name": resource.name}),
                None,
            )
            .await?;
            Ok(ResourceView::from(&resource))
        })
    }
}

/// `^[a-z0-9]+(?:-[a-z0-9]+)*$`
fn is_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.split('-').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn no_secret_provider() -> LiminaError {
    LiminaError::invariant("This Limina instance has no secret-encryption provider.")
}

async fn find_resource(
    tx: &mut Tx,
    challenge_id: &str,
    name: &str,
    active_only: bool,
) -> Result<Option<ProjectResource>> {
    let sql = if active_only {
        "SELECT * FROM project_resources WHERE challenge_id = $1 AND name = $2 \
         AND status = 'ACTIVE'"
    } else {
        "SELECT * FROM project_resources WHERE challenge_id = $1 AND name = $2"
    };
    Ok(tx.fetch_optional(sql, args![challenge_id, name]).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_actions_follow_the_python_rules() {
        assert!(is_slug("retrieval-quality-2"));
        for invalid in ["", "-a", "a-", "a--b", "A", "a_b", "é"] {
            assert!(!is_slug(invalid), "{invalid:?}");
        }
        assert_eq!(ProjectAction::parse("RESUME").unwrap(), ProjectAction::Resume);
        let error = ProjectAction::parse("Restart").unwrap_err();
        assert_eq!(error.details["action"], "restart");
        assert_eq!(ProjectAction::Start.transition("STOPPED"), Some("RUNNING"));
        assert_eq!(ProjectAction::Pause.transition("CREATED"), None);
        assert_eq!(ProjectAction::Stop.transition("FAILED"), Some("STOPPED"));
        assert!(ProjectAction::Resume.already_applied("RUNNING"));
    }
}
