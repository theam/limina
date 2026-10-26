//! The MCP handler: tools and resources over [`ProjectOperations`], as the authenticated caller.

use std::borrow::Cow;

use axum::http::request::Parts;
use limina_core::{LiminaError, Principal, Result};
use limina_domain::collaboration::MemberChange;
use limina_domain::knowledge::{KnowledgeQuery, SourceInput};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode, ErrorData,
    Implementation, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

use super::INSTRUCTIONS;
use super::results::{self, json, pretty};
use super::security::McpCaller;
use super::tools::{self, Arguments, Output, TOOLS, ToolId, ToolSpec};
use crate::app::{ACTOR_LIMIT, NewProject, ProjectOperations};

/// Python's `ProjectOperations.list_projects` page size.
const PROJECT_PAGE: i64 = 100;
/// The review's knowledge page when read as a resource.
const REVIEW_PAGE: i64 = 50;
const JSON_MIME: &str = "application/json";
/// The newest protocol revision served.
const LATEST_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;

/// One handler per request (the transport is stateless); cloning shares the services.
#[derive(Clone)]
pub(crate) struct LiminaMcp {
    operations: ProjectOperations,
}

impl LiminaMcp {
    pub(crate) fn new(operations: ProjectOperations) -> Self {
        Self { operations }
    }

