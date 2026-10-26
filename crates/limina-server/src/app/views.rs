//! The public shapes the API returns, and the functions that build them from service results.
//!
//! Each type serializes exactly like the Python runtime's HTTP response (after FastAPI applied the
//! response model in `schemas.py`): same keys, order and nullability. In particular,
//! [`ArtifactResponse::observations`] is `null` for artifacts without observations,
//! [`ResourceResponse`] omits the field that does not apply to its type (`exclude_none`), and
//! [`ReviewResource`] keeps both as `null` (the review route does not exclude them).
//!
//! Internal coordination never leaks: leases, workers, threads, continuations and versions are
//! dropped from events, and internal `limina _agent` command lines are masked.

use std::collections::BTreeMap;

use limina_core::Timestamp;
use limina_core::time::serde_iso;
use limina_domain::collaboration::{DraftProject, Role, Run, RunEvent};
use limina_domain::knowledge::{KnowledgeArtifact, Relation};
use limina_domain::service::{
    ArtifactDetail, ArtifactView, ChallengeView, EventView, MessageView, ObservationView,
    ResourceView, StatusView,
};
use limina_engines::events::{
    KNOWLEDGE_SUMMARY, contains_internal_command, is_internal_command, redact_internal_commands,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::engine_accounts::EngineAccount;

/// Events that are pure coordination and never shown.
pub const HIDDEN_EVENT_TYPES: [&str; 2] = ["coordinator.claimed", "coordinator.released"];
/// Payload keys that describe internal coordination.
pub const PRIVATE_EVENT_KEYS: [&str; 7] =
    ["worker_id", "thread_id", "continuation_id", "turn_id", "version", "expires_at", "message_id"];
/// Provider transcript events, whose text may contain internal commands.
pub const TRANSCRIPT_EVENT_TYPES: [&str; 2] = ["runtime.codex", "runtime.claude-code"];
/// Transcript kinds whose summary is prose (masked in place rather than replaced).
pub const PROSE_KINDS: [&str; 3] = ["message", "plan", "status"];
/// Stored event types and the names the API shows instead.
pub const EVENT_ALIASES: [(&str, &str); 4] = [
    ("challenge.created", "project.created"),
    ("coordinator.checkpointed", "project.checkpoint"),
    ("inbox.message_sent", "guidance.received"),
    ("inbox.message_acknowledged", "guidance.incorporated"),
];

/// A project as people see it (`ProjectResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectResponse {
    pub slug: String,
    pub name: String,
    pub mission: String,
    pub success_criteria: String,
    pub context: String,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// `ARCHIVED`, or the coordinator's status.
    pub status: String,
    pub current_objective: String,
    pub next_step: String,
    pub blocker: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    /// When the coordinator last changed.
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
    /// The caller's role on the project.
    pub role: Option<Role>,
}

/// A project and its knowledge counts, active work, pending guidance and newest event
/// (`ProjectStatusResponse`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectStatusResponse {
    pub project: ProjectResponse,
    /// Artifact counts by kind, then status.
    pub knowledge: BTreeMap<String, BTreeMap<String, i64>>,
    pub active_work: Vec<ArtifactResponse>,
    pub pending_guidance: i64,
    pub event_cursor: i64,
}

/// A knowledge artifact (`ArtifactResponse`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactResponse {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub content: Value,
    pub hypothesis_id: Option<String>,
    pub experiment_id: Option<String>,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
    /// An experiment's observations when it is read on its own; `null` otherwise.
    pub observations: Option<Vec<ObservationView>>,
    pub tags: Vec<String>,
}

impl From<KnowledgeArtifact> for ArtifactResponse {
    fn from(artifact: KnowledgeArtifact) -> Self {
        Self {
            id: artifact.id,
            kind: artifact.kind,
            title: artifact.title,
            status: artifact.status,
            content: artifact.content,
            hypothesis_id: artifact.hypothesis_id,
            experiment_id: artifact.experiment_id,
            created_at: artifact.created_at,
            updated_at: artifact.updated_at,
            observations: None,
            tags: artifact.tags,
        }
    }
}

