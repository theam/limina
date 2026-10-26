//! The public response shapes, as the OpenAPI document describes them.
//!
//! The responses themselves are built by [`crate::app`] (and the domain types it returns); these
//! types restate their wire format with the names and constraints of the Python API's
//! `schemas.py`, so `/openapi.json` keeps the same components. They also deserialize with
//! unknown fields denied, which lets tests (and typed clients) check a live response against the
//! documented shape.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

// ─── Enumerations ─────────────────────────────────────────────────────────────

// A runtime engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
pub enum Engine {
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "claude-code")]
    ClaudeCode,
}

// A project role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
pub enum RuntimeOwner {
    #[serde(rename = "limina")]
    Limina,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Local,
    Oidc,
    Workos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
pub enum CodexEngine {
    #[serde(rename = "codex")]
    Codex,
}

// How the runtime's Codex account is signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CodexMethod {
    Chatgpt,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum LoginStatus {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
pub enum SearchBackend {
    #[serde(rename = "postgresql-fts")]
    PostgresqlFts,
    #[serde(rename = "portable-substring")]
    PortableSubstring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum ResourceType {
    Variable,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Delivery {
    Live,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum SourceType {
    Url,
    Connector,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Info,
}

// ─── Errors, health and identity ──────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorDetail {
    /// A stable machine-readable code (`not_found`, `invalid_request`, …).
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Map<String, Value>,
}

/// Every error the API returns.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HealthResponse {
    pub ok: bool,
    pub version: String,
    #[schema(inline)]
    pub runtime_owner: RuntimeOwner,
    #[schema(inline)]
    pub auth_mode: AuthMode,
    #[schema(inline)]
    pub runtimes: Vec<Engine>,
    pub interfaces: BTreeMap<String, String>,
}

/// The authenticated caller.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeResponse {
    pub subject: String,
    pub display_name: String,
    #[schema(required = true)]
    pub email: Option<String>,
    pub instance_admin: bool,
    pub auth_mode: String,
}

// ─── Engines and accounts ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineAvailability {
    #[schema(inline)]
    pub engine: Engine,
    pub label: String,
    pub installed: bool,
    pub credentials: bool,
    pub detail: String,
    #[schema(required = true)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CodexAuthStatus {
    #[schema(inline)]
    pub engine: CodexEngine,
    pub configured_mode: String,
    pub configured: bool,
    #[schema(inline, required = true)]
    pub active_method: Option<CodexMethod>,
    #[schema(required = true)]
    pub account_email: Option<String>,
    #[schema(required = true)]
    pub account_plan: Option<String>,
    pub source: String,
    #[schema(required = true)]
    pub error: Option<String>,
    pub single_runtime_node: bool,
}

/// A Codex device-code sign-in.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CodexDeviceLogin {
    pub login_id: String,
    #[schema(inline)]
    pub status: LoginStatus,
    pub verification_url: String,
    #[schema(required = true)]
    pub user_code: Option<String>,
    #[schema(required = true)]
    pub error: Option<String>,
    pub created_at: String,
    #[schema(required = true)]
    pub completed_at: Option<String>,
}

/// A device login for `chatgpt`, the account status after a server-credential login.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum CodexLoginResponse {
    Device(CodexDeviceLogin),
    Status(CodexAuthStatus),
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineAccount {
    #[schema(inline)]
    pub engine: Engine,
    pub method: String,
    #[schema(required = true)]
    pub label: Option<String>,
    #[schema(required = true)]
    pub connected_by: Option<String>,
    #[schema(required = true)]
    pub connected_at: Option<String>,
}

/// Which account a project's turns use for one engine.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectEngineAccountResponse {
    #[schema(inline)]
    pub engine: Engine,
    pub label: String,
    #[schema(required = true)]
    pub account: Option<EngineAccount>,
    pub runtime_available: bool,
    pub runtime_detail: String,
}

