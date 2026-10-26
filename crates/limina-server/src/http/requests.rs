//! Request bodies: what each route accepts, validated like the Python API's pydantic models
//! (`schemas.py`, `extra="forbid"`) and documented in the OpenAPI document.
//!
//! Each `parse` reads the fields in declaration order, so errors come out in the same order as
//! pydantic's. The schema attributes restate the same constraints for `/openapi.json`.

use limina_domain::collaboration::{DraftChanges, MemberChange};
use limina_domain::knowledge::{NewRelation, SourceInput};
use limina_domain::research::{
    ExperimentOutcome, HypothesisDecision, NewExperiment, NewFinding, NewHypothesis,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

use super::schemas;
use super::validate::{Body, Errors, Fields, Int, Text};

pub const ENGINES: [&str; 2] = ["codex", "claude-code"];
pub const ROLES: [&str; 3] = ["OWNER", "EDITOR", "VIEWER"];
pub const STEERING_KINDS: [&str; 5] = ["STEER", "ANSWER", "APPROVAL", "BLOCKER", "INTERRUPT"];
pub const SOURCE_TYPES: [&str; 2] = ["URL", "CONNECTOR"];
pub const CODEX_LOGIN_METHODS: [&str; 3] = ["chatgpt", "api-key", "access-token"];
/// The longest variable or secret value.
pub const RESOURCE_VALUE_LIMIT: usize = 32_768;

// The source types a client may register (uploads have their own route).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum RequestSourceType {
    Url,
    Connector,
}

/// A literal choice with a default, as pydantic documents `Literal[…] = "…"`.
fn literal_schema(values: &[&str], default: &str) -> utoipa::openapi::Object {
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .enum_values(Some(values.iter().copied()))
        .default(Some(Value::from(default)))
        .build()
}

fn runtime_schema() -> utoipa::openapi::Object {
    literal_schema(&ENGINES, "codex")
}

fn steering_kind_schema() -> utoipa::openapi::Object {
    literal_schema(&STEERING_KINDS, "STEER")
}

fn codex_method_schema() -> utoipa::openapi::Object {
    literal_schema(&CODEX_LOGIN_METHODS, "chatgpt")
}

// ─── Projects ─────────────────────────────────────────────────────────────────

/// A new project. Execution begins with the `start` action.
#[derive(Debug, Clone, ToSchema)]
pub struct CreateProjectRequest {
    #[schema(min_length = 1, max_length = 120)]
    pub slug: String,
    #[schema(min_length = 1, max_length = 240)]
    pub name: String,
    /// The mission.
    #[schema(min_length = 1)]
    pub objective: String,
    #[schema(min_length = 1)]
    pub success_criteria: String,
    #[schema(default = "", required = false)]
    pub context: String,
    #[schema(schema_with = runtime_schema, required = false)]
    pub runtime: String,
    #[schema(max_length = 200)]
    pub model: Option<String>,
    #[schema(max_length = 64)]
    pub reasoning_effort: Option<String>,
}

impl CreateProjectRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let slug = fields.text("slug", Text::between(1, 120));
        let name = fields.text("name", Text::between(1, 240));
        let objective = fields.text("objective", Text::min(1));
        let success_criteria = fields.text("success_criteria", Text::min(1));
        let context = fields.text_or("context", "", Text::ANY);
        let runtime = fields.literal("runtime", &ENGINES, Some("codex"));
        let model = fields.nullable_text("model", Text::max(200));
        let reasoning_effort = fields.nullable_text("reasoning_effort", Text::max(64));
        fields.finish();
        Some(Self {
            slug: slug?,
            name: name?,
            objective: objective?,
            success_criteria: success_criteria?,
            context: context?,
            runtime: runtime?,
            model: model?,
            reasoning_effort: reasoning_effort?,
        })
    }
}

/// Kickoff fields to change before the project first starts; absent or `null` fields keep their
/// value.
#[derive(Debug, Clone, ToSchema)]
pub struct UpdateProjectRequest {
    #[schema(min_length = 1, max_length = 240)]
    pub name: Option<String>,
    #[schema(min_length = 1)]
    pub mission: Option<String>,
    #[schema(min_length = 1)]
    pub success_criteria: Option<String>,
    pub context: Option<String>,
    #[schema(value_type = Option<schemas::Engine>, inline)]
    pub runtime: Option<String>,
    /// Empty restores the runtime default; absent/null keeps the saved choice.
    #[schema(max_length = 200)]
    pub model: Option<String>,
    #[schema(max_length = 64)]
    pub reasoning_effort: Option<String>,
}

