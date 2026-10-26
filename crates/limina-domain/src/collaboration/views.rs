//! Response and input shapes of the collaboration service. Each response serializes with the
//! keys, order and value types of the Python dict it replaces.

use limina_core::Timestamp;
use limina_core::time::{serde_iso, serde_iso_opt};
use limina_store::models::{
    Challenge, CoordinatorState, Event, InboxMessage, ProjectMember, RuntimeRun,
};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::Role;
use crate::usage::RuntimeUsage;

/// A project member (`_member_dict`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub subject: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: Role,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl Member {
    pub(crate) fn from_row(row: ProjectMember, role: Role) -> Self {
        Self {
            subject: row.subject,
            display_name: row.display_name,
            email: row.email,
            role,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Adds or updates a project member (`PUT /members`). `role` is validated case-insensitively.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberChange {
    pub subject: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub role: String,
}

/// Kickoff fields to change before a project first starts; `None` keeps the current value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftChanges {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mission: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub success_criteria: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    /// An empty string restores the runtime default; absent/null keeps the saved choice.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

/// A project after a draft update (`CollaborationService._challenge_dict`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftProject {
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
    pub coordinator: DraftCoordinator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftCoordinator {
    pub status: String,
    pub current_objective: String,
    pub next_step: String,
    pub blocker: String,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl DraftProject {
    pub(crate) fn from_rows(challenge: Challenge, coordinator: CoordinatorState) -> Self {
        Self {
            id: challenge.id,
            slug: challenge.slug,
            name: challenge.name,
            objective: challenge.objective,
            context: challenge.context,
            success_criteria: challenge.success_criteria,
            runtime_engine: challenge.runtime_engine,
            model: challenge.model,
            reasoning_effort: challenge.reasoning_effort,
            status: challenge.status,
            version: challenge.version,
            created_at: challenge.created_at,
            updated_at: challenge.updated_at,
            coordinator: DraftCoordinator {
                status: coordinator.status,
                current_objective: coordinator.current_objective,
                next_step: coordinator.next_step,
                blocker: coordinator.blocker,
                updated_at: coordinator.updated_at,
            },
        }
    }
}

/// What preflight needs to know about the project's engine (from the supervisor's engine status).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineReadiness {
    /// The user-facing engine name (`Codex`, `Claude`).
    pub label: String,
    pub installed: bool,
    pub credentials: bool,
    pub detail: String,
}

/// Kickoff readiness: `{"ready", "checks"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preflight {
    pub ready: bool,
    pub checks: Vec<PreflightCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CheckStatus {
    Pass,
    Fail,
    Warn,
    Info,
}

/// A single-use browser ticket for the live channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTicketGrant {
    pub ticket: String,
    #[serde(with = "serde_iso")]
    pub expires_at: Timestamp,
}

/// One guidance message in the project's history (`_guidance_dict`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidanceItem {
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

impl From<InboxMessage> for GuidanceItem {
    fn from(row: InboxMessage) -> Self {
        Self {
            id: row.id,
            sequence: row.sequence,
            kind: row.kind,
            body: row.body,
            actor: row.actor,
            status: row.status,
            created_at: row.created_at,
            acknowledged_at: row.acknowledged_at,
        }
    }
}

/// How a runtime turn ended, recorded by [`super::CollaborationService::finish_run`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub status: String,
    pub summary: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub turn_id: Option<String>,
    pub usage: RuntimeUsage,
}

/// One runtime turn (`_run_dict`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub runtime: String,
    pub model: Option<String>,
    pub status: String,
    pub summary: String,
    pub error: Option<RunError>,
    pub usage: RunUsage,
    pub tool_calls: i32,
    pub retry_count: i32,
    #[serde(with = "serde_iso")]
    pub started_at: Timestamp,
    #[serde(with = "serde_iso_opt")]
    pub completed_at: Option<Timestamp>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunError {
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cost_microusd: Option<i64>,
    pub usage_source: Option<String>,
    pub cost_source: Option<String>,
}

