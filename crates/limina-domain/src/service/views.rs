//! The service's results. Each type serializes to the same JSON object (keys, key order and value
//! types) as the Python runtime's `_*_dict` converters, because results are stored as command
//! receipts, replayed later (including receipts the Python runtime wrote) and returned by the API.

use std::collections::BTreeMap;

use limina_core::Timestamp;
use limina_core::time::{serde_iso, serde_iso_opt};
use limina_store::models::{
    Artifact, Challenge, CoordinatorState, Event, InboxMessage, Observation, ProjectResource,
    WorkLease,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A project (challenge) with its coordinator state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChallengeView {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub objective: String,
    pub context: String,
    pub success_criteria: String,
    pub runtime_engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub status: String,
    pub version: i32,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
    pub coordinator: Option<CoordinatorView>,
}

impl ChallengeView {
    pub fn new(challenge: &Challenge, coordinator: Option<&CoordinatorState>) -> Self {
        Self {
            id: challenge.id.clone(),
            slug: challenge.slug.clone(),
            name: challenge.name.clone(),
            objective: challenge.objective.clone(),
            context: challenge.context.clone(),
            success_criteria: challenge.success_criteria.clone(),
            runtime_engine: challenge.runtime_engine.clone(),
            model: challenge.model.clone(),
            reasoning_effort: challenge.reasoning_effort.clone(),
            status: challenge.status.clone(),
            version: challenge.version,
            created_at: challenge.created_at,
            updated_at: challenge.updated_at,
            coordinator: coordinator.map(CoordinatorView::from),
        }
    }
}

/// The coordinator's durable working state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorView {
    pub status: String,
    pub current_objective: String,
    pub next_step: String,
    pub blocker: String,
    pub worker_id: Option<String>,
    pub continuation_id: Option<String>,
    pub inbox_cursor: i32,
    pub version: i32,
    #[serde(with = "serde_iso_opt")]
    pub heartbeat_at: Option<Timestamp>,
    #[serde(with = "serde_iso_opt")]
    pub wake_at: Option<Timestamp>,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl From<&CoordinatorState> for CoordinatorView {
    fn from(coordinator: &CoordinatorState) -> Self {
        Self {
            status: coordinator.status.clone(),
            current_objective: coordinator.current_objective.clone(),
            next_step: coordinator.next_step.clone(),
            blocker: coordinator.blocker.clone(),
            worker_id: coordinator.worker_id.clone(),
            continuation_id: coordinator.continuation_id.clone(),
            inbox_cursor: coordinator.inbox_cursor,
            version: coordinator.version,
            heartbeat_at: coordinator.heartbeat_at,
            wake_at: coordinator.wake_at,
            updated_at: coordinator.updated_at,
        }
    }
}

/// A research artifact (`H`, `E`, `F`, …). `payload` is the kind-specific content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactView {
    pub uid: String,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub payload: Value,
    pub hypothesis_id: Option<String>,
    pub experiment_id: Option<String>,
    pub version: i32,
    pub created_by: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl From<&Artifact> for ArtifactView {
    fn from(artifact: &Artifact) -> Self {
        Self {
            uid: artifact.uid.clone(),
            id: artifact.artifact_id.clone(),
            kind: artifact.kind.clone(),
            title: artifact.title.clone(),
            status: artifact.status.clone(),
            payload: artifact.payload.0.clone(),
            hypothesis_id: artifact.parent_hypothesis_id.clone(),
            experiment_id: artifact.parent_experiment_id.clone(),
            version: artifact.version,
            created_by: artifact.created_by.clone(),
            created_at: artifact.created_at,
            updated_at: artifact.updated_at,
        }
    }
}

/// One artifact as `get_artifact` returns it: experiments add their observations and, while it is
/// active, their lease.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactDetail {
    #[serde(flatten)]
    pub artifact: ArtifactView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<Vec<ObservationView>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<LeaseView>,
}

/// An append-only observation on a running experiment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationView {
    pub id: String,
    pub experiment_id: String,
    pub body: String,
    pub evidence_ref: Option<String>,
    pub actor: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
}