// ─── Projects ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectResponse {
    pub slug: String,
    pub name: String,
    pub mission: String,
    pub success_criteria: String,
    pub context: String,
    #[schema(inline)]
    pub runtime: Engine,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub status: String,
    pub current_objective: String,
    pub next_step: String,
    pub blocker: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    #[schema(inline)]
    pub role: Option<Role>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectPage {
    pub items: Vec<ProjectResponse>,
    #[schema(required = true)]
    pub next_cursor: Option<String>,
    pub total: i64,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationResponse {
    pub id: String,
    pub experiment_id: String,
    pub body: String,
    #[schema(required = true)]
    pub evidence_ref: Option<String>,
    pub actor: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactResponse {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub content: Map<String, Value>,
    #[serde(default)]
    pub hypothesis_id: Option<String>,
    #[serde(default)]
    pub experiment_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub observations: Option<Vec<ObservationResponse>>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePage {
    pub items: Vec<ArtifactResponse>,
    #[schema(required = true)]
    pub next_cursor: Option<String>,
    pub total: i64,
    #[schema(inline)]
    pub search_backend: SearchBackend,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EventResponse {
    pub sequence: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub actor: String,
    #[schema(required = true)]
    pub artifact_id: Option<String>,
    pub detail: Map<String, Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EventPage {
    pub events: Vec<EventResponse>,
    /// The last sequence scanned (hidden events included); pass it as `after` to follow.
    pub cursor: i64,
    pub has_earlier: bool,
}

/// A project variable (with its `value`) or secret (only whether it is `configured`).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceResponse {
    pub name: String,
    #[serde(rename = "type")]
    #[schema(inline)]
    pub resource_type: ResourceType,
    pub status: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub configured: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectStatusResponse {
    pub project: ProjectResponse,
    /// Artifact counts by kind, then status.
    pub knowledge: BTreeMap<String, BTreeMap<String, i64>>,
    pub active_work: Vec<ArtifactResponse>,
    pub pending_guidance: i64,
    pub event_cursor: i64,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewResponse {
    pub project: ProjectResponse,
    pub knowledge: BTreeMap<String, BTreeMap<String, i64>>,
    pub active_work: Vec<ArtifactResponse>,
    pub pending_guidance: i64,
    pub event_cursor: i64,
    pub resources: Vec<ResourceResponse>,
    pub hypotheses: Vec<ArtifactResponse>,
    pub experiments: Vec<ArtifactResponse>,
    pub findings: Vec<ArtifactResponse>,
    pub recent_activity: Vec<EventResponse>,
    #[schema(required = true)]
    pub knowledge_cursor: Option<String>,
    pub knowledge_total: i64,
}

/// The project's knowledge base as Markdown files.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotResponse {
    pub files: BTreeMap<String, String>,
}

// ─── Guidance ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GuidanceReceipt {
    pub id: String,
    #[schema(inline)]
    pub delivery: Delivery,
    pub kind: String,
    pub accepted_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GuidanceResponse {
    pub id: String,
    pub sequence: i64,
    pub kind: String,
    pub body: String,
    pub actor: String,
    pub status: String,
    pub created_at: String,
    #[schema(required = true)]
    pub acknowledged_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GuidancePage {
    pub items: Vec<GuidanceResponse>,
    #[schema(required = true)]
    pub next_cursor: Option<String>,
    pub total: i64,
}

// ─── Members and live tickets ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MemberResponse {
    pub subject: String,
    pub display_name: String,
    #[schema(required = true)]
    pub email: Option<String>,
    #[schema(inline)]
    pub role: Role,
    pub created_at: String,
    pub updated_at: String,
}

/// One person's access to a project: a member, or a pending invitation for an email.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareResponse {
    /// The member's identity-provider user ID; `null` while the invitation is pending.
    #[schema(required = true)]
    pub subject: Option<String>,
    /// The member's name; the email while the invitation is pending.
    pub display_name: String,
    #[schema(required = true)]
    pub email: Option<String>,
    #[schema(inline)]
    pub role: Role,
    /// Waiting for the first sign-in with this verified email.
    pub pending: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// An owner of a project shared with the caller.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedProjectOwner {
    pub subject: String,
    pub display_name: String,
    #[schema(required = true)]
    pub email: Option<String>,
}

/// A project someone else shared with the caller.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedProjectResponse {
    pub slug: String,
    /// The caller's role.
    #[schema(inline)]
    pub role: Role,
    pub owners: Vec<SharedProjectOwner>,
    /// When the caller became a member.
    pub shared_at: String,
}

/// A single-use ticket for `/v1/projects/{slug}/live` (subprotocol `limina.ticket.<ticket>`).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LiveTicketResponse {
    pub ticket: String,
    pub expires_at: String,
}

// ─── Knowledge ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationResponse {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    #[serde(rename = "type")]
    pub relation_type: String,
    pub description: String,
    pub derived: bool,
    pub created_by: String,
    #[schema(required = true)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeGraphResponse {
    pub nodes: Vec<ArtifactResponse>,
    pub edges: Vec<RelationResponse>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionResponse {
    pub version: i64,
    pub status: String,
    pub title: String,
    pub content: Map<String, Value>,
    pub actor: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentResponse {
    pub id: String,
    pub artifact_id: String,
    pub body: String,
    pub actor: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TagResponse {
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedViewResponse {
    pub id: String,
    pub name: String,
    pub query: Map<String, Value>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceResponse {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    #[schema(inline)]
    pub source_type: SourceType,
    pub uri: String,
    #[schema(required = true)]
    pub media_type: Option<String>,
    pub metadata: Map<String, Value>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

// ─── Kickoff ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateDefaults {
    pub context: String,
    pub success_criteria: String,
}

/// A starting point for a new project's kickoff fields.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KickoffTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub defaults: TemplateDefaults,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightCheck {
    pub name: String,
    #[schema(inline)]
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightResponse {
    pub ready: bool,
    pub checks: Vec<PreflightCheck>,
}

// ─── Runs and analytics ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunUsage {
    #[schema(required = true)]
    pub input_tokens: Option<i64>,
    #[schema(required = true)]
    pub output_tokens: Option<i64>,
    #[schema(required = true)]
    pub cached_input_tokens: Option<i64>,
    #[schema(required = true)]
    pub reasoning_output_tokens: Option<i64>,
    #[schema(required = true)]
    pub total_tokens: Option<i64>,
    #[schema(required = true)]
    pub cost_microusd: Option<i64>,
    #[schema(required = true)]
    pub usage_source: Option<String>,
    #[schema(required = true)]
    pub cost_source: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunError {
    #[schema(required = true)]
    pub code: Option<String>,
    #[schema(required = true)]
    pub message: Option<String>,
}

/// One managed turn.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRunResponse {
    pub id: String,
    #[schema(inline)]
    pub runtime: Engine,
    #[schema(required = true)]
    pub model: Option<String>,
    pub status: String,
    pub summary: String,
    #[schema(required = true)]
    pub error: Option<RunError>,
    pub usage: RunUsage,
    pub tool_calls: i64,
    pub retry_count: i64,
    pub started_at: String,
    #[schema(required = true)]
    pub completed_at: Option<String>,
    #[schema(required = true)]
    pub duration_ms: Option<i64>,
}

/// One managed turn and the public events it recorded.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRunDetail {
    pub id: String,
    #[schema(inline)]
    pub runtime: Engine,
    #[schema(required = true)]
    pub model: Option<String>,
    pub status: String,
    pub summary: String,
    #[schema(required = true)]
    pub error: Option<RunError>,
    pub usage: RunUsage,
    pub tool_calls: i64,
    pub retry_count: i64,
    pub started_at: String,
    #[schema(required = true)]
    pub completed_at: Option<String>,
    #[schema(required = true)]
    pub duration_ms: Option<i64>,
    pub events: Vec<EventResponse>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRunPage {
    pub items: Vec<RuntimeRunResponse>,
    #[schema(required = true)]
    pub next_cursor: Option<String>,
    pub total: i64,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyticsWindow {
    pub days: i64,
    pub since: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunAnalytics {
    pub total: i64,
    pub by_status: BTreeMap<String, i64>,
    #[schema(required = true)]
    pub success_rate: Option<f64>,
    #[schema(required = true)]
    pub average_duration_ms: Option<i64>,
    #[schema(required = true)]
    pub p95_duration_ms: Option<i64>,
    #[schema(required = true)]
    pub input_tokens: Option<i64>,
    #[schema(required = true)]
    pub output_tokens: Option<i64>,
    #[schema(required = true)]
    pub reasoning_output_tokens: Option<i64>,
    #[schema(required = true)]
    pub total_tokens: Option<i64>,
    #[schema(required = true)]
    pub cost_microusd: Option<i64>,
    pub usage_sources: Vec<String>,
    pub cost_sources: Vec<String>,
    pub tool_calls: i64,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeAnalytics {
    pub created: i64,
    pub by_kind: BTreeMap<String, i64>,
    pub by_status: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GuidanceAnalytics {
    pub total: i64,
    pub pending: i64,
    #[schema(required = true)]
    pub average_acknowledgement_seconds: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyticsPoint {
    pub date: String,
    pub runs: i64,
    pub completed_runs: i64,
    pub failed_runs: i64,
    pub artifacts: i64,
    pub guidance: i64,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyticsResponse {
    pub window: AnalyticsWindow,
    pub runs: RunAnalytics,
    pub knowledge: KnowledgeAnalytics,
    pub guidance: GuidanceAnalytics,
    pub timeseries: Vec<AnalyticsPoint>,
}

// ─── Shared responses and parameters ──────────────────────────────────────────

/// The errors every authenticated route may answer with.
#[derive(utoipa::IntoResponses)]
pub enum PublicErrors {
    #[response(status = 401, description = "Unauthorized")]
    Unauthorized(ErrorResponse),
    #[response(status = 403, description = "Forbidden")]
    Forbidden(ErrorResponse),
    #[response(status = 404, description = "Not Found")]
    NotFound(ErrorResponse),
    #[response(status = 409, description = "Conflict")]
    Conflict(ErrorResponse),
    #[response(status = 422, description = "Unprocessable Content")]
    Unprocessable(ErrorResponse),
    #[response(status = 429, description = "Too Many Requests")]
    TooManyRequests(ErrorResponse),
}

/// `X-Limina-Actor`: the display name static-token callers act as.
#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Header)]
pub struct ActorHeader {
    #[param(rename = "x-limina-actor", max_length = 200)]
    pub actor: Option<String>,
}

/// `Idempotency-Key`: retries with the same key replay the first result.
#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Header)]
pub struct CommandHeader {
    #[param(rename = "idempotency-key", min_length = 1, max_length = 55)]
    pub idempotency_key: String,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProjectListParams {
    #[param(default = false, required = false)]
    pub include_archived: bool,
    #[param(nullable)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 200, default = 100, required = false)]
    pub limit: i64,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatusPageParams {
    #[param(nullable)]
    pub status: Option<String>,
    #[param(nullable)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 200, default = 50, required = false)]
    pub limit: i64,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageParams {
    #[param(nullable)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 200, default = 50, required = false)]
    pub limit: i64,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KnowledgeParams {
    #[param(max_length = 500, nullable)]
    pub query: Option<String>,
    #[param(nullable)]
    pub kind: Option<String>,
    #[param(nullable)]
    pub status: Option<String>,
    #[param(nullable)]
    pub tag: Option<String>,
    #[param(max_length = 200, nullable)]
    pub parent: Option<String>,
    #[param(nullable)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 200, default = 50, required = false)]
    pub limit: i64,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventParams {
    #[param(minimum = 0, default = 0, required = false)]
    pub after: i64,
    /// Return the newest events with a lower sequence, oldest first.
    #[param(minimum = 1, nullable)]
    pub before: Option<i64>,
    #[param(minimum = 1, maximum = 1000, default = 200, required = false)]
    pub limit: i64,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AnalyticsParams {
    #[param(minimum = 1, maximum = 365, default = 30, required = false)]
    pub days: i64,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MemberParams {
    #[param(min_length = 1, max_length = 300)]
    pub subject: String,
}

#[derive(utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ShareParams {
    /// The email of the pending invitation.
    #[param(min_length = 3, max_length = 320)]
    pub email: String,
}

/// Provider model metadata used by onboarding and kickoff. Account access is checked on use.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub resolved_model: Option<String>,
    pub reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ModelCatalog {
    pub default_model: Option<String>,
    pub models: Vec<ModelOption>,
}
