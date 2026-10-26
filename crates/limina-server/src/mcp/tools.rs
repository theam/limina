//! The tool catalog: names, descriptions, input and output schemas and annotations exactly as the
//! Python server (FastMCP) published them, and the argument validation FastMCP applied.
//!
//! The schemas are written out here instead of derived, so they stay byte-compatible with what
//! MCP clients already saw: Pydantic's titles (`"Idempotency Key"`, `"<tool>Arguments"`), its
//! alphabetical key order, `default`s on optional arguments and the `DictOutput` / `Output`
//! result schemas.
//!
//! Secret values are deliberately absent: tool arguments enter model transcripts, so secrets are
//! configured through the trusted CLI or REST API only (see [`super::INSTRUCTIONS`]).

use std::sync::Arc;

use limina_core::{ErrorKind, LiminaError};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use crate::app::{ACTOR_LIMIT, PUBLIC_COMMAND_ID_LIMIT};

/// What calling a tool does to the project, published as annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// `readOnlyHint: true`.
    ReadOnly,
    /// Additive writes (`destructiveHint: false`).
    Mutation,
    /// May remove or stop something (`destructiveHint: true`).
    Destructive,
}

/// The shape of a tool's structured result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// A JSON object, returned as the structured content itself.
    Object,
    /// A list, returned as `{"result": [...]}` with one text block per item.
    List,
}

/// The JSON type of an argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Str,
    Int,
    Bool,
    Metric,
}

/// An argument's default; `Required` arguments have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultValue {
    Required,
    Str(&'static str),
    Int(i64),
    Bool(bool),
}

/// One tool argument.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub default: DefaultValue,
    /// Allowed values (a `Literal` in the Python signature).
    pub choices: &'static [&'static str],
    pub max_length: Option<usize>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
}

impl Param {
    const fn new(name: &'static str, kind: Kind, default: DefaultValue) -> Self {
        Self { name, kind, default, choices: &[], max_length: None, minimum: None, maximum: None }
    }

    /// A required string.
    const fn required(name: &'static str) -> Self {
        Self::new(name, Kind::Str, DefaultValue::Required)
    }

    /// An optional string; the empty string means "not given".
    const fn optional(name: &'static str) -> Self {
        Self::new(name, Kind::Str, DefaultValue::Str(""))
    }

    /// A required choice.
    const fn choice(name: &'static str, choices: &'static [&'static str]) -> Self {
        Self { choices, ..Self::required(name) }
    }

    /// An optional choice with a default.
    const fn choice_or(
        name: &'static str,
        choices: &'static [&'static str],
        default: &'static str,
    ) -> Self {
        Self { choices, ..Self::new(name, Kind::Str, DefaultValue::Str(default)) }
    }

    const fn int(name: &'static str, default: i64) -> Self {
        Self::new(name, Kind::Int, DefaultValue::Int(default))
    }

    const fn bounded(name: &'static str, default: i64, minimum: i64, maximum: Option<i64>) -> Self {
        Self { minimum: Some(minimum), maximum, ..Self::int(name, default) }
    }

    const fn flag(name: &'static str, default: bool) -> Self {
        Self::new(name, Kind::Bool, DefaultValue::Bool(default))
    }

    /// The teammate a write is recorded for (static-token callers only).
    const fn actor() -> Self {
        Self { max_length: Some(ACTOR_LIMIT), ..Self::optional("actor") }
    }

    /// The client's retry key for a write.
    const fn idempotency_key() -> Self {
        Self { max_length: Some(PUBLIC_COMMAND_ID_LIMIT), ..Self::optional("idempotency_key") }
    }

    /// A page size between 1 and 200 (default 50).
    const fn page_limit() -> Self {
        Self::bounded("limit", 50, 1, Some(200))
    }

    fn is_required(&self) -> bool {
        self.default == DefaultValue::Required
    }

    /// Pydantic's title for a field: words split on `_`, each capitalized.
    fn title(&self) -> String {
        python_title(self.name)
    }

