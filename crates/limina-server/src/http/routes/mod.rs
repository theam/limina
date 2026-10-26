//! The route handlers and the routing table.

pub mod accounts;
pub mod connections;
pub mod internal;
pub mod knowledge;
pub mod progress;
pub mod projects;
pub mod resources;
pub mod runtime;
pub mod sharing;
pub mod upload;
pub mod workspace;

use axum::Json;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde::Serialize;
use serde::de::DeserializeOwned;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::App;
use super::error::{ApiResult, not_found};

/// A route's path parameters; a path that cannot be decoded is a JSON 404.
pub struct Params<T>(pub T);

impl<S, T> FromRequestParts<S> for Params<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(value)) => Ok(Self(value)),
            Err(_) => Err(not_found()),
        }
    }
}

/// The request's head; the body of routes without one is ignored.
pub fn parts(request: Request) -> Parts {
    request.into_parts().0
}

/// `200 OK` with a JSON body.
pub fn ok<T: Serialize>(value: T) -> ApiResult {
    Ok(Json(value).into_response())
}

/// `201 Created` with a JSON body.
pub fn created<T: Serialize>(value: T) -> ApiResult {
    with_status(StatusCode::CREATED, &value)
}

pub fn with_status<T: Serialize>(status: StatusCode, value: &T) -> ApiResult {
    Ok((status, Json(value)).into_response())
}

/// The documented public API, in the Python API's order.
pub fn public(upload_body_limit: usize) -> OpenApiRouter<App> {
    let (schemas, paths, upload) = routes!(upload::upload_source);
    let upload = (schemas, paths, upload.layer(DefaultBodyLimit::max(upload_body_limit)));
    OpenApiRouter::new()
        .routes(routes!(connections::list, connections::create))
        .routes(routes!(connections::get, connections::update, connections::remove))
        .routes(routes!(connections::disconnect))
        .routes(routes!(connections::connect))
        .routes(routes!(connections::project))
        .routes(routes!(connections::enable, connections::disable))
        .routes(routes!(runtime::live))
        .routes(routes!(runtime::ready))
        .routes(routes!(runtime::health))
        .routes(routes!(runtime::me))
        .routes(routes!(runtime::engines))
        .routes(routes!(runtime::models))
        .routes(routes!(runtime::codex_status, runtime::codex_logout))
        .routes(routes!(runtime::codex_login))
        .routes(routes!(runtime::codex_login_attempt, runtime::codex_cancel_login))
        .routes(routes!(accounts::list))
        .routes(routes!(accounts::connect_claude))
        .route(
            "/v1/projects/{slug}/engine-accounts/claude-code",
            delete(accounts::disconnect_claude),
        )
        .routes(routes!(accounts::start_claude_login))
        .routes(routes!(
            accounts::claude_login,
            accounts::submit_claude_code,
            accounts::cancel_claude_login
        ))
        .routes(routes!(accounts::start_codex_login))
        .routes(routes!(accounts::codex_login, accounts::cancel_codex_login))
        .routes(routes!(accounts::disconnect))
        .routes(routes!(projects::create_project, projects::list_projects))
        .routes(routes!(projects::templates))
        .routes(routes!(projects::get_project, projects::update_project))
        .routes(routes!(projects::status))
        .routes(routes!(projects::preflight))
        .routes(routes!(projects::action))
        .routes(routes!(projects::steer))
        .routes(routes!(projects::guidance))
        .routes(routes!(projects::review))
        .routes(routes!(knowledge::query))
        .routes(routes!(knowledge::document))
        .routes(routes!(progress::list))
        .routes(routes!(progress::set))
        .routes(routes!(knowledge::graph))
        .routes(routes!(knowledge::create_relation))
        .route("/v1/projects/{slug}/knowledge/relations", get(knowledge::get_relations_artifact))
        .routes(routes!(knowledge::delete_relation))
        .routes(routes!(knowledge::saved_views, knowledge::save_view))
        .routes(routes!(knowledge::delete_view))
        .routes(routes!(knowledge::get))
        .routes(routes!(knowledge::revisions))
        .routes(routes!(knowledge::comments, knowledge::add_comment))
        .routes(routes!(knowledge::tags))
        .routes(routes!(knowledge::add_tag, knowledge::remove_tag))
        .routes(routes!(projects::events))
        .routes(routes!(projects::snapshot))
        .routes(routes!(workspace::assessment))
        .routes(routes!(workspace::history))
        .routes(routes!(workspace::outputs))
        .routes(routes!(workspace::preview))
        .routes(routes!(workspace::download))
        .routes(routes!(resources::set_variable))
        .routes(routes!(resources::set_secret))
        .routes(routes!(resources::list_resources))
        .routes(routes!(resources::remove_resource))
        .routes(routes!(resources::members, resources::set_member, resources::remove_member))
        .routes(routes!(sharing::sharing, sharing::share, sharing::cancel_invitation))
        .routes(routes!(sharing::shared_projects))
        .routes(routes!(resources::live_ticket))
        .routes(routes!(resources::sources, resources::set_source))
        .routes(upload)
        .route("/v1/projects/{slug}/sources/upload", delete(resources::remove_upload_source))
        .routes(routes!(resources::remove_source))
        .routes(routes!(resources::runs))
        .routes(routes!(resources::run))
        .routes(routes!(resources::analytics))
}

/// The internal agent API (`/internal/v1/…`), not documented.
pub fn internal() -> axum::Router<App> {
    let project = "/internal/v1/projects/{slug}";
    axum::Router::new()
        .route(&format!("{project}/status"), get(internal::status))
        .route(&format!("{project}/artifacts"), get(internal::artifacts))
        .route(&format!("{project}/artifacts/{{artifact_id}}"), get(internal::artifact))
        .route(&format!("{project}/hypotheses"), post(internal::create_hypothesis))
        .route(
            &format!("{project}/hypotheses/{{artifact_id}}/decision"),
            post(internal::decide_hypothesis),
        )
        .route(&format!("{project}/experiments"), post(internal::create_experiment))
        .route(
            &format!("{project}/experiments/{{artifact_id}}/claim"),
            post(internal::claim_experiment),
        )
        .route(
            &format!("{project}/experiments/{{artifact_id}}/observations"),
            post(internal::observe_experiment),
        )
        .route(
            &format!("{project}/experiments/{{artifact_id}}/complete"),
            post(internal::complete_experiment),
        )
        .route(&format!("{project}/findings"), post(internal::publish_finding))
}
