//! Explicit, evidence-backed project outcome measurements.

use axum::extract::{Request, State};
use limina_domain::progress::MetricInput;

use super::{Params, ok, parts};
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::schemas::{ActorHeader, CommandHeader, PublicErrors};
use crate::http::validate::{Body, Errors, Fields, Int};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Debug, ToSchema)]
pub struct MetricRequest {
    /// KPI fields: name, criterion, unit, direction (at_least/at_most), target, optional
    /// baseline/current, evidence_id, observed_at (ISO timestamp), and notes.
    #[schema(value_type = MetricFields)]
    pub metric: Value,
    /// 0 creates a KPI; edits require the version returned by the last read.
    pub expected_version: i32,
}

#[derive(Debug, serde::Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    AtLeast,
    AtMost,
}

#[derive(Debug, ToSchema)]
pub struct MetricFields {
    #[schema(min_length = 1, max_length = 120)]
    pub name: String,
    #[schema(max_length = 2000, required = false, default = "")]
    pub criterion: String,
    #[schema(max_length = 40, required = false, default = "")]
    pub unit: String,
    #[schema(inline)]
    pub direction: MetricDirection,
    pub target: f64,
    #[schema(required = false)]
    pub baseline: Option<f64>,
    #[schema(required = false)]
    pub current: Option<f64>,
    #[schema(required = false)]
    pub evidence_id: Option<String>,
    #[schema(required = false)]
    pub observed_at: Option<String>,
    #[schema(max_length = 4000, required = false, default = "")]
    pub notes: String,
}

impl MetricRequest {
    fn parse(body: &Body, errors: &mut Errors) -> Option<Self> {
        let mut fields = Fields::open(body, errors)?;
        let metric = fields.object("metric", true);
        let version = fields.int("expected_version", None, Int::between(0, i64::from(i32::MAX)));
        fields.finish();
        Some(Self {
            metric: Value::Object(metric?),
            expected_version: i32::try_from(version?).ok()?,
        })
    }
}

#[derive(Debug, ToSchema)]
pub struct MetricResponse {
    pub id: String,
    pub name: String,
    pub criterion: String,
    pub unit: String,
    pub direction: String,
    pub target: f64,
    pub baseline: Option<f64>,
    pub current: Option<f64>,
    pub evidence_id: Option<String>,
    pub observed_at: Option<String>,
    pub notes: String,
    pub version: i32,
    pub updated_by: String,
    pub updated_at: String,
}

#[utoipa::path(get, path = "/v1/projects/{slug}/metrics", operation_id = "project_metrics",
    summary = "Project KPIs", params(("slug" = String, Path), ActorHeader),
    responses((status = 200, body = Vec<MetricResponse>), PublicErrors), security(("HTTPBearer" = [])))]
pub async fn list(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().metrics(&slug, &principal.checked()?).await?)
}

#[utoipa::path(put, path = "/v1/projects/{slug}/metrics/{metric_id}", operation_id = "set_project_metric",
    summary = "Define or Measure a Project KPI",
    params(("slug" = String, Path), ("metric_id" = String, Path), ActorHeader, CommandHeader),
    request_body = MetricRequest, responses((status = 200, body = MetricResponse), PublicErrors), security(("HTTPBearer" = [])))]
pub async fn set(
    State(app): State<App>,
    Params((slug, id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let command_id = call.command_id(crate::app::PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    let input = MetricRequest::parse(&body, call.errors());
    call.check()?;
    let input = input.checked()?;
    ok(app
        .runtime
        .operations()
        .set_metric(
            &slug,
            &id,
            MetricInput::parse(input.metric)?,
            input.expected_version,
            &command_id.checked()?,
            &principal.checked()?,
        )
        .await?)
}