/// A project variable or secret (`ResourceResponse` with `exclude_none`): variables carry their
/// `value`, secrets only whether they are `configured`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceResponse {
    pub name: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured: Option<bool>,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

/// A resource inside a review, where both `value` and `configured` are always present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewResource {
    pub name: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub status: String,
    pub value: Option<String>,
    pub configured: Option<bool>,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl From<ResourceResponse> for ReviewResource {
    fn from(resource: ResourceResponse) -> Self {
        Self {
            name: resource.name,
            resource_type: resource.resource_type,
            status: resource.status,
            value: resource.value,
            configured: resource.configured,
            created_at: resource.created_at,
            updated_at: resource.updated_at,
        }
    }
}

/// One public activity entry (`EventResponse`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventResponse {
    pub sequence: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub actor: String,
    pub artifact_id: Option<String>,
    pub detail: Map<String, Value>,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
}

/// A page of activity (`EventPage`). `cursor` is the last sequence scanned, hidden rows included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventPage {
    pub events: Vec<EventResponse>,
    pub cursor: i64,
    pub has_earlier: bool,
}

/// Everything a reviewer needs on one page (`ReviewResponse`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewResponse {
    #[serde(flatten)]
    pub status: ProjectStatusResponse,
    pub resources: Vec<ReviewResource>,
    pub hypotheses: Vec<ArtifactResponse>,
    pub experiments: Vec<ArtifactResponse>,
    pub findings: Vec<ArtifactResponse>,
    pub recent_activity: Vec<EventResponse>,
    pub knowledge_cursor: Option<String>,
    pub knowledge_total: i64,
}

/// How guidance reached the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Delivery {
    /// Steered into the active turn.
    Live,
    /// Waiting for the next turn.
    Queued,
}

/// The receipt for a steering message (`GuidanceReceipt`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidanceReceipt {
    pub id: String,
    pub delivery: Delivery,
    pub kind: String,
    #[serde(with = "serde_iso")]
    pub accepted_at: Timestamp,
    pub status: String,
}

/// The project's knowledge base as Markdown files (`SnapshotResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub files: BTreeMap<String, String>,
}

/// An artifact's tags (`TagResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagResponse {
    pub tags: Vec<String>,
}

/// One page of knowledge (`KnowledgePage`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgePageResponse {
    pub items: Vec<ArtifactResponse>,
    pub next_cursor: Option<String>,
    pub total: i64,
    /// `postgresql-fts` or `portable-substring`.
    pub search_backend: String,
}

/// Every artifact and every edge between them (`KnowledgeGraphResponse`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeGraphResponse {
    pub nodes: Vec<ArtifactResponse>,
    pub edges: Vec<Relation>,
}

/// A run with the public events it recorded (`RuntimeRunDetail`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRunDetail {
    #[serde(flatten)]
    pub run: Run,
    pub events: Vec<EventResponse>,
}

/// A starting point for a new project's kickoff fields (`KickoffTemplate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KickoffTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub defaults: TemplateDefaults,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDefaults {
    pub context: String,
    pub success_criteria: String,
}

/// Which account a project's turns use for one engine (`ProjectEngineAccountResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEngineAccountResponse {
    pub engine: limina_domain::Engine,
    pub label: String,
    /// The project's own account, or `null` when it uses the runtime's credentials.
    pub account: Option<EngineAccount>,
    /// Whether the runtime's own credentials could run this engine.
    pub runtime_available: bool,
    pub runtime_detail: String,
}

// Builders ---------------------------------------------------------------------------------------

