//! Public project operations shared by Limina's API transports.
//!
//! The services own the research invariants and the
//! supervisor owns managed execution; this layer composes them into what people and external
//! agents may do: every call checks the caller's project role, scopes idempotency keys to the
//! caller's identity, returns the public views of [`super::views`] and applies the side effects
//! on the supervisor (start, interrupt, resource refresh). HTTP and MCP stay thin adapters.
//!
//! Every method takes the authenticated [`Principal`]. Roles: viewers read, editors steer and
//! curate knowledge, owners manage members, secrets, accounts and archiving.

#[path = "workspace.rs"]
mod workspace;

use std::path::{Component, PathBuf};

use limina_core::{LiminaError, Principal, Result, new_uuid};
use limina_domain::Engine;
use limina_domain::collaboration::paging::{self, Page};
use limina_domain::collaboration::{
    Analytics, CollaborationService, DraftChanges, EngineReadiness, GuidanceItem, LiveTicketGrant,
    Member, MemberChange, Preflight, Role, Run,
};
use limina_domain::engines::SUPPORTED_ENGINES;
use limina_domain::exporter::MarkdownExporter;
use limina_domain::knowledge::{
    Comment, KnowledgeQuery, NewRelation, Relation, Revision, SavedView, Source, SourceInput,
};
use limina_domain::project::{NewChallenge, ProjectOwner};
use limina_domain::service::{ChallengeService, EventView};
use limina_engines::{CodexAuthStatus, DeviceLoginAttempt, EngineAvailability};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::notifier::EventNotifier;
use super::supervisor::{Guidance, ProjectSupervisor};
use super::views::{
    self, ArtifactResponse, EventPage, EventResponse, GuidanceReceipt, HIDDEN_EVENT_TYPES,
    KickoffTemplate, KnowledgeGraphResponse, KnowledgePageResponse, ProjectEngineAccountResponse,
    ProjectResponse, ProjectStatusResponse, ResourceResponse, ReviewResource, ReviewResponse,
    RuntimeRunDetail, SnapshotResponse, TagResponse, public_event, public_project, public_resource,
    public_status,
};

/// The longest `X-Limina-Actor` hint accepted.
pub const ACTOR_LIMIT: usize = 200;
/// Command receipts store 64 characters; composed operations reserve room for `:resource`.
pub const PUBLIC_COMMAND_ID_LIMIT: usize = 55;
/// Uploaded sources larger than this are refused, unless configured otherwise.
pub const DEFAULT_UPLOAD_LIMIT_BYTES: u64 = 25 * 1024 * 1024;
const UPLOAD_NAME_LIMIT: usize = 160;

/// A new project (`POST /v1/projects`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewProject {
    pub slug: String,
    pub name: String,
    /// The mission (`objective` on the wire).
    pub mission: String,
    pub success_criteria: String,
    pub context: String,
    /// `codex` (the default) or `claude-code`.
    pub runtime: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

/// An uploaded file's metadata (`POST /v1/projects/{slug}/sources/upload`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Upload<'a> {
    /// The source's name.
    pub name: &'a str,
    /// The client's file name, sanitized before it touches the disk.
    pub filename: Option<&'a str>,
    pub content_type: Option<&'a str>,
}

/// What `POST /v1/runtime/engines/codex/auth/login` returns: a device login for `chatgpt`,
/// the account status after a server-credential login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RuntimeCodexLogin {
    Device(DeviceLoginAttempt),
    Status(CodexAuthStatus),
}