    /// The property schema, keys in Pydantic's (alphabetical) order.
    fn schema(&self) -> Value {
        if self.kind == Kind::Metric {
            use utoipa::PartialSchema;
            if let Ok(Value::Object(mut schema)) =
                serde_json::to_value(crate::http::routes::progress::MetricFields::schema())
            {
                schema.insert("title".to_owned(), json!(self.title()));
                return Value::Object(schema);
            }
        }
        let mut schema = Map::new();
        if let Some(default) = self.default_value() {
            schema.insert("default".into(), default);
        }
        if !self.choices.is_empty() {
            schema.insert("enum".into(), json!(self.choices));
        }
        if let Some(max_length) = self.max_length {
            schema.insert("maxLength".into(), json!(max_length));
        }
        if let Some(maximum) = self.maximum {
            schema.insert("maximum".into(), json!(maximum));
        }
        if let Some(minimum) = self.minimum {
            schema.insert("minimum".into(), json!(minimum));
        }
        schema.insert("title".into(), json!(self.title()));
        let kind = match self.kind {
            Kind::Str => "string",
            Kind::Int => "integer",
            Kind::Bool => "boolean",
            Kind::Metric => "object",
        };
        schema.insert("type".into(), json!(kind));
        Value::Object(schema)
    }

    fn default_value(&self) -> Option<Value> {
        match self.default {
            DefaultValue::Required => None,
            DefaultValue::Str(value) => Some(json!(value)),
            DefaultValue::Int(value) => Some(json!(value)),
            DefaultValue::Bool(value) => Some(json!(value)),
        }
    }
}

/// Which tool a [`ToolSpec`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolId {
    ListProjects,
    GetProjectStatus,
    CreateProject,
    ManageProject,
    SteerProject,
    ReviewProject,
    GetKnowledge,
    ListActivity,
    SetProjectVariable,
    ListProjectResources,
    RemoveProjectResource,
    PreflightProject,
    QueryKnowledge,
    GetKnowledgeGraph,
    ListGuidance,
    ListRuntimeRuns,
    GetProjectAnalytics,
    GetWorkspace,
    ListOutputs,
    ReadOutput,
    KnowledgeRevisions,
    ListProjectMetrics,
    SetProjectMetric,
    RegisterProjectSource,
    ListProjectMembers,
    SetProjectMember,
}

/// One tool of the catalog.
#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub id: ToolId,
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    pub output: Output,
    pub params: &'static [Param],
}

impl ToolSpec {
    /// The tool as `tools/list` publishes it.
    pub fn tool(&self) -> Tool {
        let mut tool = Tool::new(self.name, self.description, Arc::new(self.input_schema()));
        tool.output_schema = Some(Arc::new(self.output_schema()));
        tool.annotations = Some(self.annotations());
        tool
    }

    fn input_schema(&self) -> JsonObject {
        let properties: Map<String, Value> =
            self.params.iter().map(|param| (param.name.to_owned(), param.schema())).collect();
        let required: Vec<&str> = self
            .params
            .iter()
            .filter(|param| param.is_required())
            .map(|param| param.name)
            .collect();
        let mut schema = Map::new();
        schema.insert("properties".into(), Value::Object(properties));
        if !required.is_empty() {
            schema.insert("required".into(), json!(required));
        }
        schema.insert("title".into(), json!(format!("{}Arguments", self.name)));
        schema.insert("type".into(), json!("object"));
        schema
    }

    fn output_schema(&self) -> JsonObject {
        let schema = match self.output {
            Output::Object => json!({
                "additionalProperties": true,
                "title": format!("{}DictOutput", self.name),
                "type": "object",
            }),
            Output::List => json!({
                "properties": {
                    "result": {
                        "items": {"additionalProperties": true, "type": "object"},
                        "title": "Result",
                        "type": "array",
                    },
                },
                "required": ["result"],
                "title": format!("{}Output", self.name),
                "type": "object",
            }),
        };
        match schema {
            Value::Object(schema) => schema,
            _ => Map::new(),
        }
    }

    fn annotations(&self) -> ToolAnnotations {
        let annotations = match self.effect {
            Effect::ReadOnly => ToolAnnotations::new().read_only(true),
            Effect::Mutation => ToolAnnotations::new().read_only(false).destructive(false),
            Effect::Destructive => ToolAnnotations::new().read_only(false).destructive(true),
        };
        annotations.open_world(false)
    }

