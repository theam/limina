//! Row types for every table, mirroring the schema in `schema/`.
//!
//! Field types follow the crate conventions: `INTEGER` → `i32`, `BIGINT` → `i64`, timestamps →
//! [`Timestamp`], JSON → [`Json<Value>`]. Select columns by name (`SELECT *` is fine: every column
//! of a table is listed here).

use limina_core::Timestamp;
use serde_json::Value;
use sqlx::FromRow;
use sqlx::types::Json;

#[derive(Debug, Clone, FromRow)]
pub struct Challenge {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub objective: String,
    pub context: String,
    pub success_criteria: String,
    pub runtime_engine: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub status: String,
    pub version: i32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProjectResource {
    pub id: String,
    pub challenge_id: String,
    pub name: String,
    pub resource_type: String,
    pub value: Option<String>,
    pub secret_ciphertext: Option<String>,
    pub status: String,
    pub created_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProjectEngineAccount {
    pub challenge_id: String,
    pub engine: String,
    pub method: String,
    pub label: Option<String>,
    pub secret_ciphertext: Option<String>,
    pub connected_by: String,
    pub connected_by_subject: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProjectMember {
    pub challenge_id: String,
    pub subject: String,
    pub role: String,
    pub display_name: String,
    pub email: Option<String>,
    pub created_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A pending membership: a project shared with an email nobody has signed in with yet.
#[derive(Debug, Clone, FromRow)]
pub struct ProjectInvitation {
    pub challenge_id: String,
    /// Trimmed and lower-cased.
    pub email: String,
    pub role: String,
    /// Only a session in this organization may accept the invitation.
    pub organization_id: Option<String>,
    /// The subject of the owner who shared the project.
    pub created_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct LiveTicket {
    pub token_hash: String,
    pub challenge_id: String,
    pub subject: String,
    pub display_name: String,
    pub role: String,
    pub instance_admin: bool,
    pub expires_at: Timestamp,
    pub used_at: Option<Timestamp>,
    pub created_at: Timestamp,
    /// Issued to a principal that acts on every project (a trusted local operator).
    pub project_admin: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProjectSource {
    pub id: String,
    pub challenge_id: String,
    pub name: String,
    pub source_type: String,
    pub uri: String,
    pub media_type: Option<String>,
    pub metadata_json: Json<Value>,
    pub status: String,
    pub created_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct CoordinatorState {
    pub challenge_id: String,
    pub status: String,
    pub current_objective: String,
    pub next_step: String,
    pub blocker: String,
    pub worker_id: Option<String>,
    pub continuation_id: Option<String>,
    pub inbox_cursor: i32,
    pub version: i32,
    pub heartbeat_at: Option<Timestamp>,
    pub wake_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct IdCounter {
    pub challenge_id: String,
    pub kind: String,
    pub next_value: i32,
}

#[derive(Debug, Clone, FromRow)]
pub struct Artifact {
    pub uid: String,
    pub challenge_id: String,
    pub artifact_id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub payload: Json<Value>,
    pub parent_hypothesis_id: Option<String>,
    pub parent_experiment_id: Option<String>,
    pub version: i32,
    pub created_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct ArtifactRevision {
    pub id: i32,
    pub artifact_uid: String,
    pub version: i32,
    pub status: String,
    pub title: String,
    pub payload: Json<Value>,
    pub actor: String,
    pub command_id: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct KnowledgeRelation {
    pub id: String,
    pub challenge_id: String,
    pub source_artifact_id: String,
    pub target_artifact_id: String,
    pub relation_type: String,
    pub description: String,
    pub created_by: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct ArtifactComment {
    pub id: String,
    pub challenge_id: String,
    pub artifact_id: String,
    pub body: String,
    pub actor: String,
    pub command_id: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct ArtifactTag {
    pub challenge_id: String,
    pub artifact_id: String,
    pub tag: String,
    pub created_by: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct SavedKnowledgeView {
    pub id: String,
    pub challenge_id: String,
    pub name: String,
    pub query: Json<Value>,
    pub created_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct Observation {
    pub id: String,
    pub challenge_id: String,
    pub experiment_id: String,
    pub body: String,
    pub evidence_ref: Option<String>,
    pub actor: String,
    pub command_id: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct WorkLease {
    pub challenge_id: String,
    pub scope: String,
    pub owner: String,
    pub token: String,
    pub expires_at: Timestamp,
    pub version: i32,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct InboxMessage {
    pub sequence: i32,
    pub id: String,
    pub challenge_id: String,
    pub kind: String,
    pub body: String,
    pub actor: String,
    pub status: String,
    pub command_id: String,
    pub created_at: Timestamp,
    pub acknowledged_at: Option<Timestamp>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Event {
    pub sequence: i32,
    pub id: String,
    pub challenge_id: String,
    pub event_type: String,
    pub actor: String,
    pub artifact_id: Option<String>,
    pub payload: Json<Value>,
    pub command_id: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct RuntimeRun {
    pub id: String,
    pub challenge_id: String,
    pub runtime_engine: String,
    pub model: Option<String>,
    pub status: String,
    pub summary: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub turn_id: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cost_microusd: Option<i64>,
    pub usage_source: Option<String>,
    pub cost_source: Option<String>,
    pub tool_calls: i32,
    pub retry_count: i32,
    pub started_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CommandReceipt {
    pub command_id: String,
    pub command_type: String,
    pub actor: String,
    pub result: Json<Value>,
    pub created_at: Timestamp,
}

/// A person's remote MCP server (`mcp_connections`). Credentials live only in
/// `secret_ciphertext`, encrypted with the runtime's secret cipher.
#[derive(Debug, Clone, FromRow)]
pub struct McpConnection {
    pub id: String,
    pub owner_subject: String,
    pub owner_name: String,
    pub name: String,
    pub server_url: String,
    pub auth_kind: String,
    pub header_name: Option<String>,
    pub status: String,
    pub last_error: Option<String>,
    pub secret_ciphertext: Option<String>,
    pub scopes: Option<String>,
    pub token_expires_at: Option<Timestamp>,
    pub connected_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A connection its owner enabled for one project (`project_mcp_connections`).
#[derive(Debug, Clone, FromRow)]
pub struct ProjectMcpConnection {
    pub challenge_id: String,
    pub connection_id: String,
    pub server_key: String,
    pub granted_as: String,
    pub enabled_by: String,
    pub created_at: Timestamp,
}

/// A pending OAuth authorization of a connection (`mcp_authorization_flows`), keyed by the
/// SHA-256 of its `state`.
#[derive(Debug, Clone, FromRow)]
pub struct McpAuthorizationFlow {
    pub state_hash: String,
    pub connection_id: String,
    pub subject: String,
    pub browser_hash: String,
    pub redirect_uri: String,
    pub return_to: String,
    pub payload_ciphertext: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProjectMetric {
    pub challenge_id: String,
    pub metric_id: String,
    pub payload: Json<Value>,
    pub version: i32,
    pub updated_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
