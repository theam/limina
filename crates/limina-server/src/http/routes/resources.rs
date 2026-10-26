//! Resources, members, live tickets, sources, runs and analytics.

use axum::extract::{Request, State};

use super::{Params, ok, parts};
use crate::app::PUBLIC_COMMAND_ID_LIMIT;
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::requests::{
    MemberRequest, SecretValueRequest, SourceRequest, VariableValueRequest,
};
use crate::http::schemas::{
    ActorHeader, AnalyticsParams, AnalyticsResponse, CommandHeader, LiveTicketResponse,
    MemberParams, MemberResponse, PublicErrors, ResourceResponse, RuntimeRunDetail, RuntimeRunPage,
    SourceResponse, StatusPageParams,
};
use crate::http::validate::{Int, Text};

/// Sets a project variable (editors) and restarts active work with it.
#[utoipa::path(put, path = "/v1/projects/{slug}/resources/variables/{name}",
    operation_id = "set_variable_v1_projects__slug__resources_variables__name__put", summary = "Set Variable",
    params(("slug" = String, Path), ("name" = String, Path), CommandHeader, ActorHeader),
    request_body = VariableValueRequest,
    responses((status = 200, description = "Successful Response", body = ResourceResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn set_variable(
    State(app): State<App>,
    Params((slug, name)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    let value = VariableValueRequest::parse(&body, call.errors());
    call.check()?;
    let resource = app
        .runtime
        .operations()
        .set_variable(
            &slug,
            &name,
            &value.checked()?.value,
            &command_id.checked()?,
            &principal.checked()?,
        )
        .await?;
    ok(resource)
}

/// Sets a project secret (owners). The value is encrypted and never returned.
#[utoipa::path(put, path = "/v1/projects/{slug}/resources/secrets/{name}",
    operation_id = "set_secret_v1_projects__slug__resources_secrets__name__put", summary = "Set Secret",
    params(("slug" = String, Path), ("name" = String, Path), CommandHeader, ActorHeader),
    request_body = SecretValueRequest,
    responses((status = 200, description = "Successful Response", body = ResourceResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn set_secret(
    State(app): State<App>,
    Params((slug, name)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    let value = SecretValueRequest::parse(&body, call.errors());
    call.check()?;
    let resource = app
        .runtime
        .operations()
        .set_secret(
            &slug,
            &name,
            &value.checked()?.value,
            &command_id.checked()?,
            &principal.checked()?,
        )
        .await?;
    ok(resource)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/resources",
    operation_id = "list_resources_v1_projects__slug__resources_get", summary = "List Resources",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<ResourceResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn list_resources(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().list_resources(&slug, &principal.checked()?).await?)
}

#[utoipa::path(delete, path = "/v1/projects/{slug}/resources/{name}",
    operation_id = "remove_resource_v1_projects__slug__resources__name__delete", summary = "Remove Resource",
    params(("slug" = String, Path), ("name" = String, Path), CommandHeader, ActorHeader),
    responses((status = 200, description = "Successful Response", body = ResourceResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn remove_resource(
    State(app): State<App>,
    Params((slug, name)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    let removed = operations
        .remove_resource(&slug, &name, &command_id.checked()?, &principal.checked()?)
        .await?;
    ok(removed)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/members",
    operation_id = "list_members_v1_projects__slug__members_get", summary = "List Members",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<MemberResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn members(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().members(&slug, &principal.checked()?).await?)
}

/// Adds a member or changes their role (owners).
#[utoipa::path(put, path = "/v1/projects/{slug}/members",
    operation_id = "set_member_v1_projects__slug__members_put", summary = "Set Member",
    params(("slug" = String, Path), ActorHeader), request_body = MemberRequest,
    responses((status = 200, description = "Successful Response", body = MemberResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn set_member(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let change = MemberRequest::parse(&body, call.errors());
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.set_member(&slug, &change.checked()?, &principal.checked()?).await?)
}

/// Removes a member (owners).
#[utoipa::path(delete, path = "/v1/projects/{slug}/members",
    operation_id = "remove_member_v1_projects__slug__members_delete", summary = "Remove Member",
    params(("slug" = String, Path), MemberParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = MemberResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn remove_member(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let subject = call.query_required_text("subject", Text::between(1, 300));
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.remove_member(&slug, &subject.checked()?, &principal.checked()?).await?)
}

/// A single-use ticket for the live WebSocket, carrying the caller's role.
#[utoipa::path(post, path = "/v1/projects/{slug}/live-ticket",
    operation_id = "live_ticket_v1_projects__slug__live_ticket_post", summary = "Live Ticket",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = LiveTicketResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn live_ticket(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().issue_live_ticket(&slug, &principal.checked()?).await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/sources",
    operation_id = "list_sources_v1_projects__slug__sources_get", summary = "List Sources",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<SourceResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn sources(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().sources(&slug, &principal.checked()?).await?)
}

/// Adds or replaces a URL or connector source (editors). Sources never carry credentials.
#[utoipa::path(put, path = "/v1/projects/{slug}/sources",
    operation_id = "set_source_v1_projects__slug__sources_put", summary = "Set Source",
    params(("slug" = String, Path), ActorHeader), request_body = SourceRequest,
    responses((status = 200, description = "Successful Response", body = SourceResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn set_source(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let source = SourceRequest::parse(&body, call.errors());
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.set_source(&slug, &source.checked()?, &principal.checked()?).await?)
}

#[utoipa::path(delete, path = "/v1/projects/{slug}/sources/{source_id}",
    operation_id = "remove_source_v1_projects__slug__sources__source_id__delete", summary = "Remove Source",
    params(("slug" = String, Path), ("source_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = SourceResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn remove_source(
    State(app): State<App>,
    Params((slug, source_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    removed_source(app, slug, source_id, request).await
}

/// `DELETE …/sources/upload`: the static `upload` route only takes `POST`, so this is the source
/// route with `upload` as its ID (as the Python router matched it).
pub async fn remove_upload_source(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    removed_source(app, slug, "upload".to_owned(), request).await
}

async fn removed_source(app: App, slug: String, source_id: String, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.remove_source(&slug, &source_id, &principal.checked()?).await?)
}

/// Managed turns, newest first.
#[utoipa::path(get, path = "/v1/projects/{slug}/runs", operation_id = "list_runs_v1_projects__slug__runs_get",
    summary = "List Runs", params(("slug" = String, Path), StatusPageParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = RuntimeRunPage), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn runs(
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
        .runs(
            &slug,
            status.checked()?.as_deref(),
            cursor.checked()?.as_deref(),
            limit.checked()?,
            &principal.checked()?,
        )
        .await?;
    ok(page)
}

/// One managed turn with the public events it recorded.
#[utoipa::path(get, path = "/v1/projects/{slug}/runs/{run_id}",
    operation_id = "get_run_v1_projects__slug__runs__run_id__get", summary = "Get Run",
    params(("slug" = String, Path), ("run_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = RuntimeRunDetail), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn run(
    State(app): State<App>,
    Params((slug, run_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().run(&slug, &run_id, &principal.checked()?).await?)
}

/// Runs, knowledge and guidance over the last `days` days.
#[utoipa::path(get, path = "/v1/projects/{slug}/analytics",
    operation_id = "project_analytics_v1_projects__slug__analytics_get", summary = "Project Analytics",
    params(("slug" = String, Path), AnalyticsParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = AnalyticsResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn analytics(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let days = call.query_int("days", 30, Int::between(1, 365));
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.analytics(&slug, days.checked()?, &principal.checked()?).await?)
}
