//! The internal research protocol for managed agents (`internal_api.py`): project-scoped,
//! authenticated by the turn's capability token (`Authorization: Bearer <capability>`, optional
//! `X-Limina-Agent-Lane`), and left out of the OpenAPI document.
//!
//! Every write wakes the project's live clients through the event notifier.

use axum::extract::{Request, State};
use axum::http::StatusCode;

use super::{Params, ok, parts, with_status};
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::requests;
use crate::http::validate::Body;

/// The internal API's `Idempotency-Key` limit (the full receipt width).
pub const INTERNAL_COMMAND_ID_LIMIT: usize = 64;

/// The capability's actor (`internal_actor`): 401 without a valid capability for `slug`.
fn actor(app: &App, call: &Call, slug: &str) -> ApiResult<String> {
    let authorization = call.header("authorization");
    let lane = call.header("x-limina-agent-lane");
    Ok(app.runtime.internal_actor(authorization.as_deref(), slug, lane.as_deref())?)
}

/// A command's actor, `Idempotency-Key` and body, validated in the Python dependency order.
async fn command(
    app: &App,
    slug: &str,
    request: Request,
) -> ApiResult<(Call, String, String, Body)> {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let actor = actor(app, &call, slug)?;
    let command_id = call.command_id(INTERNAL_COMMAND_ID_LIMIT);
    Ok((call, actor, command_id.unwrap_or_default(), body))
}

pub async fn status(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let call = Call::new(parts(request));
    actor(&app, &call, &slug)?;
    ok(app.runtime.service().status(&slug).await?)
}

pub async fn artifacts(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let call = Call::new(parts(request));
    actor(&app, &call, &slug)?;
    let kind = call.query("kind").map(str::to_owned);
    ok(app.runtime.service().list_artifacts(&slug, kind.as_deref()).await?)
}

pub async fn artifact(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let call = Call::new(parts(request));
    actor(&app, &call, &slug)?;
    ok(app.runtime.service().get_artifact(&slug, &artifact_id).await?)
}

pub async fn create_hypothesis(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (mut call, actor, command_id, body) = command(&app, &slug, request).await?;
    let hypothesis = requests::hypothesis(&body, call.errors());
    call.check()?;
    let service = app.runtime.service();
    let created =
        service.create_hypothesis(&slug, hypothesis.checked()?, &actor, &command_id).await?;
    app.runtime.notifier().notify(&slug);
    with_status(StatusCode::CREATED, &created)
}

pub async fn decide_hypothesis(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (mut call, actor, command_id, body) = command(&app, &slug, request).await?;
    let decision = requests::hypothesis_decision(&body, call.errors());
    call.check()?;
    let service = app.runtime.service();
    let decided = service
        .decide_hypothesis(&slug, &artifact_id, decision.checked()?, &actor, &command_id)
        .await?;
    app.runtime.notifier().notify(&slug);
    ok(decided)
}

pub async fn create_experiment(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (mut call, actor, command_id, body) = command(&app, &slug, request).await?;
    let experiment = requests::experiment(&body, call.errors());
    call.check()?;
    let service = app.runtime.service();
    let created =
        service.create_experiment(&slug, experiment.checked()?, &actor, &command_id).await?;
    app.runtime.notifier().notify(&slug);
    with_status(StatusCode::CREATED, &created)
}

pub async fn claim_experiment(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (mut call, actor, command_id, body) = command(&app, &slug, request).await?;
    let ttl = requests::claim_ttl(&body, call.errors());
    call.check()?;
    let service = app.runtime.service();
    let claim =
        service.claim_experiment(&slug, &artifact_id, ttl.checked()?, &actor, &command_id).await?;
    app.runtime.notifier().notify(&slug);
    ok(claim)
}

pub async fn observe_experiment(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (mut call, actor, command_id, body) = command(&app, &slug, request).await?;
    let observation = requests::observation(&body, call.errors());
    call.check()?;
    let (text, evidence_ref) = observation.checked()?;
    let service = app.runtime.service();
    let observed = service
        .append_observation(
            &slug,
            &artifact_id,
            &text,
            evidence_ref.as_deref(),
            &actor,
            &command_id,
        )
        .await?;
    app.runtime.notifier().notify(&slug);
    with_status(StatusCode::CREATED, &observed)
}

pub async fn complete_experiment(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (mut call, actor, command_id, body) = command(&app, &slug, request).await?;
    let outcome = requests::experiment_outcome(&body, call.errors());
    call.check()?;
    let service = app.runtime.service();
    let completed = service
        .complete_experiment(&slug, &artifact_id, outcome.checked()?, &actor, &command_id)
        .await?;
    app.runtime.notifier().notify(&slug);
    ok(completed)
}

pub async fn publish_finding(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (mut call, actor, command_id, body) = command(&app, &slug, request).await?;
    let finding = requests::finding(&body, call.errors());
    call.check()?;
    let service = app.runtime.service();
    let published = service.publish_finding(&slug, finding.checked()?, &actor, &command_id).await?;
    app.runtime.notifier().notify(&slug);
    with_status(StatusCode::CREATED, &published)
}