/// `_command_id`: scopes a client's idempotency key to the authenticated identity.
///
/// The service records a human-friendly actor on events, but display names are not identities
/// and two users may share one. Hashing the signed subject with the caller's key keeps retries
/// stable without exposing the subject or letting receipts collide across users.
pub fn scoped_command_id(principal: &Principal, command_id: &str) -> String {
    let digest = Sha256::digest(format!("{}\0{command_id}", principal.subject).as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("usr:{}", &hex[..40])
}

/// The provider-neutral collaboration contract. Cloning shares the services.
#[derive(Clone)]
pub struct ProjectOperations {
    service: ChallengeService,
    collaboration: CollaborationService,
    exporter: MarkdownExporter,
    supervisor: ProjectSupervisor,
    notifier: EventNotifier,
    upload_limit_bytes: u64,
}

impl ProjectOperations {
    pub fn new(
        service: ChallengeService,
        exporter: MarkdownExporter,
        supervisor: ProjectSupervisor,
        notifier: EventNotifier,
        upload_limit_bytes: u64,
    ) -> Self {
        Self {
            collaboration: CollaborationService::new(service.db().clone()),
            service,
            exporter,
            supervisor,
            notifier,
            upload_limit_bytes,
        }
    }

    pub fn service(&self) -> &ChallengeService {
        &self.service
    }

    pub fn collaboration(&self) -> &CollaborationService {
        &self.collaboration
    }

    pub fn supervisor(&self) -> &ProjectSupervisor {
        &self.supervisor
    }

    pub fn notifier(&self) -> &EventNotifier {
        &self.notifier
    }

    async fn require(&self, slug: &str, principal: &Principal, minimum: Role) -> Result<Role> {
        self.collaboration.require_role(slug, principal, minimum).await
    }

    fn changed(&self, slug: &str) {
        self.notifier.notify(slug);
    }

    /// Project catalogs require membership just like project status; onboarding has no project yet.
    pub async fn model_catalog(
        &self,
        engine: Engine,
        slug: Option<&str>,
        principal: &Principal,
    ) -> Result<limina_engines::models::ModelCatalog> {
        if let Some(slug) = slug {
            self.require(slug, principal, Role::Viewer).await?;
        }
        self.supervisor.model_catalog(engine, slug).await
    }

    // Projects -----------------------------------------------------------------------------------

    /// Creates a project owned by the caller. Execution begins with a `start` action.
    pub async fn create_project(
        &self,
        project: NewProject,
        command_id: &str,
        principal: &Principal,
    ) -> Result<ProjectResponse> {
        let receipt_id = scoped_command_id(principal, command_id);
        let slug = project.slug.clone();
        let new = NewChallenge {
            slug: project.slug,
            name: project.name,
            objective: project.mission,
            success_criteria: project.success_criteria,
            context: project.context,
            runtime_engine: project.runtime,
            model: project.model,
            reasoning_effort: project.reasoning_effort,
            owner: Some(ProjectOwner {
                subject: principal.subject.clone(),
                display_name: Some(principal.display_name.clone()),
                email: principal.email.clone(),
            }),
        };
        let created = self.service.create_challenge(new, &principal.actor(), &receipt_id).await?;
        self.changed(&created.slug);
        let role = self.collaboration.role_for(&slug, principal).await?;
        Ok(public_project(&created, Some(role)))
    }

    /// The projects the caller can see, oldest first, with the caller's role on each.
    pub async fn list_projects(
        &self,
        include_archived: bool,
        cursor: Option<&str>,
        limit: i64,
        principal: &Principal,
    ) -> Result<Page<ProjectResponse>> {
        let roles = self.collaboration.project_roles(principal).await?;
        let projects: Vec<ProjectResponse> = self
            .service
            .list_projects(include_archived)
            .await?
            .iter()
            .filter_map(|project| match &roles {
                None => Some(public_project(project, Some(Role::Owner))),
                Some(roles) => {
                    roles.get(&project.slug).map(|role| public_project(project, Some(*role)))
                }
            })
            .collect();
        let offset = match cursor.filter(|cursor| !cursor.is_empty()) {
            Some(cursor) => paging::page_offset(Some(cursor))?,
            None => 0,
        };
        let limit = paging::clamp_limit(limit);
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(projects.len());
        let end = start.saturating_add(usize::try_from(limit).unwrap_or(0)).min(projects.len());
        let total = i64::try_from(projects.len()).unwrap_or(i64::MAX);
        let items = projects[start..end].to_vec();
        Ok(Page { next_cursor: paging::next_cursor(offset, items.len(), total), items, total })
    }

    pub async fn get_project(&self, slug: &str, principal: &Principal) -> Result<ProjectResponse> {
        let role = self.require(slug, principal, Role::Viewer).await?;
        Ok(public_project(&self.service.get_challenge(slug).await?, Some(role)))
    }

    pub async fn get_status(
        &self,
        slug: &str,
        principal: &Principal,
    ) -> Result<ProjectStatusResponse> {
        let role = self.require(slug, principal, Role::Viewer).await?;
        Ok(public_status(&self.service.status(slug).await?, Some(role)))
    }

    /// Starts, pauses, resumes, stops (editors) or archives (owners) a project, then starts or
    /// interrupts its managed execution.
    pub async fn apply_lifecycle(
        &self,
        slug: &str,
        action: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<ProjectResponse> {
        let action = action.to_lowercase();
        let minimum = if action == "archive" { Role::Owner } else { Role::Editor };
        let role = self.require(slug, principal, minimum).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        let project = self
            .service
            .change_project_state(slug, &action, &principal.actor(), &receipt_id)
            .await?;
        self.changed(slug);
        match action.as_str() {
            "start" | "resume" => self.supervisor.ensure_running(slug).await?,
            "pause" | "stop" | "archive" => self.supervisor.stop_runtime(slug).await?,
            _ => {}
        }
        Ok(public_project(&project, Some(role)))
    }

    /// Changes kickoff fields before the first start, or model defaults for future turns (owners).
    pub async fn update_project(
        &self,
        slug: &str,
        changes: &DraftChanges,
        principal: &Principal,
    ) -> Result<ProjectResponse> {
        let role = self.require(slug, principal, Role::Owner).await?;
        let project = self.collaboration.update_draft(slug, changes, &principal.actor()).await?;
        self.changed(slug);
        Ok(views::public_draft_project(&project, Some(role)))
    }

    /// Kickoff readiness, with the project's engine status (its own account first).
    pub async fn preflight(&self, slug: &str, principal: &Principal) -> Result<Preflight> {
        self.require(slug, principal, Role::Viewer).await?;
        let supervisor = self.supervisor.clone();
        let project = slug.to_owned();
        self.collaboration
            .preflight(slug, move |engine| async move {
                let engine = Engine::parse(&engine)?;
                let status = supervisor.project_engine_status(&project, engine).await?;
                Ok(EngineReadiness {
                    label: status.label,
                    installed: status.installed,
                    credentials: status.credentials,
                    detail: status.detail,
                })
            })
            .await
    }

    pub fn kickoff_templates(&self) -> Vec<KickoffTemplate> {
        views::kickoff_templates()
    }

    /// The runtime's own readiness for every engine.
    pub async fn engine_availability(&self) -> Vec<EngineAvailability> {
        self.supervisor.engine_availability().await
    }

    // Steering -----------------------------------------------------------------------------------

    /// Sends guidance (editors): live into the active turn when possible, otherwise queued.
    /// `INTERRUPT` also pauses a running project.
    pub async fn steer(
        &self,
        slug: &str,
        body: &str,
        kind: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<GuidanceReceipt> {
        self.require(slug, principal, Role::Editor).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        let actor = principal.actor();
        let submitted = if kind.to_uppercase() == "INTERRUPT" {
            self.supervisor.interrupt(slug, &actor, body, Some(&receipt_id)).await?
        } else {
            let guidance = Guidance::new(slug, kind, body, actor).command_id(receipt_id);
            self.supervisor.submit_message(guidance).await?
        };
        Ok(views::guidance_receipt(&submitted.message, submitted.delivery))
    }

    /// Guidance history, newest first.
    pub async fn guidance(
        &self,
        slug: &str,
        status: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
        principal: &Principal,
    ) -> Result<Page<GuidanceItem>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.guidance(slug, status, cursor, limit).await
    }

    // Review and activity ------------------------------------------------------------------------

    /// Status, resources, a page of knowledge split by kind and the latest 50 activity entries.
    pub async fn review(
        &self,
        slug: &str,
        cursor: Option<&str>,
        limit: i64,
        principal: &Principal,
    ) -> Result<ReviewResponse> {
        let role = self.require(slug, principal, Role::Viewer).await?;
        let status = self.service.status(slug).await?;
        let query = KnowledgeQuery {
            cursor: cursor.map(str::to_owned),
            limit,
            ..KnowledgeQuery::default()
        };
        let page = self.collaboration.query_knowledge(slug, &query).await?;
        let events = self.service.recent_events(slug, 200).await?;
        let events = self.with_guidance_bodies(slug, events).await?;
        let visible: Vec<EventResponse> = events.iter().filter_map(public_event).collect();
        let recent = visible[visible.len().saturating_sub(50)..].to_vec();
        let resources = self
            .service
            .list_resources(slug, true)
            .await?
            .iter()
            .map(|resource| ReviewResource::from(public_resource(resource)))
            .collect();
        let artifacts: Vec<ArtifactResponse> =
            page.items.into_iter().map(ArtifactResponse::from).collect();
        let of_kind = |kind: &str| -> Vec<ArtifactResponse> {
            artifacts.iter().filter(|item| item.kind == kind).cloned().collect()
        };
        Ok(ReviewResponse {
            status: public_status(&status, Some(role)),
            resources,
            hypotheses: of_kind("H"),
            experiments: of_kind("E"),
            findings: of_kind("F"),
            recent_activity: recent,
            knowledge_cursor: page.next_cursor,
            knowledge_total: page.total,
        })
    }

    /// Pages durable activity forward from `after`, or backward from `before`.
    ///
    /// Forward pages keep `cursor` as the last sequence scanned, so followers never re-read or
    /// skip hidden rows. Backward pages return the newest visible events below `before`, oldest
    /// first.
    pub async fn activity(
        &self,
        slug: &str,
        after: i64,
        before: Option<i64>,
        limit: i64,
        principal: &Principal,
    ) -> Result<EventPage> {
        self.require(slug, principal, Role::Viewer).await?;
        let (events, cursor, has_earlier) = if let Some(before) = before {
            let (events, has_earlier) =
                self.service.events_before(slug, before, after, limit, &HIDDEN_EVENT_TYPES).await?;
            let cursor = events
                .last()
                .map_or_else(|| (before - 1).max(after), |event| i64::from(event.sequence));
            (events, cursor, has_earlier)
        } else {
            let events = self.service.events(slug, after, limit).await?;
            let cursor = events.last().map_or(after, |event| i64::from(event.sequence));
            let boundary = events.first().map_or(after + 1, |event| i64::from(event.sequence));
            let has_earlier = after > 0
                && self.service.has_events_before(slug, boundary, &HIDDEN_EVENT_TYPES).await?;
            (events, cursor, has_earlier)
        };
        let events = self.with_guidance_bodies(slug, events).await?;
        Ok(EventPage {
            events: events.iter().filter_map(public_event).collect(),
            cursor,
            has_earlier,
        })
    }

    /// Backfills guidance text on events recorded before it was part of the payload.
    async fn with_guidance_bodies(
        &self,
        slug: &str,
        mut events: Vec<EventView>,
    ) -> Result<Vec<EventView>> {
        fn missing_body(event: &EventView) -> Option<&str> {
            let payload = event.payload.as_object()?;
            if event.event_type != "inbox.message_sent" || payload.contains_key("body") {
                return None;
            }
            payload.get("message_id")?.as_str()
        }
        let missing: Vec<String> =
            events.iter().filter_map(missing_body).map(str::to_owned).collect();
        if missing.is_empty() {
            return Ok(events);
        }
        let bodies = self.service.message_bodies(slug, &missing).await?;
        for event in &mut events {
            let Some(body) = missing_body(event).and_then(|id| bodies.get(id)).cloned() else {
                continue;
            };
            if let Some(payload) = event.payload.as_object_mut() {
                payload.insert("body".to_owned(), Value::String(body));
            }
        }
        Ok(events)
    }

    /// The project's knowledge base as Markdown files.
    pub async fn snapshot(&self, slug: &str, principal: &Principal) -> Result<SnapshotResponse> {
        self.require(slug, principal, Role::Viewer).await?;
        Ok(SnapshotResponse { files: self.exporter.snapshot(slug).await? })
    }

    pub async fn knowledge_document(
        &self,
        slug: &str,
        key: &str,
        principal: &Principal,
    ) -> Result<SnapshotResponse> {
        self.require(slug, principal, Role::Viewer).await?;
        Ok(SnapshotResponse { files: self.exporter.document(slug, key).await? })
    }

    pub async fn metrics(
        &self,
        slug: &str,
        principal: &Principal,
    ) -> Result<Vec<limina_domain::progress::Metric>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.metrics(slug).await
    }

    pub async fn set_metric(
        &self,
        slug: &str,
        id: &str,
        input: limina_domain::progress::MetricInput,
        expected_version: i32,
        command_id: &str,
        principal: &Principal,
    ) -> Result<limina_domain::progress::Metric> {
        self.require(slug, principal, Role::Editor).await?;
        let metric = self
            .collaboration
            .set_metric(
                slug,
                id,
                input,
                expected_version,
                &principal.actor(),
                &scoped_command_id(principal, command_id),
            )
            .await?;
        self.changed(slug);
        Ok(metric)
    }

    // Resources ----------------------------------------------------------------------------------

    /// Sets a project variable (editors) and restarts active work with it.
    pub async fn set_variable(
        &self,
        slug: &str,
        name: &str,
        value: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<ResourceResponse> {
        self.require(slug, principal, Role::Editor).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        let actor = principal.actor();
        let resource = self.service.set_variable(slug, name, value, &actor, &receipt_id).await?;
        self.changed(slug);
        let body = format!("Project variable '{}' was set.", resource.name);
        self.resource_changed(slug, &body, "ANSWER", &actor, &receipt_id).await?;
        Ok(public_resource(&resource))
    }

    /// Sets a project secret (owners) and restarts active work with it. The value is write-only.
    pub async fn set_secret(
        &self,
        slug: &str,
        name: &str,
        value: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<ResourceResponse> {
        self.require(slug, principal, Role::Owner).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        let actor = principal.actor();
        let resource = self.service.set_secret(slug, name, value, &actor, &receipt_id).await?;
        self.changed(slug);
        let body = format!("Project secret '{}' was set.", resource.name);
        self.resource_changed(slug, &body, "ANSWER", &actor, &receipt_id).await?;
        Ok(public_resource(&resource))
    }

    pub async fn list_resources(
        &self,
        slug: &str,
        principal: &Principal,
    ) -> Result<Vec<ResourceResponse>> {
        self.require(slug, principal, Role::Viewer).await?;
        Ok(self.service.list_resources(slug, true).await?.iter().map(public_resource).collect())
    }

    /// Revokes a resource (owners) and restarts active work without it.
    pub async fn remove_resource(
        &self,
        slug: &str,
        name: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<ResourceResponse> {
        self.require(slug, principal, Role::Owner).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        let actor = principal.actor();
        let resource = self.service.remove_resource(slug, name, &actor, &receipt_id).await?;
        self.changed(slug);
        let body = format!("Resource '{}' has been revoked.", resource.name);
        self.resource_changed(slug, &body, "STEER", &actor, &receipt_id).await?;
        Ok(public_resource(&resource))
    }

    /// Tells the project about the change (queued guidance marked `resource`), restarts an
    /// active turn and wakes a running project.
    async fn resource_changed(
        &self,
        slug: &str,
        body: &str,
        kind: &str,
        actor: &str,
        receipt_id: &str,
    ) -> Result<()> {
        let guidance = Guidance::new(slug, kind, body, actor)
            .command_id(format!("{receipt_id}:resource"))
            .queued_only()
            .origin("resource");
        self.supervisor.submit_message(guidance).await?;
        self.supervisor.refresh_resources(slug).await;
        self.wake_if_active(slug).await
    }

    /// Wakes a `RUNNING` or `WAITING` project so a change applies from its next turn.
    async fn wake_if_active(&self, slug: &str) -> Result<()> {
        let project = self.service.get_challenge(slug).await?;
        if project
            .coordinator
            .as_ref()
            .is_some_and(|state| matches!(state.status.as_str(), "RUNNING" | "WAITING"))
        {
            self.supervisor.ensure_running(slug).await?;
        }
        Ok(())
    }

    // Engine accounts ----------------------------------------------------------------------------

    /// Which account each engine runs on for this project (viewers). Never contains a secret.
    pub async fn engine_accounts(
        &self,
        slug: &str,
        principal: &Principal,
    ) -> Result<Vec<ProjectEngineAccountResponse>> {
        self.require(slug, principal, Role::Viewer).await?;
        let mut views = Vec::with_capacity(SUPPORTED_ENGINES.len());
        for engine in SUPPORTED_ENGINES {
            views.push(self.engine_account_view(slug, engine).await?);
        }
        Ok(views)
    }

    /// Connects a Claude token (owners). It applies from the project's next turn.
    pub async fn connect_claude_account(
        &self,
        slug: &str,
        token: &str,
        principal: &Principal,
    ) -> Result<ProjectEngineAccountResponse> {
        self.require(slug, principal, Role::Owner).await?;
        self.supervisor.engine_accounts().connect_claude(slug, token, principal).await?;
        self.wake_if_active(slug).await?;
        self.engine_account_view(slug, Engine::ClaudeCode).await
    }

    /// Starts a Claude sign-in for the project (owners): the attempt carries the claude.com URL.
    pub async fn start_claude_account_login(
        &self,
        slug: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.require(slug, principal, Role::Owner).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        self.supervisor.engine_accounts().start_claude_login(slug, &receipt_id).await
    }

    /// A Claude sign-in attempt (owners).
    pub async fn claude_account_login(
        &self,
        slug: &str,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.require(slug, principal, Role::Owner).await?;
        self.supervisor.engine_accounts().claude_login(slug, login_id, principal).await
    }

    /// Completes a Claude sign-in with the code claude.com showed (owners). Success records the
    /// account and wakes the project.
    pub async fn submit_claude_account_code(
        &self,
        slug: &str,
        login_id: &str,
        code: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.require(slug, principal, Role::Owner).await?;
        let attempt = self
            .supervisor
            .engine_accounts()
            .submit_claude_code(slug, login_id, code, principal)
            .await?;
        if attempt.status == "SUCCEEDED" {
            self.wake_if_active(slug).await?;
        }
        Ok(attempt)
    }

    pub async fn cancel_claude_account_login(
        &self,
        slug: &str,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.require(slug, principal, Role::Owner).await?;
        self.supervisor.engine_accounts().cancel_claude_login(slug, login_id).await
    }

    /// Starts a Codex device-code sign-in for the project (owners).
    pub async fn start_codex_account_login(
        &self,
        slug: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.require(slug, principal, Role::Owner).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        self.supervisor.engine_accounts().start_codex_login(slug, &receipt_id).await
    }

    /// A Codex sign-in attempt (owners). Success records the account and wakes the project.
    pub async fn codex_account_login(
        &self,
        slug: &str,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.require(slug, principal, Role::Owner).await?;
        let attempt =
            self.supervisor.engine_accounts().codex_login(slug, login_id, principal).await?;
        if attempt.status == "SUCCEEDED" {
            self.wake_if_active(slug).await?;
        }
        Ok(attempt)
    }

    pub async fn cancel_codex_account_login(
        &self,
        slug: &str,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        self.require(slug, principal, Role::Owner).await?;
        self.supervisor.engine_accounts().cancel_codex_login(slug, login_id).await
    }

    /// Disconnects the project's account for `engine` (owners); turns fall back to the
    /// runtime's credentials.
    pub async fn disconnect_engine_account(
        &self,
        slug: &str,
        engine: &str,
        principal: &Principal,
    ) -> Result<ProjectEngineAccountResponse> {
        self.require(slug, principal, Role::Owner).await?;
        let engine = Engine::parse(engine)?;
        self.supervisor.engine_accounts().disconnect(slug, engine).await?;
        self.wake_if_active(slug).await?;
        self.engine_account_view(slug, engine).await
    }

    async fn engine_account_view(
        &self,
        slug: &str,
        engine: Engine,
    ) -> Result<ProjectEngineAccountResponse> {
        let runtime = self.supervisor.engine_status(engine).await;
        let account = self.supervisor.engine_accounts().account(slug, engine).await?;
        Ok(ProjectEngineAccountResponse {
            engine,
            label: runtime.label,
            account,
            runtime_available: runtime.installed && runtime.credentials,
            runtime_detail: runtime.detail,
        })
    }

    // The runtime's own Codex account (instance administrators) ----------------------------------

    fn require_instance_admin(principal: &Principal) -> Result<()> {
        if principal.instance_admin {
            return Ok(());
        }
        Err(LiminaError::authorization_with("Instance administrator access is required."))
    }

    /// The runtime's Codex account, waiting while a login or logout is in progress.
    pub async fn runtime_codex_status(&self, principal: &Principal) -> Result<CodexAuthStatus> {
        Self::require_instance_admin(principal)?;
        Ok(self.supervisor.codex_auth().status().await)
    }

    /// Signs the runtime in to Codex: a device login for `chatgpt`, otherwise the server's
    /// `api-key` or `access-token` credential. The idempotency key is used as given.
    pub async fn runtime_codex_login(
        &self,
        method: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<RuntimeCodexLogin> {
        Self::require_instance_admin(principal)?;
        let auth = self.supervisor.codex_auth();
        let outcome = if method == "chatgpt" {
            auth.start_device_login(command_id).await.map(RuntimeCodexLogin::Device)
        } else {
            auth.login_from_environment(method).await.map(RuntimeCodexLogin::Status)
        };
        self.supervisor.forget_codex_auth_status();
        outcome
    }

    pub async fn runtime_codex_login_attempt(
        &self,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        Self::require_instance_admin(principal)?;
        let attempt = self.supervisor.codex_auth().login_attempt(login_id)?;
        if attempt.status != "PENDING" {
            self.supervisor.forget_codex_auth_status();
        }
        Ok(attempt)
    }

    pub async fn runtime_codex_cancel_login(
        &self,
        login_id: &str,
        principal: &Principal,
    ) -> Result<DeviceLoginAttempt> {
        Self::require_instance_admin(principal)?;
        let outcome = self.supervisor.codex_auth().cancel_device_login(login_id).await;
        self.supervisor.forget_codex_auth_status();
        outcome
    }

    pub async fn runtime_codex_logout(&self, principal: &Principal) -> Result<CodexAuthStatus> {
        Self::require_instance_admin(principal)?;
        let outcome = self.supervisor.codex_auth().logout().await;
        self.supervisor.forget_codex_auth_status();
        outcome
    }

    // Members and live tickets -------------------------------------------------------------------

    pub async fn members(&self, slug: &str, principal: &Principal) -> Result<Vec<Member>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.list_members(slug).await
    }

    /// Adds or changes a member (owners). A project always keeps an owner.
    pub async fn set_member(
        &self,
        slug: &str,
        change: &MemberChange,
        principal: &Principal,
    ) -> Result<Member> {
        self.require(slug, principal, Role::Owner).await?;
        let member = self.collaboration.set_member(slug, change, &principal.actor()).await?;
        self.changed(slug);
        Ok(member)
    }

    pub async fn remove_member(
        &self,
        slug: &str,
        subject: &str,
        principal: &Principal,
    ) -> Result<Member> {
        self.require(slug, principal, Role::Owner).await?;
        let member = self.collaboration.remove_member(slug, subject, &principal.actor()).await?;
        self.changed(slug);
        Ok(member)
    }

    /// A single-use ticket for the live channel, carrying the caller's role.
    pub async fn issue_live_ticket(
        &self,
        slug: &str,
        principal: &Principal,
    ) -> Result<LiveTicketGrant> {
        let role = self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.issue_live_ticket(slug, principal, role).await
    }

    // Knowledge ----------------------------------------------------------------------------------

    pub async fn query_knowledge(
        &self,
        slug: &str,
        query: &KnowledgeQuery,
        principal: &Principal,
    ) -> Result<KnowledgePageResponse> {
        self.require(slug, principal, Role::Viewer).await?;
        let page = self.collaboration.query_knowledge(slug, query).await?;
        Ok(KnowledgePageResponse {
            items: page.items.into_iter().map(ArtifactResponse::from).collect(),
            next_cursor: page.next_cursor,
            total: page.total,
            search_backend: page.search_backend,
        })
    }

    /// One artifact with its tags (and an experiment's observations).
    pub async fn get_knowledge(
        &self,
        slug: &str,
        artifact_id: &str,
        principal: &Principal,
    ) -> Result<ArtifactResponse> {
        self.require(slug, principal, Role::Viewer).await?;
        let mut artifact =
            views::public_artifact(&self.service.get_artifact(slug, artifact_id).await?);
        artifact.tags = self.collaboration.tags(slug, artifact_id).await?;
        Ok(artifact)
    }

    pub async fn knowledge_graph(
        &self,
        slug: &str,
        principal: &Principal,
    ) -> Result<KnowledgeGraphResponse> {
        self.require(slug, principal, Role::Viewer).await?;
        let graph = self.collaboration.graph(slug).await?;
        Ok(KnowledgeGraphResponse {
            nodes: graph.nodes.into_iter().map(ArtifactResponse::from).collect(),
            edges: graph.edges,
        })
    }

    pub async fn revisions(
        &self,
        slug: &str,
        artifact_id: &str,
        principal: &Principal,
    ) -> Result<Vec<Revision>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.revisions(slug, artifact_id).await
    }

    pub async fn create_relation(
        &self,
        slug: &str,
        relation: &NewRelation,
        principal: &Principal,
    ) -> Result<Relation> {
        self.require(slug, principal, Role::Editor).await?;
        let created =
            self.collaboration.create_relation(slug, relation, &principal.actor()).await?;
        self.changed(slug);
        Ok(created)
    }

    pub async fn delete_relation(
        &self,
        slug: &str,
        relation_id: &str,
        principal: &Principal,
    ) -> Result<Relation> {
        self.require(slug, principal, Role::Editor).await?;
        let deleted =
            self.collaboration.delete_relation(slug, relation_id, &principal.actor()).await?;
        self.changed(slug);
        Ok(deleted)
    }

    pub async fn comments(
        &self,
        slug: &str,
        artifact_id: &str,
        principal: &Principal,
    ) -> Result<Vec<Comment>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.comments(slug, artifact_id).await
    }

    /// Comments on an artifact (editors), idempotent on the caller's key.
    pub async fn add_comment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: &str,
        command_id: &str,
        principal: &Principal,
    ) -> Result<Comment> {
        self.require(slug, principal, Role::Editor).await?;
        let receipt_id = scoped_command_id(principal, command_id);
        let comment = self
            .collaboration
            .add_comment(slug, artifact_id, body, &principal.actor(), &receipt_id)
            .await?;
        self.changed(slug);
        Ok(comment)
    }

    pub async fn tags(
        &self,
        slug: &str,
        artifact_id: &str,
        principal: &Principal,
    ) -> Result<TagResponse> {
        self.require(slug, principal, Role::Viewer).await?;
        Ok(TagResponse { tags: self.collaboration.tags(slug, artifact_id).await? })
    }

    pub async fn add_tag(
        &self,
        slug: &str,
        artifact_id: &str,
        tag: &str,
        principal: &Principal,
    ) -> Result<TagResponse> {
        self.require(slug, principal, Role::Editor).await?;
        let tags = self.collaboration.add_tag(slug, artifact_id, tag, &principal.actor()).await?;
        self.changed(slug);
        Ok(TagResponse { tags })
    }

    pub async fn remove_tag(
        &self,
        slug: &str,
        artifact_id: &str,
        tag: &str,
        principal: &Principal,
    ) -> Result<TagResponse> {
        self.require(slug, principal, Role::Editor).await?;
        let tags =
            self.collaboration.remove_tag(slug, artifact_id, tag, &principal.actor()).await?;
        self.changed(slug);
        Ok(TagResponse { tags })
    }

    pub async fn saved_views(&self, slug: &str, principal: &Principal) -> Result<Vec<SavedView>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.saved_views(slug).await
    }

    pub async fn save_view(
        &self,
        slug: &str,
        name: &str,
        query: &Map<String, Value>,
        principal: &Principal,
    ) -> Result<SavedView> {
        self.require(slug, principal, Role::Editor).await?;
        let view = self.collaboration.save_view(slug, name, query, &principal.actor()).await?;
        self.changed(slug);
        Ok(view)
    }

    pub async fn delete_view(
        &self,
        slug: &str,
        view_id: &str,
        principal: &Principal,
    ) -> Result<SavedView> {
        self.require(slug, principal, Role::Editor).await?;
        let view = self.collaboration.delete_view(slug, view_id, &principal.actor()).await?;
        self.changed(slug);
        Ok(view)
    }

    // Sources ------------------------------------------------------------------------------------

    pub async fn sources(&self, slug: &str, principal: &Principal) -> Result<Vec<Source>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.sources(slug).await
    }

    /// Adds or replaces a URL or connector source (editors). Sources never carry credentials.
    pub async fn set_source(
        &self,
        slug: &str,
        source: &SourceInput,
        principal: &Principal,
    ) -> Result<Source> {
        self.require(slug, principal, Role::Editor).await?;
        let stored = self.collaboration.set_source(slug, source, &principal.actor()).await?;
        self.changed(slug);
        Ok(stored)
    }

    /// Stores an uploaded file in the project's workspace (`.limina/sources/<id>/<name>`) and
    /// registers it as an `UPLOAD` source (editors). Uploads larger than the configured limit
    /// (`LIMINA_UPLOAD_LIMIT_BYTES`, 25 MiB by default) are refused and nothing is kept.
    pub async fn upload_source<R>(
        &self,
        slug: &str,
        upload: Upload<'_>,
        mut body: R,
        principal: &Principal,
    ) -> Result<Source>
    where
        R: AsyncRead + Unpin + Send,
    {
        self.require(slug, principal, Role::Editor).await?;
        let client_name = upload.filename.filter(|name| !name.is_empty());
        let safe_name = safe_upload_name(client_name.unwrap_or("source.bin"));
        let relative = PathBuf::from(".limina")
            .join("sources")
            .join(new_uuid().replace('-', ""))
            .join(&safe_name);
        let workspace = self.supervisor.workspace_root().join(slug.to_lowercase());
        let destination = workspace.join(&relative);
        let parent = destination.parent().map(PathBuf::from).unwrap_or_else(|| workspace.clone());
        tokio::fs::create_dir_all(&parent).await.map_err(limina_core::internal)?;
        let resolved_workspace =
            tokio::fs::canonicalize(&workspace).await.map_err(limina_core::internal)?;
        let resolved_parent =
            tokio::fs::canonicalize(&parent).await.map_err(limina_core::internal)?;
        if !resolved_parent.starts_with(&resolved_workspace) {
            return Err(LiminaError::invariant(
                "The project upload directory is outside its workspace.",
            ));
        }
        let destination = resolved_parent.join(&safe_name);
        let written = write_upload(&destination, &mut body, self.upload_limit_bytes).await;
        let (size, digest) = match written {
            Ok(written) => written,
            Err(error) => {
                let _ = tokio::fs::remove_file(&destination).await;
                return Err(error);
            }
        };
        let uri = relative
            .components()
            .filter_map(|part| match part {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        let metadata = json!({
            "filename": client_name.unwrap_or(&safe_name),
            "size_bytes": size,
            "sha256": digest,
        });
        let source = SourceInput {
            name: upload.name.to_owned(),
            source_type: "UPLOAD".to_owned(),
            uri: format!("workspace://{uri}"),
            media_type: upload.content_type.map(str::to_owned),
            metadata: metadata.as_object().cloned().unwrap_or_default(),
        };
        let stored = self.collaboration.set_source(slug, &source, &principal.actor()).await?;
        self.changed(slug);
        Ok(stored)
    }

    pub async fn remove_source(
        &self,
        slug: &str,
        source_id: &str,
        principal: &Principal,
    ) -> Result<Source> {
        self.require(slug, principal, Role::Editor).await?;
        let removed = self.collaboration.remove_source(slug, source_id, &principal.actor()).await?;
        self.changed(slug);
        Ok(removed)
    }

    // Runs and analytics -------------------------------------------------------------------------

    pub async fn runs(
        &self,
        slug: &str,
        status: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
        principal: &Principal,
    ) -> Result<Page<Run>> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.runs(slug, status, cursor, limit).await
    }

    /// One run with the public events it recorded.
    pub async fn run(
        &self,
        slug: &str,
        run_id: &str,
        principal: &Principal,
    ) -> Result<RuntimeRunDetail> {
        self.require(slug, principal, Role::Viewer).await?;
        let detail = self.collaboration.run(slug, run_id).await?;
        Ok(RuntimeRunDetail {
            run: detail.run,
            events: detail.events.iter().filter_map(views::public_run_event).collect(),
        })
    }

    pub async fn analytics(
        &self,
        slug: &str,
        days: i64,
        principal: &Principal,
    ) -> Result<Analytics> {
        self.require(slug, principal, Role::Viewer).await?;
        self.collaboration.analytics(slug, days).await
    }
}