/// `public_project`: a project with the caller's role. A project without coordinator state reads
/// as freshly created.
pub fn public_project(project: &ChallengeView, role: Option<Role>) -> ProjectResponse {
    let coordinator = project.coordinator.as_ref();
    let coordinator_status = coordinator.map_or("CREATED", |state| state.status.as_str());
    ProjectResponse {
        slug: project.slug.clone(),
        name: project.name.clone(),
        mission: project.objective.clone(),
        success_criteria: project.success_criteria.clone(),
        context: project.context.clone(),
        runtime: project.runtime_engine.clone(),
        model: project.model.clone(),
        reasoning_effort: project.reasoning_effort.clone(),
        status: if project.status == "ARCHIVED" { "ARCHIVED" } else { coordinator_status }
            .to_owned(),
        current_objective: coordinator
            .map_or_else(|| project.objective.clone(), |state| state.current_objective.clone()),
        next_step: coordinator.map(|state| state.next_step.clone()).unwrap_or_default(),
        blocker: coordinator.map_or_else(|| "None".to_owned(), |state| state.blocker.clone()),
        created_at: project.created_at,
        updated_at: coordinator.map_or(project.updated_at, |state| state.updated_at),
        role,
    }
}

/// `public_project` for a project after a kickoff (draft) update.
pub fn public_draft_project(project: &DraftProject, role: Option<Role>) -> ProjectResponse {
    let coordinator = &project.coordinator;
    ProjectResponse {
        slug: project.slug.clone(),
        name: project.name.clone(),
        mission: project.objective.clone(),
        success_criteria: project.success_criteria.clone(),
        context: project.context.clone(),
        runtime: project.runtime_engine.clone(),
        model: project.model.clone(),
        reasoning_effort: project.reasoning_effort.clone(),
        status: if project.status == "ARCHIVED" { "ARCHIVED" } else { &coordinator.status }
            .to_owned(),
        current_objective: coordinator.current_objective.clone(),
        next_step: coordinator.next_step.clone(),
        blocker: coordinator.blocker.clone(),
        created_at: project.created_at,
        updated_at: coordinator.updated_at,
        role,
    }
}

/// `public_status`.
pub fn public_status(status: &StatusView, role: Option<Role>) -> ProjectStatusResponse {
    ProjectStatusResponse {
        project: public_project(&status.challenge, role),
        knowledge: status.counts.clone(),
        active_work: status.running_experiments.iter().map(public_artifact_view).collect(),
        pending_guidance: status.pending_inbox,
        event_cursor: status.last_event_sequence,
    }
}

/// `public_artifact` for a listed artifact (no observations, no tags).
pub fn public_artifact_view(artifact: &ArtifactView) -> ArtifactResponse {
    ArtifactResponse {
        id: artifact.id.clone(),
        kind: artifact.kind.clone(),
        title: artifact.title.clone(),
        status: artifact.status.clone(),
        content: artifact.payload.clone(),
        hypothesis_id: artifact.hypothesis_id.clone(),
        experiment_id: artifact.experiment_id.clone(),
        created_at: artifact.created_at,
        updated_at: artifact.updated_at,
        observations: None,
        tags: Vec::new(),
    }
}

/// `public_artifact` for one artifact read on its own: experiments keep their observations.
pub fn public_artifact(detail: &ArtifactDetail) -> ArtifactResponse {
    ArtifactResponse {
        observations: detail.observations.clone(),
        ..public_artifact_view(&detail.artifact)
    }
}

/// `public_resource`.
pub fn public_resource(resource: &ResourceView) -> ResourceResponse {
    let variable = resource.resource_type == "VARIABLE";
    ResourceResponse {
        name: resource.name.clone(),
        resource_type: resource.resource_type.clone(),
        status: resource.status.clone(),
        value: if variable { resource.value.clone() } else { None },
        configured: if variable { None } else { resource.configured },
        created_at: resource.created_at,
        updated_at: resource.updated_at,
    }
}

/// `public_event`: `None` for coordination events; otherwise the event with private keys dropped,
/// worker actors shown as `Limina`, stored types aliased and internal commands masked.
pub fn public_event(event: &EventView) -> Option<EventResponse> {
    public_event_parts(
        i64::from(event.sequence),
        &event.event_type,
        &event.actor,
        event.artifact_id.as_deref(),
        &event.payload,
        event.created_at,
    )
}

/// [`public_event`] for an event as a run reports it.
pub fn public_run_event(event: &RunEvent) -> Option<EventResponse> {
    public_event_parts(
        i64::from(event.sequence),
        &event.event_type,
        &event.actor,
        event.artifact_id.as_deref(),
        &event.payload,
        event.created_at,
    )
}

