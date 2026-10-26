//! Authentication errors as HTTP responses.

use axum::Json;
use axum::http::StatusCode;
use axum::http::header::{HeaderValue, RETRY_AFTER};
use axum::response::{IntoResponse, Response};
use limina_core::{ErrorKind, LiminaError};
use serde_json::Value;

/// The public error body (`{"error": {"code", "message", "details"}}`) with the error's status.
/// A `rate_limited` error also carries `Retry-After` (seconds), the same value as
/// `details.retry_after_seconds`.
pub fn error_response(error: &LiminaError) -> Response {
    let status =
        StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, Json(error.to_body())).into_response();
    if error.kind == ErrorKind::RateLimited
        && let Some(seconds) = error.details.get("retry_after_seconds").and_then(Value::as_u64)
    {
        response.headers_mut().insert(RETRY_AFTER, HeaderValue::from(seconds));
    }
    response
}

/// The rejection of the authentication extractors.
#[derive(Debug, Clone)]
pub struct AuthRejection(pub LiminaError);

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        error_response(&self.0)
    }
}

impl From<LiminaError> for AuthRejection {
    fn from(error: LiminaError) -> Self {
        Self(error)
    }
}