impl From<&Observation> for ObservationView {
    fn from(observation: &Observation) -> Self {
        Self {
            id: observation.id.clone(),
            experiment_id: observation.experiment_id.clone(),
            body: observation.body.clone(),
            evidence_ref: observation.evidence_ref.clone(),
            actor: observation.actor.clone(),
            created_at: observation.created_at,
        }
    }
}

/// A work lease. Also the result of `claim_coordinator` (whose scope is `coordinator`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseView {
    pub scope: String,
    pub owner: String,
    pub token: String,
    #[serde(with = "serde_iso")]
    pub expires_at: Timestamp,
    pub version: i32,
}

impl From<&WorkLease> for LeaseView {
    fn from(lease: &WorkLease) -> Self {
        Self {
            scope: lease.scope.clone(),
            owner: lease.owner.clone(),
            token: lease.token.clone(),
            expires_at: lease.expires_at,
            version: lease.version,
        }
    }
}

/// The lease part of an experiment claim (the scope is the experiment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseGrant {
    pub owner: String,
    pub token: String,
    #[serde(with = "serde_iso")]
    pub expires_at: Timestamp,
    pub version: i32,
}

impl From<&WorkLease> for LeaseGrant {
    fn from(lease: &WorkLease) -> Self {
        Self {
            owner: lease.owner.clone(),
            token: lease.token.clone(),
            expires_at: lease.expires_at,
            version: lease.version,
        }
    }
}

/// The result of `claim_experiment`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentClaim {
    pub artifact: ArtifactView,
    pub lease: LeaseGrant,
}

/// The result of `release_coordinator`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorRelease {
    pub scope: String,
    pub owner: String,
    pub released: bool,
}

/// Durable human guidance in a project's inbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageView {
    pub id: String,
    pub sequence: i32,
    pub kind: String,
    pub body: String,
    pub actor: String,
    pub status: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso_opt")]
    pub acknowledged_at: Option<Timestamp>,
}

impl From<&InboxMessage> for MessageView {
    fn from(message: &InboxMessage) -> Self {
        Self {
            id: message.id.clone(),
            sequence: message.sequence,
            kind: message.kind.clone(),
            body: message.body.clone(),
            actor: message.actor.clone(),
            status: message.status.clone(),
            created_at: message.created_at,
            acknowledged_at: message.acknowledged_at,
        }
    }
}

/// A project variable or secret. Secret values never leave the service: `value` is only set for
/// variables, and `configured` only for secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceView {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub value: Option<String>,
    pub configured: Option<bool>,
    pub status: String,
    pub created_by: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl From<&ProjectResource> for ResourceView {
    fn from(resource: &ProjectResource) -> Self {
        Self {
            id: resource.id.clone(),
            name: resource.name.clone(),
            resource_type: resource.resource_type.clone(),
            value: (resource.resource_type == "VARIABLE").then(|| resource.value.clone()).flatten(),
            configured: (resource.resource_type == "SECRET").then(|| {
                resource.secret_ciphertext.as_deref().is_some_and(|text| !text.is_empty())
            }),
            status: resource.status.clone(),
            created_by: resource.created_by.clone(),
            created_at: resource.created_at,
            updated_at: resource.updated_at,
        }
    }
}

/// One entry of a project's durable event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventView {
    pub sequence: i32,
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub actor: String,
    pub artifact_id: Option<String>,
    pub payload: Value,
    pub command_id: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
}

impl From<&Event> for EventView {
    fn from(event: &Event) -> Self {
        Self {
            sequence: event.sequence,
            id: event.id.clone(),
            event_type: event.event_type.clone(),
            actor: event.actor.clone(),
            artifact_id: event.artifact_id.clone(),
            payload: event.payload.0.clone(),
            command_id: event.command_id.clone(),
            created_at: event.created_at,
        }
    }
}

/// A project's status: its state, artifact counts by kind and status, running experiments, pending
/// guidance and the newest event sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusView {
    pub challenge: ChallengeView,
    pub counts: BTreeMap<String, BTreeMap<String, i64>>,
    pub running_experiments: Vec<ArtifactView>,
    pub pending_inbox: i64,
    pub last_event_sequence: i64,
}
