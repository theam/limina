//! Projects: kickoff, lifecycle, steering, review and activity (`api.py`).

use axum::extract::{Request, State};
use axum::http::StatusCode;
use limina_domain::collaboration::paging::Page;

use super::{Params, created, ok, parts, with_status};
use crate::app::{NewProject, PUBLIC_COMMAND_ID_LIMIT, ProjectResponse};
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::requests::{CreateProjectRequest, SteeringRequest, UpdateProjectRequest};
use crate::http::schemas::{
    self, ActorHeader, CommandHeader, EventPage, GuidancePage, GuidanceReceipt, KickoffTemplate,
    PageParams, PreflightResponse, ProjectListParams, ProjectPage, ProjectStatusResponse,
    PublicErrors, ReviewResponse, SnapshotResponse, StatusPageParams,
};
use crate::http::validate::{Int, Text};

/// Creates a project owned by the caller. Execution begins with the `start` action.
#[utoipa::path(post, path = "/v1/projects", operation_id = "create_project_v1_projects_post",
    summary = "Create Project", params(CommandHeader, ActorHeader), request_body = CreateProjectRequest,
    responses((status = 201, description = "Successful Response", body = schemas::ProjectResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn create_project(State(app): State<App>, request: Request) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    let project = CreateProjectRequest::parse(&body, call.errors());
    call.check()?;
    let project = project.checked()?;
    let new = NewProject {
        slug: project.slug,
        name: project.name,
        mission: project.objective,
        success_criteria: project.success_criteria,
        context: project.context,
        runtime: Some(project.runtime),
        model: project.model,
        reasoning_effort: project.reasoning_effort,
    };
    let operations = app.runtime.operations();
    created(operations.create_project(new, &command_id.checked()?, &principal.checked()?).await?)
}

/// The projects the caller can see, oldest first, with the caller's role on each.
#[utoipa::path(get, path = "/v1/projects", operation_id = "list_projects_v1_projects_get",
    summary = "List Projects", params(ProjectListParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = ProjectPage), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn list_projects(State(app): State<App>, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let include_archived = call.query_bool("include_archived", false);
    let cursor = call.query_text("cursor", Text::ANY);
    let limit = call.query_int("limit", 100, Int::between(1, 200));
    call.check()?;
    let page: Page<ProjectResponse> = app
        .runtime
        .operations()
        .list_projects(
            include_archived.checked()?,
            cursor.checked()?.as_deref(),
            limit.checked()?,
            &principal.checked()?,
        )
        .await?;
    ok(page)
}

/// Starting points for a new project's kickoff fields.
#[utoipa::path(get, path = "/v1/project-templates",
    operation_id = "list_project_templates_v1_project_templates_get", summary = "List Project Templates",
    params(ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<KickoffTemplate>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn templates(State(app): State<App>, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().kickoff_templates())
}

#[utoipa::path(get, path = "/v1/projects/{slug}", operation_id = "get_project_v1_projects__slug__get",
    summary = "Get Project", params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = schemas::ProjectResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn get_project(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().get_project(&slug, &principal.checked()?).await?)
}

/// Changes kickoff fields before the project first starts (owners).
#[utoipa::path(patch, path = "/v1/projects/{slug}", operation_id = "update_project_v1_projects__slug__patch",
    summary = "Update Project", params(("slug" = String, Path), ActorHeader), request_body = UpdateProjectRequest,
    responses((status = 200, description = "Successful Response", body = schemas::ProjectResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn update_project(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let changes = UpdateProjectRequest::parse(&body, call.errors());
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.update_project(&slug, &changes.checked()?, &principal.checked()?).await?)
}

/// The project, knowledge counts, active work, pending guidance and newest event.
#[utoipa::path(get, path = "/v1/projects/{slug}/status", operation_id = "project_status_v1_projects__slug__status_get",
    summary = "Project Status", params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = ProjectStatusResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn status(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().get_status(&slug, &principal.checked()?).await?)
}