impl UpdateProjectRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<DraftChanges> {
        let mut fields = Fields::open(body, errors)?;
        let name = fields.nullable_text("name", Text::between(1, 240));
        let mission = fields.nullable_text("mission", Text::min(1));
        let success_criteria = fields.nullable_text("success_criteria", Text::min(1));
        let context = fields.nullable_text("context", Text::ANY);
        let runtime = fields.nullable_literal("runtime", &ENGINES);
        let model = fields.nullable_text("model", Text::max(200));
        let reasoning_effort = fields.nullable_text("reasoning_effort", Text::max(64));
        fields.finish();
        Some(DraftChanges {
            name: name?,
            mission: mission?,
            context: context?,
            success_criteria: success_criteria?,
            runtime: runtime?,
            model: model?,
            reasoning_effort: reasoning_effort?,
        })
    }
}

/// Guidance for the project: live into the active turn when possible, otherwise queued.
#[derive(Debug, Clone, ToSchema)]
pub struct SteeringRequest {
    #[schema(min_length = 1)]
    pub body: String,
    #[schema(schema_with = steering_kind_schema, required = false)]
    pub kind: String,
}

impl SteeringRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let text = fields.text("body", Text::min(1));
        let kind = fields.literal("kind", &STEERING_KINDS, Some("STEER"));
        fields.finish();
        Some(Self { body: text?, kind: kind? })
    }
}

// ─── Resources ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, ToSchema)]
pub struct VariableValueRequest {
    #[schema(min_length = 1, max_length = 32768)]
    pub value: String,
}

impl VariableValueRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let value = fields.text("value", Text::between(1, RESOURCE_VALUE_LIMIT));
        fields.finish();
        Some(Self { value: value? })
    }
}

/// A secret value: write-only, never returned.
#[derive(Clone, ToSchema)]
pub struct SecretValueRequest {
    #[schema(min_length = 1, max_length = 32768, format = Password, write_only)]
    pub value: String,
}

impl std::fmt::Debug for SecretValueRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretValueRequest").field("value", &"[REDACTED]").finish()
    }
}

impl SecretValueRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let value = fields.secret("value", Text::between(1, RESOURCE_VALUE_LIMIT));
        fields.finish();
        Some(Self { value: value? })
    }
}

// ─── Members ──────────────────────────────────────────────────────────────────

/// Adds a member or changes their role (owners). A project always keeps an owner.
#[derive(Debug, Clone, ToSchema)]
pub struct MemberRequest {
    #[schema(min_length = 1, max_length = 300)]
    pub subject: String,
    #[schema(min_length = 1, max_length = 240)]
    pub display_name: String,
    #[schema(max_length = 320)]
    pub email: Option<String>,
    #[schema(value_type = schemas::Role, inline)]
    pub role: String,
}

impl MemberRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<MemberChange> {
        let mut fields = Fields::open(body, errors)?;
        let subject = fields.text("subject", Text::between(1, 300));
        let display_name = fields.text("display_name", Text::between(1, 240));
        let email = fields.nullable_text("email", Text::max(320));
        let role = fields.literal("role", &ROLES, None);
        fields.finish();
        Some(MemberChange {
            subject: subject?,
            display_name: display_name?,
            email: email?,
            role: role?,
        })
    }
}

/// Shares the project by email with someone in your organization (owners).
#[derive(Debug, Clone, ToSchema)]
pub struct ShareRequest {
    #[schema(min_length = 3, max_length = 320)]
    pub email: String,
    #[schema(value_type = schemas::Role, inline)]
    pub role: String,
}

impl ShareRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let email = fields.text("email", Text::between(3, 320));
        let role = fields.literal("role", &ROLES, None);
        fields.finish();
        Some(Self { email: email?, role: role? })
    }
}

// ─── Knowledge ────────────────────────────────────────────────────────────────

/// An explicit edge between two artifacts.
#[derive(Debug, Clone, ToSchema)]
pub struct RelationRequest {
    pub source_id: String,
    pub target_id: String,
    #[schema(rename = "type", min_length = 1, max_length = 80)]
    pub relation_type: String,
    #[schema(default = "", required = false)]
    pub description: String,
}

impl RelationRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<NewRelation> {
        let mut fields = Fields::open(body, errors)?;
        let source_id = fields.text("source_id", Text::ANY);
        let target_id = fields.text("target_id", Text::ANY);
        let relation_type = fields.text("type", Text::between(1, 80));
        let description = fields.text_or("description", "", Text::ANY);
        fields.finish();
        Some(NewRelation {
            source_id: source_id?,
            target_id: target_id?,
            relation_type: relation_type?,
            description: description?,
        })
    }
}

#[derive(Debug, Clone, ToSchema)]
pub struct CommentRequest {
    #[schema(min_length = 1)]
    pub body: String,
}

impl CommentRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let text = fields.text("body", Text::min(1));
        fields.finish();
        Some(Self { body: text? })
    }
}