    async fn call(
        &self,
        request: &CallToolRequestParams,
        context: &RequestContext<RoleServer>,
    ) -> CallToolResult {
        let name = request.name.as_ref();
        let Some(spec) = tools::find(name) else {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "Unknown tool: {name}"
            ))]);
        };
        let outcome = match (caller(context), spec.arguments(request.arguments.as_ref())) {
            (None, _) => Err(LiminaError::authentication().message),
            (Some(_), Err(report)) => Err(report),
            (Some(caller), Ok(arguments)) => {
                let client = context.meta.client_info().map(|client| client.name);
                self.run(spec, &arguments, &caller, client.as_deref()).await.map_err(|error| {
                    if error.http_status() >= 500 {
                        tracing::warn!(
                            tool = name,
                            code = error.code(),
                            "MCP tool failed: {error}"
                        );
                    }
                    error.message
                })
            }
        };
        match outcome {
            Ok(value) => success(spec.output, value),
            // FastMCP reported every failure inside the tool as an error result with this text.
            Err(message) => CallToolResult::error(vec![ContentBlock::text(format!(
                "Error executing tool {name}: {message}"
            ))]),
        }
    }

    async fn run(
        &self,
        spec: &ToolSpec,
        arguments: &Arguments,
        caller: &Principal,
        client: Option<&str>,
    ) -> Result<Value> {
        let ops = &self.operations;
        let project = arguments.text("project");
        let acting = || acting_principal(caller, arguments.text("actor"), client);
        let command_id = || tools::command_id(arguments.text("idempotency_key"));
        match spec.id {
            ToolId::ListProjects => {
                let archived = arguments.flag("include_archived");
                json(&ops.list_projects(archived, None, PROJECT_PAGE, caller).await?)
            }
            ToolId::GetProjectStatus => results::status(&ops.get_status(project, caller).await?),
            ToolId::CreateProject => {
                let principal = acting()?;
                let command_id = command_id()?;
                let new = NewProject {
                    slug: arguments.text("slug").to_owned(),
                    name: arguments.text("name").to_owned(),
                    mission: arguments.text("mission").to_owned(),
                    success_criteria: arguments.text("success_criteria").to_owned(),
                    context: arguments.text("context").to_owned(),
                    runtime: Some(arguments.text("runtime").to_owned()),
                    model: Some(arguments.text("model").to_owned()),
                    reasoning_effort: Some(arguments.text("reasoning_effort").to_owned()),
                };
                json(&ops.create_project(new, &command_id, &principal).await?)
            }
            ToolId::ManageProject => {
                let principal = acting()?;
                let command_id = command_id()?;
                let action = arguments.text("action");
                json(&ops.apply_lifecycle(project, action, &command_id, &principal).await?)
            }
            ToolId::SteerProject => {
                let principal = acting()?;
                let command_id = command_id()?;
                let (message, kind) = (arguments.text("message"), arguments.text("kind"));
                json(&ops.steer(project, message, kind, &command_id, &principal).await?)
            }
            ToolId::ReviewProject => {
                let (cursor, limit) = (arguments.non_empty("cursor"), arguments.int("limit"));
                results::review(&ops.review(project, cursor, limit, caller).await?)
            }
            ToolId::GetKnowledge => {
                let artifact_id = arguments.text("artifact_id");
                results::artifact(&ops.get_knowledge(project, artifact_id, caller).await?)
            }
            ToolId::ListActivity => {
                let (after, limit) = (arguments.int("after"), arguments.int("limit"));
                json(&ops.activity(project, after, None, limit, caller).await?)
            }
            ToolId::SetProjectVariable => {
                let principal = acting()?;
                let command_id = command_id()?;
                let (name, value) = (arguments.text("name"), arguments.text("value"));
                let resource =
                    ops.set_variable(project, name, value, &command_id, &principal).await?;
                results::resource(&resource)
            }
            ToolId::ListProjectResources => {
                results::resources(&ops.list_resources(project, caller).await?).map(Value::Array)
            }
            ToolId::RemoveProjectResource => {
                let principal = acting()?;
                let command_id = command_id()?;
                let name = arguments.text("name");
                results::resource(
                    &ops.remove_resource(project, name, &command_id, &principal).await?,
                )
            }
            ToolId::PreflightProject => json(&ops.preflight(project, caller).await?),
            ToolId::QueryKnowledge => {
                let query = KnowledgeQuery {
                    query: arguments.non_empty("query").map(str::to_owned),
                    kind: arguments.non_empty("kind").map(str::to_owned),
                    status: arguments.non_empty("status").map(str::to_owned),
                    tag: arguments.non_empty("tag").map(str::to_owned),
                    parent: arguments.non_empty("parent").map(str::to_owned),
                    cursor: arguments.non_empty("cursor").map(str::to_owned),
                    limit: arguments.int("limit"),
                };
                results::knowledge_page(&ops.query_knowledge(project, &query, caller).await?)
            }
            ToolId::GetKnowledgeGraph => {
                results::knowledge_graph(&ops.knowledge_graph(project, caller).await?)
            }
            ToolId::GetWorkspace => {
                let assessment = ops.workspace_assessment(project, caller).await?;
                json(&json!({"assessment": assessment}))
            }
            ToolId::ListOutputs => {
                json(&ops.project_outputs(project, arguments.int("offset") as usize, caller).await?)
            }
            ToolId::ReadOutput => {
                json(&ops.output_preview(project, arguments.text("name"), caller).await?)
            }
            ToolId::KnowledgeRevisions => json(
                &ops.knowledge_history(
                    project,
                    arguments.text("artifact_id"),
                    arguments.int("before") as i32,
                    caller,
                )
                .await?,
            ),
            ToolId::ListProjectMetrics => json(&ops.metrics(project, caller).await?),
            ToolId::SetProjectMetric => {
                let principal = acting()?;
                let id = command_id()?;
                let metric =
                    limina_domain::progress::MetricInput::parse(arguments.value("metric"))?;
                let version = i32::try_from(arguments.int("expected_version"))
                    .map_err(|_| LiminaError::invariant("Invalid KPI version."))?;
                json(
                    &ops.set_metric(
                        project,
                        arguments.text("metric_id"),
                        metric,
                        version,
                        &id,
                        &principal,
                    )
                    .await?,
                )
            }
            ToolId::ListGuidance => {
                let (status, cursor) =
                    (arguments.non_empty("status"), arguments.non_empty("cursor"));
                let limit = arguments.int("limit");
                json(&ops.guidance(project, status, cursor, limit, caller).await?)
            }
            ToolId::ListRuntimeRuns => {
                let (status, cursor) =
                    (arguments.non_empty("status"), arguments.non_empty("cursor"));
                let limit = arguments.int("limit");
                json(&ops.runs(project, status, cursor, limit, caller).await?)
            }
            ToolId::GetProjectAnalytics => {
                json(&ops.analytics(project, arguments.int("days"), caller).await?)
            }
            ToolId::RegisterProjectSource => {
                let principal = acting()?;
                let source = SourceInput {
                    name: arguments.text("name").to_owned(),
                    source_type: arguments.text("source_type").to_owned(),
                    uri: arguments.text("uri").to_owned(),
                    media_type: arguments.non_empty("media_type").map(str::to_owned),
                    metadata: Map::new(),
                };
                json(&ops.set_source(project, &source, &principal).await?)
            }
            ToolId::ListProjectMembers => json(&ops.members(project, caller).await?),
            ToolId::SetProjectMember => {
                let principal = acting()?;
                let change = MemberChange {
                    subject: arguments.text("subject").to_owned(),
                    display_name: arguments.text("display_name").to_owned(),
                    email: arguments.non_empty("email").map(str::to_owned),
                    role: arguments.text("role").to_owned(),
                };
                json(&ops.set_member(project, &change, &principal).await?)
            }
        }
    }

    async fn read(&self, uri: &str, caller: &Principal) -> Result<Option<Value>> {
        let ops = &self.operations;
        let value = match ResourceUri::parse(uri) {
            None => return Ok(None),
            Some(ResourceUri::Projects) => {
                json(&ops.list_projects(false, None, PROJECT_PAGE, caller).await?)?
            }
            Some(ResourceUri::Status(project)) => {
                results::status(&ops.get_status(project, caller).await?)?
            }
            Some(ResourceUri::Review(project)) => {
                results::review(&ops.review(project, None, REVIEW_PAGE, caller).await?)?
            }
            Some(ResourceUri::Knowledge(project, artifact_id)) => {
                results::artifact(&ops.get_knowledge(project, artifact_id, caller).await?)?
            }
            Some(ResourceUri::Snapshot(project)) => json(&ops.snapshot(project, caller).await?)?,
        };
        Ok(Some(value))
    }
}