/// Kickoff readiness, including the engine's credentials.
#[utoipa::path(get, path = "/v1/projects/{slug}/preflight",
    operation_id = "project_preflight_v1_projects__slug__preflight_get", summary = "Project Preflight",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = PreflightResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn preflight(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().preflight(&slug, &principal.checked()?).await?)
}

/// `start`, `pause`, `resume`, `stop` (editors) or `archive` (owners).
#[utoipa::path(post, path = "/v1/projects/{slug}/actions/{action}",
    operation_id = "project_action_v1_projects__slug__actions__action__post", summary = "Project Action",
    params(("slug" = String, Path), ("action" = String, Path), CommandHeader, ActorHeader),
    responses((status = 200, description = "Successful Response", body = schemas::ProjectResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn action(
    State(app): State<App>,
    Params((slug, action)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    let project = operations
        .apply_lifecycle(&slug, &action, &command_id.checked()?, &principal.checked()?)
        .await?;
    ok(project)
}

/// Sends guidance (editors): live into the active turn when possible, otherwise queued.
/// `INTERRUPT` also pauses a running project.
#[utoipa::path(post, path = "/v1/projects/{slug}/steering",
    operation_id = "steer_project_v1_projects__slug__steering_post", summary = "Steer Project",
    params(("slug" = String, Path), CommandHeader, ActorHeader), request_body = SteeringRequest,
    responses((status = 202, description = "Successful Response", body = GuidanceReceipt), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn steer(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    let steering = SteeringRequest::parse(&body, call.errors());
    call.check()?;
    let steering = steering.checked()?;
    let receipt = app
        .runtime
        .operations()
        .steer(&slug, &steering.body, &steering.kind, &command_id.checked()?, &principal.checked()?)
        .await?;
    with_status(StatusCode::ACCEPTED, &receipt)
}

/// Guidance history, newest first.
#[utoipa::path(get, path = "/v1/projects/{slug}/guidance",
    operation_id = "guidance_history_v1_projects__slug__guidance_get", summary = "Guidance History",
    params(("slug" = String, Path), StatusPageParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = GuidancePage), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn guidance(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let status = call.query_text("status", Text::ANY);
    let cursor = call.query_text("cursor", Text::ANY);
    let limit = call.query_int("limit", 50, Int::between(1, 200));
    call.check()?;
    let page = app
        .runtime
        .operations()
        .guidance(
            &slug,
            status.checked()?.as_deref(),
            cursor.checked()?.as_deref(),
            limit.checked()?,
            &principal.checked()?,
        )
        .await?;
    ok(page)
}

/// Status, resources, a page of knowledge split by kind and the latest activity.
#[utoipa::path(get, path = "/v1/projects/{slug}/review", operation_id = "review_project_v1_projects__slug__review_get",
    summary = "Review Project", params(("slug" = String, Path), PageParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = ReviewResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn review(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let cursor = call.query_text("cursor", Text::ANY);
    let limit = call.query_int("limit", 50, Int::between(1, 200));
    call.check()?;
    let review = app
        .runtime
        .operations()
        .review(&slug, cursor.checked()?.as_deref(), limit.checked()?, &principal.checked()?)
        .await?;
    ok(review)
}

/// Durable activity after `after`, or the newest events before `before` (oldest first).
#[utoipa::path(get, path = "/v1/projects/{slug}/events", operation_id = "project_events_v1_projects__slug__events_get",
    summary = "Project Events", params(("slug" = String, Path), schemas::EventParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = EventPage), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn events(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let after = call.query_int("after", 0, Int::ge(0));
    let before = call.query_optional_int("before", Int::ge(1));
    let limit = call.query_int("limit", 200, Int::between(1, 1000));
    call.check()?;
    let page = app
        .runtime
        .operations()
        .activity(
            &slug,
            after.checked()?,
            before.checked()?,
            limit.checked()?,
            &principal.checked()?,
        )
        .await?;
    ok(page)
}

/// The project's knowledge base as Markdown files.
#[utoipa::path(get, path = "/v1/projects/{slug}/snapshot", operation_id = "snapshot_v1_projects__slug__snapshot_get",
    summary = "Snapshot", params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = SnapshotResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn snapshot(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().snapshot(&slug, &principal.checked()?).await?)
}