fn public_event_parts(
    sequence: i64,
    event_type: &str,
    actor: &str,
    artifact_id: Option<&str>,
    raw_payload: &Value,
    created_at: Timestamp,
) -> Option<EventResponse> {
    if HIDDEN_EVENT_TYPES.contains(&event_type) {
        return None;
    }
    let empty = Map::new();
    let raw = raw_payload.as_object().unwrap_or(&empty);
    let renames_engine = event_type == "challenge.created";
    let mut payload = Map::new();
    let mut runtime = None;
    for (key, value) in raw {
        if PRIVATE_EVENT_KEYS.contains(&key.as_str()) {
            continue;
        }
        if renames_engine && key == "runtime_engine" {
            runtime = Some(value.clone());
            continue;
        }
        payload.insert(key.clone(), value.clone());
    }
    if let Some(runtime) = runtime {
        payload.insert("runtime".to_owned(), runtime);
    }
    if matches!(event_type, "inbox.message_sent" | "inbox.message_acknowledged") {
        payload.insert(
            "guidance_id".to_owned(),
            raw.get("message_id").cloned().unwrap_or(Value::Null),
        );
    }
    if TRANSCRIPT_EVENT_TYPES.contains(&event_type) {
        mask_internal_commands(&mut payload);
    }
    let actor = if actor.starts_with("limina:") { "Limina" } else { actor };
    let shown = EVENT_ALIASES
        .iter()
        .find(|(stored, _)| *stored == event_type)
        .map_or(event_type, |(_, alias)| alias);
    Some(EventResponse {
        sequence,
        event_type: shown.to_owned(),
        actor: actor.to_owned(),
        artifact_id: artifact_id.map(str::to_owned),
        detail: payload,
        created_at,
    })
}

/// The last line of defense: never publish internal knowledge command lines or their ids.
fn mask_internal_commands(payload: &mut Map<String, Value>) {
    if payload.get("input").is_some_and(is_internal_command) {
        payload.insert("input".to_owned(), Value::Null);
    }
    if let Some(Value::String(text)) = payload.get("text")
        && contains_internal_command(text)
    {
        let masked = redact_internal_commands(text);
        payload.insert("text".to_owned(), Value::String(masked));
    }
    if let Some(Value::String(summary)) = payload.get("summary")
        && contains_internal_command(summary)
    {
        let prose = payload
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| PROSE_KINDS.contains(&kind));
        let masked =
            if prose { redact_internal_commands(summary) } else { KNOWLEDGE_SUMMARY.to_owned() };
        payload.insert("summary".to_owned(), Value::String(masked));
    }
}

/// The guidance receipt for a stored message.
pub(crate) fn guidance_receipt(message: &MessageView, delivery: Delivery) -> GuidanceReceipt {
    GuidanceReceipt {
        id: message.id.clone(),
        delivery,
        kind: message.kind.clone(),
        accepted_at: message.created_at,
        status: message.status.clone(),
    }
}

/// The three kickoff templates the console offers.
pub fn kickoff_templates() -> Vec<KickoffTemplate> {
    let template =
        |id: &str, name: &str, description: &str, context: &str, criteria: &str| KickoffTemplate {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            defaults: TemplateDefaults {
                context: context.to_owned(),
                success_criteria: criteria.to_owned(),
            },
        };
    vec![
        template(
            "technical-investigation",
            "Technical investigation",
            "Resolve a technical uncertainty with a falsifiable evidence chain.",
            "State the current system, strongest baseline, constraints, and available data.",
            "A decision is supported by reproducible evidence and remaining uncertainty is \
             explicit.",
        ),
        template(
            "capability-evaluation",
            "Capability evaluation",
            "Test whether a capability is credible beyond a narrow benchmark slice.",
            "Describe the capability, evaluation set, baseline, and shortcut risks.",
            "The capability beats a fair baseline with guardrails and generalization checks.",
        ),
        template(
            "incident-learning",
            "Incident learning",
            "Investigate a failure mechanism and establish a durable prevention strategy.",
            "Include the incident timeline, symptoms, evidence, and known system changes.",
            "The mechanism is reproduced or bounded and a verified prevention is identified.",
        ),
    ]
}