/// A named knowledge query.
#[derive(Debug, Clone, ToSchema)]
pub struct SavedViewRequest {
    #[schema(min_length = 1, max_length = 160)]
    pub name: String,
    pub query: Map<String, Value>,
}

impl SavedViewRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let name = fields.text("name", Text::between(1, 160));
        let query = fields.object("query", true);
        fields.finish();
        Some(Self { name: name?, query: query? })
    }
}

/// A URL or connector source. Sources never carry credentials.
#[derive(Debug, Clone, ToSchema)]
pub struct SourceRequest {
    #[schema(min_length = 1, max_length = 200)]
    pub name: String,
    #[schema(rename = "type", value_type = RequestSourceType, inline)]
    pub source_type: String,
    #[schema(min_length = 1)]
    pub uri: String,
    #[schema(max_length = 200)]
    pub media_type: Option<String>,
    #[schema(required = false)]
    pub metadata: Map<String, Value>,
}

impl SourceRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<SourceInput> {
        let mut fields = Fields::open(body, errors)?;
        let name = fields.text("name", Text::between(1, 200));
        let source_type = fields.literal("type", &SOURCE_TYPES, None);
        let uri = fields.text("uri", Text::min(1));
        let media_type = fields.nullable_text("media_type", Text::max(200));
        let metadata = fields.object("metadata", false);
        fields.finish();
        Some(SourceInput {
            name: name?,
            source_type: source_type?,
            uri: uri?,
            media_type: media_type?,
            metadata: metadata?,
        })
    }
}

/// `multipart/form-data` of `POST /v1/projects/{slug}/sources/upload`.
#[derive(Debug, Clone, ToSchema)]
#[schema(as = Body_upload_source_v1_projects__slug__sources_upload_post)]
pub struct UploadSourceForm {
    #[schema(min_length = 1, max_length = 200)]
    pub name: String,
    #[schema(content_media_type = "application/octet-stream")]
    pub file: String,
}

// ─── Engines ──────────────────────────────────────────────────────────────────

/// How the runtime signs in to Codex: a ChatGPT device login, or a server credential.
#[derive(Debug, Clone, ToSchema)]
pub struct CodexAuthLoginRequest {
    #[schema(schema_with = codex_method_schema, required = false)]
    pub method: String,
}

impl CodexAuthLoginRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let method = fields.literal("method", &CODEX_LOGIN_METHODS, Some("chatgpt"));
        fields.finish();
        Some(Self { method: method? })
    }
}

/// A Claude subscription token or API key for one project (write-only).
#[derive(Clone, ToSchema)]
pub struct EngineTokenRequest {
    #[schema(min_length = 20, max_length = 4096, format = Password, write_only)]
    pub token: String,
}

impl std::fmt::Debug for EngineTokenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineTokenRequest").field("token", &"[REDACTED]").finish()
    }
}

impl EngineTokenRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let token = fields.secret("token", Text::between(20, 4096));
        fields.finish();
        Some(Self { token: token? })
    }
}

/// `PUT …/engine-accounts/claude-code/login/{login_id}`: the code claude.com shows after sign-in.
#[derive(Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct ClaudeLoginCodeRequest {
    #[schema(min_length = 1, max_length = 4096, format = Password, write_only)]
    pub code: String,
}

impl std::fmt::Debug for ClaudeLoginCodeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeLoginCodeRequest").field("code", &"[REDACTED]").finish()
    }
}

impl ClaudeLoginCodeRequest {
    pub fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let code = fields.secret("code", Text::between(1, 4096));
        fields.finish();
        Some(Self { code: code? })
    }
}

// ─── The internal research protocol ───────────────────────────────────────────