impl ServerHandler for LiminaMcp {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_prompts()
            .enable_resources()
            .enable_tools()
            .build();
        ServerConfig::new(capabilities)
            .with_protocol_version(LATEST_PROTOCOL_VERSION)
            .with_server_info(Implementation::new("Limina", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    /// The versions the Python server negotiated (its SDK's), all with an `initialize` handshake.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&LATEST_PROTOCOL_VERSION))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(TOOLS.iter().map(ToolSpec::tool).collect()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools::find(name).map(ToolSpec::tool)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        Ok(self.call(&request, &context).await.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourcesResult, ErrorData> {
        let projects = Resource::new(ResourceUri::PROJECTS, "Limina projects")
            .with_description("All active Limina projects as JSON.")
            .with_mime_type(JSON_MIME);
        Ok(ListResourcesResult::with_all_items(vec![projects]))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourceTemplatesResult, ErrorData> {
        let templates = RESOURCE_TEMPLATES
            .iter()
            .map(|(uri_template, name, description)| {
                ResourceTemplate::new(*uri_template, *name)
                    .with_description(*description)
                    .with_mime_type(JSON_MIME)
            })
            .collect();
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri;
        let caller = caller(&context).ok_or_else(|| mcp_error(&LiminaError::authentication()))?;
        match self.read(&uri, &caller).await {
            Ok(Some(value)) => {
                let contents =
                    ResourceContents::text(pretty(&value), uri.clone()).with_mime_type(JSON_MIME);
                Ok(ReadResourceResult::new(vec![contents]).into())
            }
            Ok(None) => {
                Err(ErrorData::resource_not_found(format!("Unknown resource: {uri}"), None))
            }
            Err(error) => Err(mcp_error(&error)),
        }
    }
}

/// The resource templates: URI template, name, description.
const RESOURCE_TEMPLATES: [(&str, &str, &str); 4] = [
    (
        "limina://projects/{project}/status",
        "Limina project status",
        "Mission, active state, and knowledge counts for one project.",
    ),
    (
        "limina://projects/{project}/review",
        "Limina project review",
        "The complete review surface for one project.",
    ),
    (
        "limina://projects/{project}/knowledge/{artifact_id}",
        "Limina knowledge artifact",
        "One durable hypothesis, experiment, or finding.",
    ),
    (
        "limina://projects/{project}/snapshot",
        "Limina knowledge snapshot",
        "Deterministic Markdown knowledge-base files for one project.",
    ),
];

/// A readable resource. Template parameters are single non-empty path segments, used as given
/// (FastMCP's `[^/]+` match).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceUri<'a> {
    Projects,
    Status(&'a str),
    Review(&'a str),
    Knowledge(&'a str, &'a str),
    Snapshot(&'a str),
}

impl<'a> ResourceUri<'a> {
    const PROJECTS: &'static str = "limina://projects";

    fn parse(uri: &'a str) -> Option<Self> {
        if uri == Self::PROJECTS {
            return Some(Self::Projects);
        }
        let rest = uri.strip_prefix(Self::PROJECTS)?.strip_prefix('/')?;
        let segments: Vec<&str> = rest.split('/').collect();
        if segments.iter().any(|segment| segment.is_empty()) {
            return None;
        }
        match segments.as_slice() {
            [project, "status"] => Some(Self::Status(project)),
            [project, "review"] => Some(Self::Review(project)),
            [project, "knowledge", artifact_id] => Some(Self::Knowledge(project, artifact_id)),
            [project, "snapshot"] => Some(Self::Snapshot(project)),
            _ => None,
        }
    }
}

/// The caller the transport guard authenticated (see [`super::security`]).
fn caller(context: &RequestContext<RoleServer>) -> Option<Principal> {
    let parts = context.extensions.get::<Parts>()?;
    parts.extensions.get::<McpCaller>().map(|caller| caller.0.clone())
}

/// `_principal`: who a write is recorded for.
///
/// Static-token callers name the teammate they act for with `actor` (as REST callers do with
/// `X-Limina-Actor`); identity-provider principals always act as themselves. A principal without
/// a display name is recorded as the MCP client (`mcp:<client name>`, `mcp:client` when the
/// request does not say).
fn acting_principal(caller: &Principal, actor: &str, client: Option<&str>) -> Result<Principal> {
    let actor = actor.trim();
    let principal = if caller.auth_mode == "local" && !actor.is_empty() {
        Principal::local(actor, false)
    } else if caller.display_name.is_empty() {
        let client = client.filter(|name| !name.is_empty()).unwrap_or("client");
        Principal { display_name: format!("mcp:{client}"), ..caller.clone() }
    } else {
        caller.clone()
    };
    if principal.actor().chars().count() > ACTOR_LIMIT {
        return Err(LiminaError::invariant(format!(
            "Actor identity must be at most {ACTOR_LIMIT} characters."
        )));
    }
    Ok(principal)
}

/// `_mcp_error`: caller errors are invalid parameters, runtime failures internal errors; the
/// data carries the API's error code and details.
fn mcp_error(error: &LiminaError) -> ErrorData {
    let code = if error.http_status() < 500 {
        ErrorCode::INVALID_PARAMS
    } else {
        ErrorCode::INTERNAL_ERROR
    };
    let data = json!({"code": error.code(), "details": Value::Object(error.details.clone())});
    ErrorData::new(code, error.message.clone(), Some(data))
}

/// A structured result with FastMCP's text rendering: one JSON block for an object, one per
/// item for a list (whose structured form is `{"result": [...]}`).
fn success(output: Output, value: Value) -> CallToolResult {
    let (content, structured) = match (output, value) {
        (Output::List, Value::Array(items)) => {
            let content = items.iter().map(|item| ContentBlock::text(pretty(item))).collect();
            (content, json!({"result": items}))
        }
        (_, value) => (vec![ContentBlock::text(pretty(&value))], value),
    };
    let mut result = CallToolResult::success(content);
    result.structured_content = Some(structured);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_uris_follow_the_templates() {
        assert_eq!(ResourceUri::parse("limina://projects"), Some(ResourceUri::Projects));
        assert_eq!(
            ResourceUri::parse("limina://projects/p/status"),
            Some(ResourceUri::Status("p"))
        );
        assert_eq!(
            ResourceUri::parse("limina://projects/p/review"),
            Some(ResourceUri::Review("p"))
        );
        assert_eq!(
            ResourceUri::parse("limina://projects/p/knowledge/H001"),
            Some(ResourceUri::Knowledge("p", "H001"))
        );
        assert_eq!(
            ResourceUri::parse("limina://projects/p/snapshot"),
            Some(ResourceUri::Snapshot("p"))
        );
        for unknown in [
            "limina://projects/",
            "limina://projects//status",
            "limina://projects/p",
            "limina://projects/p/status/extra",
            "limina://projectsx/p/status",
            "limina://nothing",
        ] {
            assert_eq!(ResourceUri::parse(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn only_static_token_callers_name_an_actor() {
        let local = Principal::local("maya", true);
        let acting = acting_principal(&local, " sam ", None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((acting.subject.as_str(), acting.actor().as_str()), ("local:sam", "sam"));
        assert!(!acting.instance_admin);
        let same = acting_principal(&local, "", None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(same, local);
        let person = Principal::new("oidc|1", "Olivia");
        let acting = acting_principal(&person, "sam", None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(acting, person);
        let nameless = Principal::new("svc", "");
        let acting = acting_principal(&nameless, "", Some("ide")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(acting.actor(), "mcp:ide");
        let acting = acting_principal(&nameless, "", None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(acting.actor(), "mcp:client");
    }

    #[test]
    fn errors_map_like_python() {
        let caller = mcp_error(&LiminaError::not_found("Challenge 'x' does not exist."));
        assert_eq!(caller.code, ErrorCode::INVALID_PARAMS);
        assert_eq!(caller.message, "Challenge 'x' does not exist.");
        assert_eq!(caller.data, Some(json!({"code": "not_found", "details": {}})));
        let runtime = mcp_error(&LiminaError::transport("down"));
        assert_eq!(runtime.code, ErrorCode::INTERNAL_ERROR);
    }
}