    /// Validates `arguments` like FastMCP's Pydantic model (lax mode, unknown keys ignored) and
    /// fills in defaults. The error is the Pydantic-style report Python put in the tool error.
    pub fn arguments(&self, arguments: Option<&JsonObject>) -> Result<Arguments, String> {
        let empty = Map::new();
        let supplied = arguments.unwrap_or(&empty);
        let mut values = Map::new();
        let mut errors = Vec::new();
        for param in self.params {
            match supplied.get(param.name) {
                None => match param.default_value() {
                    Some(default) => {
                        values.insert(param.name.to_owned(), default);
                    }
                    None => errors.push((param.name, "missing", "Field required".to_owned())),
                },
                Some(value) => match coerce(param, value) {
                    Ok(value) => {
                        values.insert(param.name.to_owned(), value);
                    }
                    Err((kind, message)) => errors.push((param.name, kind, message)),
                },
            }
        }
        if errors.is_empty() {
            return Ok(Arguments { values });
        }
        let count = errors.len();
        let noun = if count == 1 { "error" } else { "errors" };
        let mut report = format!("{count} validation {noun} for {}Arguments", self.name);
        for (field, kind, message) in errors {
            report.push_str(&format!("\n{field}\n  {message} [type={kind}]"));
        }
        Err(report)
    }
}

/// Validated arguments with defaults applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Arguments {
    values: Map<String, Value>,
}

impl Arguments {
    /// A string argument (`""` when absent from the catalog, which validation rules out).
    pub fn text(&self, name: &str) -> &str {
        self.values.get(name).and_then(Value::as_str).unwrap_or_default()
    }

    /// A string argument, `None` when empty (Python's `value or None`).
    pub fn non_empty(&self, name: &str) -> Option<&str> {
        Some(self.text(name)).filter(|value| !value.is_empty())
    }

    pub fn int(&self, name: &str) -> i64 {
        self.values.get(name).and_then(Value::as_i64).unwrap_or_default()
    }

    pub fn value(&self, name: &str) -> Value {
        self.values.get(name).cloned().unwrap_or(Value::Null)
    }

    pub fn flag(&self, name: &str) -> bool {
        self.values.get(name).and_then(Value::as_bool).unwrap_or_default()
    }
}

type Invalid = (&'static str, String);

/// Pydantic's lax-mode coercion for one argument.
fn coerce(param: &Param, value: &Value) -> Result<Value, Invalid> {
    let value = match param.kind {
        Kind::Str => coerce_str(param, value)?,
        Kind::Int => coerce_int(param, value)?,
        Kind::Bool => coerce_bool(value)?,
        Kind::Metric => value
            .as_object()
            .map(|object| Value::Object(object.clone()))
            .ok_or_else(|| ("dict_type", "Input should be an object".to_owned()))?,
    };
    Ok(value)
}

fn coerce_str(param: &Param, value: &Value) -> Result<Value, Invalid> {
    let Some(text) = value.as_str() else {
        if !param.choices.is_empty() {
            return Err(literal_error(param));
        }
        return Err(("string_type", "Input should be a valid string".to_owned()));
    };
    if !param.choices.is_empty() && !param.choices.contains(&text) {
        return Err(literal_error(param));
    }
    if let Some(max_length) = param.max_length
        && text.chars().count() > max_length
    {
        let unit = if max_length == 1 { "character" } else { "characters" };
        return Err(("string_too_long", format!("String should have at most {max_length} {unit}")));
    }
    Ok(Value::String(text.to_owned()))
}

/// `Input should be 'a', 'b' or 'c'`.
fn literal_error(param: &Param) -> Invalid {
    let quoted: Vec<String> = param.choices.iter().map(|choice| format!("'{choice}'")).collect();
    let listed = match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    };
    ("literal_error", format!("Input should be {listed}"))
}

fn coerce_int(param: &Param, value: &Value) -> Result<Value, Invalid> {
    const INVALID: &str = "Input should be a valid integer";
    let number = match value {
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(number), _) => number,
            // Beyond `i64`: saturate, so the bound check below reports it.
            _ if number.is_u64() => i64::MAX,
            (None, Some(float)) if float.is_finite() && float.fract().abs() < f64::EPSILON => {
                float as i64
            }
            (None, Some(float)) if float.is_finite() => {
                return Err((
                    "int_from_float",
                    format!("{INVALID}, got a number with a fractional part"),
                ));
            }
            _ => return Err(("int_type", INVALID.to_owned())),
        },
        Value::String(text) => text.trim().parse::<i64>().map_err(|_| {
            ("int_parsing", format!("{INVALID}, unable to parse string as an integer"))
        })?,
        _ => return Err(("int_type", INVALID.to_owned())),
    };
    if let Some(minimum) = param.minimum
        && number < minimum
    {
        return Err((
            "greater_than_equal",
            format!("Input should be greater than or equal to {minimum}"),
        ));
    }
    if let Some(maximum) = param.maximum
        && number > maximum
    {
        return Err((
            "less_than_equal",
            format!("Input should be less than or equal to {maximum}"),
        ));
    }
    Ok(json!(number))
}

