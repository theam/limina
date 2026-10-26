//! A project's own provider accounts (`engine_account_api.py`). Viewers see which account a
//! project runs on; only owners connect or disconnect one. Tokens are write-only.

use axum::extract::{Request, State};

use super::{Params, ok, parts};
use crate::app::PUBLIC_COMMAND_ID_LIMIT;
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::requests::{ClaudeLoginCodeRequest, EngineTokenRequest};
use crate::http::schemas::{
    ActorHeader, CodexDeviceLogin, CommandHeader, ProjectEngineAccountResponse, PublicErrors,
};

#[utoipa::path(get, path = "/v1/projects/{slug}/engine-accounts",
    operation_id = "list_engine_accounts_v1_projects__slug__engine_accounts_get", summary = "List Engine Accounts",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<ProjectEngineAccountResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn list(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().engine_accounts(&slug, &principal.checked()?).await?)
}

/// Connects a Claude subscription token or API key to the project (owners).
#[utoipa::path(put, path = "/v1/projects/{slug}/engine-accounts/claude-code",
    operation_id = "connect_claude_account_v1_projects__slug__engine_accounts_claude_code_put",
    summary = "Connect Claude Account", params(("slug" = String, Path), ActorHeader),
    request_body = EngineTokenRequest,
    responses((status = 200, description = "Successful Response", body = ProjectEngineAccountResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn connect_claude(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let token = EngineTokenRequest::parse(&body, call.errors());
    call.check()?;
    let operations = app.runtime.operations();
    let account = operations
        .connect_claude_account(&slug, &token.checked()?.token, &principal.checked()?)
        .await?;
    ok(account)
}

/// Starts a Claude sign-in for the project's own account (owners). Open `verification_url`,
/// sign in with your Claude account, then send the code claude.com shows.
#[utoipa::path(post, path = "/v1/projects/{slug}/engine-accounts/claude-code/login",
    operation_id = "start_claude_account_login_v1_projects__slug__engine_accounts_claude_code_login_post",
    summary = "Start Claude Account Login", params(("slug" = String, Path), CommandHeader, ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn start_claude_login(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    let login = operations
        .start_claude_account_login(&slug, &command_id.checked()?, &principal.checked()?)
        .await?;
    ok(login)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/engine-accounts/claude-code/login/{login_id}",
    operation_id = "claude_account_login_v1_projects__slug__engine_accounts_claude_code_login__login_id__get",
    summary = "Claude Account Login", params(("slug" = String, Path), ("login_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn claude_login(
    State(app): State<App>,
    Params((slug, login_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.claude_account_login(&slug, &login_id, &principal.checked()?).await?)
}

/// Completes a Claude sign-in with the code claude.com showed (owners).
#[utoipa::path(put, path = "/v1/projects/{slug}/engine-accounts/claude-code/login/{login_id}",
    operation_id = "submit_claude_account_code_v1_projects__slug__engine_accounts_claude_code_login__login_id__put",
    summary = "Submit Claude Account Code", params(("slug" = String, Path), ("login_id" = String, Path), ActorHeader),
    request_body = ClaudeLoginCodeRequest,
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn submit_claude_code(
    State(app): State<App>,
    Params((slug, login_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let code = ClaudeLoginCodeRequest::parse(&body, call.errors());
    call.check()?;
    let operations = app.runtime.operations();
    let attempt = operations
        .submit_claude_account_code(&slug, &login_id, &code.checked()?.code, &principal.checked()?)
        .await?;
    ok(attempt)
}

#[utoipa::path(delete, path = "/v1/projects/{slug}/engine-accounts/claude-code/login/{login_id}",
    operation_id = "cancel_claude_account_login_v1_projects__slug__engine_accounts_claude_code_login__login_id__delete",
    summary = "Cancel Claude Account Login",
    params(("slug" = String, Path), ("login_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn cancel_claude_login(
    State(app): State<App>,
    Params((slug, login_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.cancel_claude_account_login(&slug, &login_id, &principal.checked()?).await?)
}

/// Starts a ChatGPT device login for the project's own Codex account (owners).
#[utoipa::path(post, path = "/v1/projects/{slug}/engine-accounts/codex/login",
    operation_id = "start_codex_account_login_v1_projects__slug__engine_accounts_codex_login_post",
    summary = "Start Codex Account Login", params(("slug" = String, Path), CommandHeader, ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn start_codex_login(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    let login = operations
        .start_codex_account_login(&slug, &command_id.checked()?, &principal.checked()?)
        .await?;
    ok(login)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/engine-accounts/codex/login/{login_id}",
    operation_id = "codex_account_login_v1_projects__slug__engine_accounts_codex_login__login_id__get",
    summary = "Codex Account Login", params(("slug" = String, Path), ("login_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn codex_login(
    State(app): State<App>,
    Params((slug, login_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.codex_account_login(&slug, &login_id, &principal.checked()?).await?)
}

#[utoipa::path(delete, path = "/v1/projects/{slug}/engine-accounts/codex/login/{login_id}",
    operation_id = "cancel_codex_account_login_v1_projects__slug__engine_accounts_codex_login__login_id__delete",
    summary = "Cancel Codex Account Login",
    params(("slug" = String, Path), ("login_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = CodexDeviceLogin), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn cancel_codex_login(
    State(app): State<App>,
    Params((slug, login_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.cancel_codex_account_login(&slug, &login_id, &principal.checked()?).await?)
}

/// Disconnects the project's own account for an engine (owners); turns use the runtime's
/// credentials again.
#[utoipa::path(delete, path = "/v1/projects/{slug}/engine-accounts/{engine}",
    operation_id = "disconnect_engine_account_v1_projects__slug__engine_accounts__engine__delete",
    summary = "Disconnect Engine Account", params(("slug" = String, Path), ("engine" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = ProjectEngineAccountResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn disconnect(
    State(app): State<App>,
    Params((slug, engine)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    disconnected(app, slug, engine, request).await
}

/// `DELETE …/engine-accounts/claude-code`: the static route only takes `PUT`, so this is the
/// `{engine}` route for Claude (as the Python router matched it).
pub async fn disconnect_claude(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    disconnected(app, slug, "claude-code".to_owned(), request).await
}

async fn disconnected(app: App, slug: String, engine: String, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.disconnect_engine_account(&slug, &engine, &principal.checked()?).await?)
}
