//! Sharing a project by email, and the projects other people shared with the caller.
//!
//! The subject-based member routes (`/v1/projects/{slug}/members`) stay as they were for the CLI,
//! MCP and API clients; these routes add sharing by email and pending invitations.

use axum::extract::{Request, State};

use super::{Params, ok, parts};
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::requests::ShareRequest;
use crate::http::schemas::{
    ActorHeader, PublicErrors, ShareParams, ShareResponse, SharedProjectResponse,
};
use crate::http::validate::Text;

/// Everyone with access to the project: members, then pending invitations (viewers).
#[utoipa::path(get, path = "/v1/projects/{slug}/sharing",
    operation_id = "list_sharing_v1_projects__slug__sharing_get", summary = "List Sharing",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<ShareResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn sharing(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().sharing(&slug, &principal.checked()?).await?)
}

/// Shares the project by email with someone in your organization (owners). A person the
/// directory knows becomes a member at once; anyone else gets a pending invitation that applies
/// when they first sign in with that verified email. Sharing again changes the role.
#[utoipa::path(put, path = "/v1/projects/{slug}/sharing",
    operation_id = "share_project_v1_projects__slug__sharing_put", summary = "Share Project",
    params(("slug" = String, Path), ActorHeader), request_body = ShareRequest,
    responses((status = 200, description = "Successful Response", body = ShareResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn share(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let share = ShareRequest::parse(&body, call.errors());
    call.check()?;
    let share = share.checked()?;
    let shared = app
        .runtime
        .operations()
        .share_project(
            &slug,
            &share.email,
            &share.role,
            app.auth.directory(),
            &principal.checked()?,
        )
        .await?;
    ok(shared)
}

/// Withdraws a pending invitation (owners). Members are removed with `DELETE …/members`.
#[utoipa::path(delete, path = "/v1/projects/{slug}/sharing",
    operation_id = "cancel_invitation_v1_projects__slug__sharing_delete", summary = "Cancel Invitation",
    params(("slug" = String, Path), ShareParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = ShareResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn cancel_invitation(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let email = call.query_required_text("email", Text::between(3, 320));
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.cancel_invitation(&slug, &email.checked()?, &principal.checked()?).await?)
}

/// The projects other people shared with you, with their owners.
#[utoipa::path(get, path = "/v1/me/shared-projects",
    operation_id = "shared_projects_v1_me_shared_projects_get", summary = "Shared Projects",
    params(ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<SharedProjectResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn shared_projects(State(app): State<App>, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().shared_projects(&principal.checked()?).await?)
}
