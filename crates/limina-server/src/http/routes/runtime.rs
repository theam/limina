//! Probes, the caller's identity, engine readiness and the runtime's own Codex sign-in
//! (`runtime_api.py`).

use axum::Json;
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use limina_domain::engines::SUPPORTED_ENGINES;
use serde_json::json;

use super::{ok, parts};
use crate::app::PUBLIC_COMMAND_ID_LIMIT;
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::requests::CodexAuthLoginRequest;
use crate::http::schemas::{
    ActorHeader, CodexAuthStatus, CodexDeviceLogin, CodexLoginResponse, CommandHeader,
    EngineAvailability, HealthResponse, MeResponse, PublicErrors,
};

/// Liveness: the process answers.
#[utoipa::path(get, path = "/livez", operation_id = "live_livez_get", summary = "Live",
    responses((status = 200, description = "Successful Response", body = std::collections::BTreeMap<String, bool>)))]
pub async fn live() -> Response {
    Json(json!({"ok": true})).into_response()
}

/// Readiness: the database answers (503 otherwise).
#[utoipa::path(get, path = "/readyz", operation_id = "ready_readyz_get", summary = "Ready",
    responses((status = 200, description = "Successful Response", body = serde_json::Value)))]
pub async fn ready(State(app): State<App>) -> Response {
    match app.runtime.db().ping().await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(error) => {
            tracing::warn!(%error, "readiness check failed");
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"ok": false}))).into_response()
        }
    }
}

/// The runtime's version, sign-in mode, engines and interfaces.
#[utoipa::path(get, path = "/healthz", operation_id = "health_healthz_get", summary = "Health",
    params(ActorHeader),
    responses((status = 200, description = "Successful Response", body = HealthResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn health(State(app): State<App>, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    call.caller().await?;
    call.check()?;
    let runtimes: Vec<&str> = SUPPORTED_ENGINES.iter().map(|engine| engine.id()).collect();
    ok(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "runtime_owner": "limina",
        "auth_mode": app.auth.mode(),
        "runtimes": runtimes,
        "interfaces": {"rest": "/v1", "mcp": "/mcp/"},
    }))
}

/// Who the caller is.
#[utoipa::path(get, path = "/v1/me", operation_id = "me_v1_me_get", summary = "Me",
    params(ActorHeader),
    responses((status = 200, description = "Successful Response", body = MeResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn me(request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let principal = principal.checked()?;
    ok(json!({
        "subject": principal.subject,
        "display_name": principal.display_name,
        "email": principal.email,
        "instance_admin": principal.instance_admin,
        "auth_mode": principal.auth_mode,
    }))
}

/// The runtime's own readiness for every engine (credentials are never returned).
#[utoipa::path(get, path = "/v1/runtime/engines", operation_id = "runtime_engines_v1_runtime_engines_get",
    summary = "Runtime Engines", tag = "runtime", params(ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<EngineAvailability>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn engines(State(app): State<App>, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().engine_availability().await)
}

/// Model choices and reasoning levels, without starting a provider turn.
#[utoipa::path(get, path = "/v1/runtime/engines/{engine}/models",
    operation_id = "engine_models", summary = "Engine Models", tag = "runtime",
    params(("engine" = String, Path), ("project" = Option<String>, Query, description = "Use a project account; requires project membership."), ActorHeader),
    responses((status = 200, body = crate::http::schemas::ModelCatalog), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn models(
    State(app): State<App>,
    Path(engine): Path<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let project = call.query_text("project", crate::http::validate::Text::between(1, 120));
    call.check()?;
    let engine = limina_domain::Engine::parse(&engine)?;
    ok(app
        .runtime
        .operations()
        .model_catalog(engine, project.checked()?.as_deref(), &principal.checked()?)
        .await?)
}

/// The runtime's Codex account (instance administrators).
#[utoipa::path(get, path = "/v1/runtime/engines/codex/auth",
    operation_id = "codex_auth_status_v1_runtime_engines_codex_auth_get", summary = "Codex Auth Status",
    tag = "runtime administration", params(ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexAuthStatus), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn codex_status(State(app): State<App>, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.admin().await?;
    call.check()?;
    ok(app.runtime.operations().runtime_codex_status(&principal.checked()?).await?)
}

/// Signs the runtime in to Codex: a ChatGPT device login, or the server's API key or access
/// token.
#[utoipa::path(post, path = "/v1/runtime/engines/codex/auth/login",
    operation_id = "codex_auth_login_v1_runtime_engines_codex_auth_login_post", summary = "Codex Auth Login",
    tag = "runtime administration", params(CommandHeader, ActorHeader), request_body = CodexAuthLoginRequest,
    responses((status = 200, description = "Successful Response", body = inline(CodexLoginResponse)), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn codex_login(State(app): State<App>, request: Request) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.admin().await?;
    let login = CodexAuthLoginRequest::parse(&body, call.errors());
    call.check()?;
    let outcome = app
        .runtime
        .operations()
        .runtime_codex_login(
            &login.checked()?.method,
            &command_id.checked()?,
            &principal.checked()?,
        )
        .await?;
    ok(outcome)
}

/// One device login of the runtime's Codex account.
#[utoipa::path(get, path = "/v1/runtime/engines/codex/auth/login/{login_id}",
    operation_id = "codex_auth_login_attempt_v1_runtime_engines_codex_auth_login__login_id__get",
    summary = "Codex Auth Login Attempt", tag = "runtime administration",
    params(("login_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn codex_login_attempt(
    State(app): State<App>,
    Path(login_id): Path<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.admin().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.runtime_codex_login_attempt(&login_id, &principal.checked()?).await?)
}

/// Cancels a pending device login of the runtime's Codex account.
#[utoipa::path(delete, path = "/v1/runtime/engines/codex/auth/login/{login_id}",
    operation_id = "cancel_codex_auth_login_v1_runtime_engines_codex_auth_login__login_id__delete",
    summary = "Cancel Codex Auth Login", tag = "runtime administration",
    params(("login_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn codex_cancel_login(
    State(app): State<App>,
    Path(login_id): Path<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.admin().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.runtime_codex_cancel_login(&login_id, &principal.checked()?).await?)
}

/// Signs the runtime's Codex account out.
#[utoipa::path(delete, path = "/v1/runtime/engines/codex/auth",
    operation_id = "codex_auth_logout_v1_runtime_engines_codex_auth_delete", summary = "Codex Auth Logout",
    tag = "runtime administration", params(ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexAuthStatus), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn codex_logout(State(app): State<App>, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.admin().await?;
    call.check()?;
    ok(app.runtime.operations().runtime_codex_logout(&principal.checked()?).await?)
}