fn coerce_bool(value: &Value) -> Result<Value, Invalid> {
    const INVALID: &str = "Input should be a valid boolean";
    let flag = match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => match number.as_i64() {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(("bool_parsing", format!("{INVALID}, unable to interpret input"))),
        },
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "0" | "off" | "f" | "false" | "n" | "no" => false,
            "1" | "on" | "t" | "true" | "y" | "yes" => true,
            _ => return Err(("bool_parsing", format!("{INVALID}, unable to interpret input"))),
        },
        _ => return Err(("bool_type", INVALID.to_owned())),
    };
    Ok(Value::Bool(flag))
}

/// Python's `name.replace("_", " ").title()` for identifiers.
fn python_title(name: &str) -> String {
    name.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const LIFECYCLE_ACTIONS: &[&str] = &["start", "pause", "resume", "stop", "archive"];
const GUIDANCE_KINDS: &[&str] = &["STEER", "ANSWER", "APPROVAL", "BLOCKER", "INTERRUPT"];
const RUNTIMES: &[&str] = &["codex", "claude-code"];
const SOURCE_TYPES: &[&str] = &["URL", "CONNECTOR"];
const ROLES: &[&str] = &["OWNER", "EDITOR", "VIEWER"];

/// Every tool, in the order the Python server registered (and listed) them.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        id: ToolId::ListProjects,
        name: "limina_list_projects",
        description: "List Limina projects visible to this team instance.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::flag("include_archived", false)],
    },
    ToolSpec {
        id: ToolId::GetProjectStatus,
        name: "limina_get_project_status",
        description: "Get a project's mission, state, next step, blocker, and knowledge counts.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project")],
    },
    ToolSpec {
        id: ToolId::CreateProject,
        name: "limina_create_project",
        description: "Create a durable project; execution begins only after a start action.\n\n\
                      Actor is the teammate steering the project. Supply a stable \
                      idempotency_key\nif the client may retry the request.\n",
        effect: Effect::Mutation,
        output: Output::Object,
        params: &[
            Param::required("slug"),
            Param::required("name"),
            Param::required("mission"),
            Param::required("success_criteria"),
            Param::choice_or("runtime", RUNTIMES, "codex"),
            Param::optional("model"),
            Param::optional("reasoning_effort"),
            Param::optional("context"),
            Param::actor(),
            Param::idempotency_key(),
        ],
    },
    ToolSpec {
        id: ToolId::ManageProject,
        name: "limina_manage_project",
        description: "Start, pause, resume, stop, or archive a Limina-managed project.",
        effect: Effect::Destructive,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::choice("action", LIFECYCLE_ACTIONS),
            Param::actor(),
            Param::idempotency_key(),
        ],
    },
    ToolSpec {
        id: ToolId::SteerProject,
        name: "limina_steer_project",
        description: "Give durable feedback or direction to a project, including active work.",
        effect: Effect::Mutation,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::required("message"),
            Param::choice_or("kind", GUIDANCE_KINDS, "STEER"),
            Param::actor(),
            Param::idempotency_key(),
        ],
    },
    ToolSpec {
        id: ToolId::ReviewProject,
        name: "limina_review_project",
        description: "Review accepted knowledge, active evidence, resources, and recent activity.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project"), Param::optional("cursor"), Param::page_limit()],
    },
    ToolSpec {
        id: ToolId::GetKnowledge,
        name: "limina_get_knowledge",
        description: "Read one hypothesis, experiment, or finding by its durable artifact ID.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project"), Param::required("artifact_id")],
    },
    ToolSpec {
        id: ToolId::ListActivity,
        name: "limina_list_activity",
        description: "Read ordered durable activity after a cursor; use the returned cursor next \
                      time.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::bounded("after", 0, 0, None),
            Param::bounded("limit", 200, 1, Some(1000)),
        ],
    },
    ToolSpec {
        id: ToolId::SetProjectVariable,
        name: "limina_set_project_variable",
        description: "Set visible project configuration or a resource reference; never use for \
                      secrets.",
        effect: Effect::Mutation,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::required("name"),
            Param::required("value"),
            Param::actor(),
            Param::idempotency_key(),
        ],
    },
    ToolSpec {
        id: ToolId::ListProjectResources,
        name: "limina_list_project_resources",
        description: "List project variables and redacted secret metadata.",
        effect: Effect::ReadOnly,
        output: Output::List,
        params: &[Param::required("project")],
    },
    ToolSpec {
        id: ToolId::RemoveProjectResource,
        name: "limina_remove_project_resource",
        description: "Revoke a project variable or secret without exposing its value.",
        effect: Effect::Destructive,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::required("name"),
            Param::actor(),
            Param::idempotency_key(),
        ],
    },
    ToolSpec {
        id: ToolId::PreflightProject,
        name: "limina_preflight_project",
        description: "Check whether a project draft has the mission and runtime prerequisites to \
                      start.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project")],
    },
    ToolSpec {
        id: ToolId::QueryKnowledge,
        name: "limina_query_knowledge",
        description: "Search and filter the paginated project knowledge base.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::optional("query"),
            Param::optional("kind"),
            Param::optional("status"),
            Param::optional("tag"),
            Param::optional("parent"),
            Param::optional("cursor"),
            Param::page_limit(),
        ],
    },
    ToolSpec {
        id: ToolId::GetKnowledgeGraph,
        name: "limina_get_knowledge_graph",
        description: "Read knowledge nodes and both evidence-chain and explicit relations.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project")],
    },
    ToolSpec {
        id: ToolId::ListGuidance,
        name: "limina_list_guidance",
        description: "Review durable feedback with delivery and acknowledgement state.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::optional("status"),
            Param::optional("cursor"),
            Param::page_limit(),
        ],
    },
    ToolSpec {
        id: ToolId::ListRuntimeRuns,
        name: "limina_list_runtime_runs",
        description: "Inspect managed runtime turns, timing, tool counts, failures, and usage.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::optional("status"),
            Param::optional("cursor"),
            Param::page_limit(),
        ],
    },
    ToolSpec {
        id: ToolId::GetProjectAnalytics,
        name: "limina_get_project_analytics",
        description: "Aggregate run, knowledge, and human-guidance metrics for a project.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project"), Param::int("days", 30)],
    },
    ToolSpec {
        id: ToolId::RegisterProjectSource,
        name: "limina_register_project_source",
        description: "Register a URL or connector for the managed runtime to consume.",
        effect: Effect::Mutation,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::required("name"),
            Param::choice("source_type", SOURCE_TYPES),
            Param::required("uri"),
            Param::optional("media_type"),
            Param::actor(),
        ],
    },
    ToolSpec {
        id: ToolId::ListProjectMembers,
        name: "limina_list_project_members",
        description: "List project owners, editors, and viewers.",
        effect: Effect::ReadOnly,
        output: Output::List,
        params: &[Param::required("project")],
    },
    ToolSpec {
        id: ToolId::SetProjectMember,
        name: "limina_set_project_member",
        description: "Add a teammate or change their project role; project ownership is \
                      required.",
        effect: Effect::Mutation,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::required("subject"),
            Param::required("display_name"),
            Param::choice("role", ROLES),
            Param::optional("email"),
            Param::actor(),
        ],
    },
    ToolSpec {
        id: ToolId::ListProjectMetrics,
        name: "limina_list_project_metrics",
        description: "Read evidence-backed KPIs: baseline, current value, target, unit, direction, observation date and evidence note. Missing measurements are unknown, not zero.",
        effect: Effect::ReadOnly,
        output: Output::List,
        params: &[Param::required("project")],
    },
    ToolSpec {
        id: ToolId::SetProjectMetric,
        name: "limina_set_project_metric",
        description: "Define or update a KPI (editors). metric contains name, criterion, unit, direction (at_least/at_most), target, optional baseline/current, evidence_id, observed_at (ISO timestamp), notes. A current value requires a finding or completed experiment in this project. expected_version is 0 to create, or the version from the last read to edit.",
        effect: Effect::Mutation,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::required("metric_id"),
            Param::new("metric", Kind::Metric, DefaultValue::Required),
            Param::bounded("expected_version", 0, 0, Some(2147483647)),
            Param::actor(),
            Param::idempotency_key(),
        ],
    },
    ToolSpec {
        id: ToolId::GetWorkspace,
        name: "limina_get_project_workspace",
        description: "Read the latest durable mission assessment, evidence IDs and pending decision independently of transcript history. Null means no completed checkpoint yet.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project")],
    },
    ToolSpec {
        id: ToolId::ListOutputs,
        name: "limina_list_project_outputs",
        description: "List published project deliverables, 25 per page. Follow next_offset; truncated reports the 1000-entry directory limit. Only the project's outputs/ directory is exposed.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project"), Param::bounded("offset", 0, 0, Some(1000))],
    },
    ToolSpec {
        id: ToolId::ReadOutput,
        name: "limina_read_project_output",
        description: "Read a published deliverable by its exact file name. UTF-8 preview is limited to 128 KiB and indicates truncation; binary files are downloadable through the authorized HTTP download endpoint.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[Param::required("project"), Param::required("name")],
    },
    ToolSpec {
        id: ToolId::KnowledgeRevisions,
        name: "limina_get_knowledge_revisions",
        description: "Read 25 dated note versions per page, newest first. Pass next_before as before to page backwards. Historical content is read-only; this never rolls back the current note.",
        effect: Effect::ReadOnly,
        output: Output::Object,
        params: &[
            Param::required("project"),
            Param::required("artifact_id"),
            Param::bounded("before", 2147483647, 1, Some(2147483647)),
        ],
    },
];