impl From<RuntimeRun> for Run {
    fn from(row: RuntimeRun) -> Self {
        let has_error = row.error_code.as_deref().is_some_and(|code| !code.is_empty())
            || row.error_message.as_deref().is_some_and(|message| !message.is_empty());
        Self {
            id: row.id,
            runtime: row.runtime_engine,
            model: row.model,
            status: row.status,
            summary: row.summary,
            error: has_error
                .then_some(RunError { code: row.error_code, message: row.error_message }),
            usage: RunUsage {
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cached_input_tokens: row.cached_input_tokens,
                reasoning_output_tokens: row.reasoning_output_tokens,
                total_tokens: row.total_tokens,
                cost_microusd: row.cost_microusd,
                usage_source: row.usage_source,
                cost_source: row.cost_source,
            },
            tool_calls: row.tool_calls,
            retry_count: row.retry_count,
            started_at: row.started_at,
            completed_at: row.completed_at,
            duration_ms: row.duration_ms,
        }
    }
}

/// A run with the events it recorded: the run's fields followed by `"events"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunDetail {
    #[serde(flatten)]
    pub run: Run,
    pub events: Vec<RunEvent>,
}

/// A stored event as runs report it (`_event_dict`); worker actors read `Limina`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEvent {
    pub sequence: i32,
    #[serde(rename = "type")]
    pub event_type: String,
    pub actor: String,
    pub artifact_id: Option<String>,
    pub payload: Value,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
}

impl From<Event> for RunEvent {
    fn from(row: Event) -> Self {
        let actor = if row.actor.starts_with("limina:") { "Limina".to_owned() } else { row.actor };
        Self {
            sequence: row.sequence,
            event_type: row.event_type,
            actor,
            artifact_id: row.artifact_id,
            payload: row.payload.0,
            created_at: row.created_at,
        }
    }
}

/// Project analytics over a trailing window of days.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analytics {
    pub window: AnalyticsWindow,
    pub runs: RunAnalytics,
    pub knowledge: KnowledgeAnalytics,
    pub guidance: GuidanceAnalytics,
    pub timeseries: Vec<DailyActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsWindow {
    /// The requested number of days, as given (the query itself is clamped to 1–365).
    pub days: i64,
    #[serde(with = "serde_iso")]
    pub since: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunAnalytics {
    pub total: i64,
    pub by_status: Counts,
    pub success_rate: Option<f64>,
    pub average_duration_ms: Option<i64>,
    pub p95_duration_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cost_microusd: Option<i64>,
    pub usage_sources: Vec<String>,
    pub cost_sources: Vec<String>,
    pub tool_calls: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeAnalytics {
    pub created: i64,
    pub by_kind: Counts,
    pub by_status: Counts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidanceAnalytics {
    pub total: i64,
    pub pending: i64,
    pub average_acknowledgement_seconds: Option<i64>,
}

/// Activity on one UTC day (`YYYY-MM-DD`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyActivity {
    pub date: String,
    pub runs: i64,
    pub completed_runs: i64,
    pub failed_runs: i64,
    pub artifacts: i64,
    pub guidance: i64,
}

/// Counts by key in first-seen order, serialized as a JSON object (Python's `Counter`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts(pub Vec<(String, i64)>);

impl Counts {
    pub fn add(&mut self, key: &str) {
        match self.0.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, count)) => *count += 1,
            None => self.0.push((key.to_owned(), 1)),
        }
    }

    pub fn get(&self, key: &str) -> Option<i64> {
        self.0.iter().find(|(existing, _)| existing == key).map(|(_, count)| *count)
    }
}

impl FromIterator<String> for Counts {
    fn from_iter<I: IntoIterator<Item = String>>(keys: I) -> Self {
        let mut counts = Self::default();
        for key in keys {
            counts.add(&key);
        }
        counts
    }
}

impl Serialize for Counts {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, count) in &self.0 {
            map.serialize_entry(key, count)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Counts {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CountsVisitor;
        impl<'de> Visitor<'de> for CountsVisitor {
            type Value = Counts;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object of counts")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Counts, A::Error> {
                let mut counts = Vec::new();
                while let Some(entry) = access.next_entry::<String, i64>()? {
                    counts.push(entry);
                }
                Ok(Counts(counts))
            }
        }
        deserializer.deserialize_map(CountsVisitor)
    }
}