fn version(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub fn hypothesis(body: &Body, errors: &mut Errors) -> Option<NewHypothesis> {
    let mut fields = Fields::open(body, errors)?;
    let title = fields.text("title", Text::ANY);
    let statement = fields.text("statement", Text::ANY);
    let mechanism = fields.text_or("mechanism", "", Text::ANY);
    let generalization = fields.text_or("generalization", "", Text::ANY);
    let shortcut_risks = fields.text_or("shortcut_risks", "", Text::ANY);
    let test_plan = fields.text_or("test_plan", "", Text::ANY);
    fields.finish();
    Some(NewHypothesis {
        title: title?,
        statement: statement?,
        mechanism: mechanism?,
        generalization: generalization?,
        shortcut_risks: shortcut_risks?,
        test_plan: test_plan?,
    })
}

pub fn hypothesis_decision(body: &Body, errors: &mut Errors) -> Option<HypothesisDecision> {
    let mut fields = Fields::open(body, errors)?;
    let status = fields.text("status", Text::ANY);
    let conclusion = fields.text("conclusion", Text::ANY);
    let expected_version = fields.int("expected_version", None, Int::ge(1));
    fields.finish();
    Some(HypothesisDecision {
        status: status?,
        conclusion: conclusion?,
        expected_version: version(expected_version?),
    })
}

pub fn experiment(body: &Body, errors: &mut Errors) -> Option<NewExperiment> {
    let mut fields = Fields::open(body, errors)?;
    let hypothesis_id = fields.text("hypothesis_id", Text::ANY);
    let title = fields.text("title", Text::ANY);
    let objective = fields.text("objective", Text::ANY);
    let procedure = fields.text_or("procedure", "", Text::ANY);
    let success_criteria = fields.text_or("success_criteria", "", Text::ANY);
    let guardrails = fields.text_or("guardrails", "", Text::ANY);
    fields.finish();
    Some(NewExperiment {
        hypothesis_id: hypothesis_id?,
        title: title?,
        objective: objective?,
        procedure: procedure?,
        success_criteria: success_criteria?,
        guardrails: guardrails?,
    })
}

/// `ttl_seconds` of an experiment claim (default 1800, 30 to 86 400).
pub fn claim_ttl(body: &Body, errors: &mut Errors) -> Option<i64> {
    let mut fields = Fields::open(body, errors)?;
    let ttl = fields.int("ttl_seconds", Some(1800), Int::between(30, 86_400));
    fields.finish();
    ttl
}

/// An observation's body and optional evidence reference.
pub fn observation(body: &Body, errors: &mut Errors) -> Option<(String, Option<String>)> {
    let mut fields = Fields::open(body, errors)?;
    let text = fields.text("body", Text::ANY);
    let evidence_ref = fields.nullable_text("evidence_ref", Text::ANY);
    fields.finish();
    Some((text?, evidence_ref?))
}

pub fn experiment_outcome(body: &Body, errors: &mut Errors) -> Option<ExperimentOutcome> {
    let mut fields = Fields::open(body, errors)?;
    let results = fields.text("results", Text::ANY);
    let analysis = fields.text("analysis", Text::ANY);
    let decision = fields.text("decision", Text::ANY);
    let expected_version = fields.int("expected_version", None, Int::ge(1));
    fields.finish();
    Some(ExperimentOutcome {
        results: results?,
        analysis: analysis?,
        decision: decision?,
        expected_version: version(expected_version?),
    })
}

pub fn finding(body: &Body, errors: &mut Errors) -> Option<NewFinding> {
    let mut fields = Fields::open(body, errors)?;
    let experiment_id = fields.text("experiment_id", Text::ANY);
    let title = fields.text("title", Text::ANY);
    let finding = fields.text("finding", Text::ANY);
    let evidence = fields.text("evidence", Text::ANY);
    let improvement = fields.text_or("improvement", "", Text::ANY);
    let remaining_debt = fields.text_or("remaining_debt", "", Text::ANY);
    let next_move = fields.text_or("next_move", "", Text::ANY);
    let impact = fields.text_or("impact", "HIGH", Text::ANY);
    fields.finish();
    Some(NewFinding {
        experiment_id: experiment_id?,
        title: title?,
        finding: finding?,
        evidence: evidence?,
        improvement: improvement?,
        remaining_debt: remaining_debt?,
        next_move: next_move?,
        impact: impact?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn errors_of(body: Value, parse: impl Fn(&Body, &mut Errors) -> bool) -> Vec<Value> {
        let mut errors = Errors::new();
        assert!(!parse(&Body::Json(body), &mut errors));
        errors.into_vec()
    }

    #[test]
    fn errors_follow_declaration_order_then_extra_fields() {
        let errors = errors_of(
            json!({"extra": 1, "slug": "", "name": "N", "objective": "O", "success_criteria": "S"}),
            |body, errors| CreateProjectRequest::parse(body, errors).is_some(),
        );
        assert_eq!(
            errors,
            [
                json!({"type": "string_too_short", "loc": ["body", "slug"],
                       "msg": "String should have at least 1 character", "input": "",
                       "ctx": {"min_length": 1}}),
                json!({"type": "extra_forbidden", "loc": ["body", "extra"],
                       "msg": "Extra inputs are not permitted", "input": 1}),
            ]
        );
    }

    #[test]
    fn missing_fields_report_the_whole_body() {
        let errors = errors_of(json!({"type": "t"}), |body, errors| {
            RelationRequest::parse(body, errors).is_some()
        });
        assert_eq!(errors[0]["loc"], json!(["body", "source_id"]));
        assert_eq!(errors[0]["input"], json!({"type": "t"}));
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn nulls_keep_draft_fields() {
        let mut errors = Errors::new();
        let changes = UpdateProjectRequest::parse(
            &Body::Json(json!({"runtime": null, "context": "ctx"})),
            &mut errors,
        )
        .unwrap();
        assert_eq!(changes.runtime, None);
        assert_eq!(changes.context.as_deref(), Some("ctx"));
        assert!(errors.is_empty());
    }
}
