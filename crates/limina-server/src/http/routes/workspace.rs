//! Workspace context and published deliverables; shared authorization lives in operations.
use super::{Params, ok, parts};
use crate::http::{
    App,
    call::{Call, Checked},
    error::ApiResult,
    validate::Int,
};
use axum::extract::{Request, State};
use axum::response::IntoResponse;

#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/{artifact_id}/history", operation_id = "knowledge_history",
    params(("slug" = String, Path), ("artifact_id" = String, Path), ("before" = Option<i32>, Query)), responses((status = 200, body = Object)), security(("HTTPBearer" = [])))]
pub async fn history(
    State(app): State<App>,
    Params((slug, id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let before =
        call.query_int("before", i64::from(i32::MAX), Int::between(1, i64::from(i32::MAX)));
    call.check()?;
    ok(app
        .runtime
        .operations()
        .knowledge_history(&slug, &id, before.checked()? as i32, &principal.checked()?)
        .await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/workspace", operation_id = "project_workspace",
    params(("slug" = String, Path)), responses((status = 200, body = Object)), security(("HTTPBearer" = [])))]
pub async fn assessment(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().workspace_assessment(&slug, &principal.checked()?).await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/outputs", operation_id = "project_outputs",
    params(("slug" = String, Path), ("offset" = Option<i64>, Query)), responses((status = 200, body = Object)), security(("HTTPBearer" = [])))]
pub async fn outputs(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let offset = call.query_int("offset", 0, Int::between(0, 1000));
    call.check()?;
    ok(app
        .runtime
        .operations()
        .project_outputs(&slug, offset.checked()? as usize, &principal.checked()?)
        .await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/outputs/{name}", operation_id = "preview_project_output",
    params(("slug" = String, Path), ("name" = String, Path)), responses((status = 200, body = Object)), security(("HTTPBearer" = [])))]
pub async fn preview(
    State(app): State<App>,
    Params((slug, name)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().output_preview(&slug, &name, &principal.checked()?).await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/outputs/{name}/download", operation_id = "download_project_output",
    params(("slug" = String, Path), ("name" = String, Path)), responses((status = 200, body = String, content_type = "application/octet-stream")), security(("HTTPBearer" = [])))]
pub async fn download(
    State(app): State<App>,
    Params((slug, name)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let bytes = app.runtime.operations().read_output(&slug, &name, &principal.checked()?).await?;
    Ok((
        [
            ("content-type", "application/octet-stream"),
            ("content-disposition", "attachment"),
            ("x-content-type-options", "nosniff"),
            ("cache-control", "private, no-store"),
        ],
        bytes,
    )
        .into_response())
}