/// The catalog entry for `name`.
pub fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// `_command_id`: the caller's idempotency key, or a fresh one for callers that do not retry.
pub fn command_id(supplied: &str) -> Result<String, LiminaError> {
    let supplied = supplied.trim();
    let command_id = if supplied.is_empty() {
        format!("mcp:{}", limina_core::new_uuid())
    } else {
        supplied.to_owned()
    };
    if command_id.chars().count() > PUBLIC_COMMAND_ID_LIMIT {
        return Err(LiminaError::new(
            ErrorKind::Invariant,
            format!("Idempotency key must be at most {PUBLIC_COMMAND_ID_LIMIT} characters."),
        ));
    }
    Ok(command_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_python() {
        assert_eq!(python_title("idempotency_key"), "Idempotency Key");
        assert_eq!(python_title("artifact_id"), "Artifact Id");
        assert_eq!(python_title("project"), "Project");
    }

    #[test]
    fn arguments_coerce_like_pydantic() {
        let Some(review) = find("limina_review_project") else { panic!("missing tool") };
        let parsed = review
            .arguments(json!({"project": "p", "limit": "20"}).as_object())
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(parsed.int("limit"), 20);
        assert_eq!(parsed.text("cursor"), "");
        let error = review.arguments(json!({"limit": 0}).as_object()).err().unwrap_or_default();
        assert_eq!(
            error,
            "2 validation errors for limina_review_projectArguments\nproject\n  Field required \
             [type=missing]\nlimit\n  Input should be greater than or equal to 1 \
             [type=greater_than_equal]"
        );
        let Some(manage) = find("limina_manage_project") else { panic!("missing tool") };
        let error = manage
            .arguments(json!({"project": "p", "action": "explode"}).as_object())
            .err()
            .unwrap_or_default();
        assert!(error.ends_with(
            "action\n  Input should be 'start', 'pause', 'resume', 'stop' or 'archive' \
             [type=literal_error]"
        ));
        let Some(list) = find("limina_list_projects") else { panic!("missing tool") };
        let parsed =
            list.arguments(json!({"include_archived": "yes"}).as_object()).unwrap_or_else(|e| {
                panic!("{e}");
            });
        assert!(parsed.flag("include_archived"));
    }

    #[test]
    fn idempotency_keys_are_bounded() {
        assert!(command_id("").is_ok_and(|id| id.starts_with("mcp:")));
        assert!(command_id(" key ").is_ok_and(|id| id == "key"));
        let error = command_id(&"x".repeat(56)).err().map(|error| error.message);
        assert_eq!(error.as_deref(), Some("Idempotency key must be at most 55 characters."));
    }
}