/// Streams an upload to a new file, refusing it past `limit` bytes. Returns its size and SHA-256.
async fn write_upload<R: AsyncRead + Unpin>(
    destination: &std::path::Path,
    body: &mut R,
    limit: u64,
) -> Result<(u64, String)> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(limina_core::internal)?;
    let mut digest = Sha256::new();
    let mut size: u64 = 0;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = body.read(&mut buffer).await.map_err(limina_core::internal)?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if size > limit {
            return Err(LiminaError::invariant(format!(
                "Uploaded sources must be at most {limit} bytes."
            )));
        }
        let chunk = &buffer[..read];
        digest.update(chunk);
        file.write_all(chunk).await.map_err(limina_core::internal)?;
    }
    file.flush().await.map_err(limina_core::internal)?;
    let hex: String = digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    Ok((size, hex))
}

/// `re.sub(r"[^A-Za-z0-9._-]+", "-", name).strip(".-")[:160] or "source.bin"`.
pub fn safe_upload_name(name: &str) -> String {
    let mut safe = String::with_capacity(name.len());
    let mut replacing = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || "._-".contains(character) {
            safe.push(character);
            replacing = false;
        } else if !replacing {
            safe.push('-');
            replacing = true;
        }
    }
    let trimmed: String = safe.trim_matches(['.', '-']).chars().take(UPLOAD_NAME_LIMIT).collect();
    if trimmed.is_empty() { "source.bin".to_owned() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_are_scoped_to_the_subject() {
        let owner = Principal::new("owner", "Olivia Owner");
        let namesake = Principal::new("other-owner", "Olivia Owner");
        let first = scoped_command_id(&owner, "shared-key");
        assert_eq!(first.len(), 44);
        assert!(first.starts_with("usr:"));
        assert_eq!(first, scoped_command_id(&owner, "shared-key"));
        assert_ne!(first, scoped_command_id(&namesake, "shared-key"));
        // sha256("owner\0shared-key")[:40], as the Python runtime computes it.
        assert_eq!(first, "usr:66515f6eb85e71b80b6e5b9763cce5c0268c0832");
    }

    #[test]
    fn upload_names_are_sanitized_like_python() {
        assert_eq!(safe_upload_name("brief.md"), "brief.md");
        assert_eq!(safe_upload_name("../../etc/passwd"), "etc-passwd");
        assert_eq!(safe_upload_name("my report (final).pdf"), "my-report-final-.pdf");
        assert_eq!(safe_upload_name("résumé.txt"), "r-sum-.txt");
        assert_eq!(safe_upload_name("..."), "source.bin");
        assert_eq!(safe_upload_name(&"a".repeat(200)).len(), 160);
    }
}
