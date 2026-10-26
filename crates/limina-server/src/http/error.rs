//! Error responses: the stable `{"error": {"code", "message", "details"}}` body for every
//! [`LiminaError`], FastAPI's 422 `invalid_request` body for request validation, and the
//! `{"detail": …}` bodies FastAPI gave for unknown routes, unsupported methods and unreadable
//! bodies.

use axum::Json;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use limina_core::{ErrorKind, LiminaError};
use serde_json::{Value, json};

use crate::auth::error_response;

/// The message of every request-validation error (FastAPI's `RequestValidationError` handler).
pub const INVALID_REQUEST_MESSAGE: &str = "The request did not match the API contract.";

/// Why a request was not handled.
#[derive(Debug)]
pub enum ApiError {
    /// A domain or authentication error, answered with its own status and stable body.
    Limina(LiminaError),
    /// Request validation failed: 422 `invalid_request` with these pydantic-style entries in
    /// `details.errors`.
    Invalid(Vec<Value>),
    /// The request body is larger than the route accepts (413).
    TooLarge { limit_bytes: u64 },
    /// The body could not be parsed at all (FastAPI's 400 `{"detail": …}`).
    Unreadable,
    /// No route matches (FastAPI's 404 `{"detail": "Not Found"}`).
    NotFound,
}

pub type ApiResult<T = Response> = Result<T, ApiError>;

impl From<LiminaError> for ApiError {
    fn from(error: LiminaError) -> Self {
        Self::Limina(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Limina(error) => {
                if error.http_status() >= 500 {
                    tracing::warn!(code = error.code(), message = %error.message, "request failed");
                }
                error_response(&error)
            }
            Self::Invalid(errors) => invalid_request(errors),
            Self::TooLarge { limit_bytes } => {
                let body = json!({
                    "error": {
                        "code": "payload_too_large",
                        "message": format!("The request body must be at most {limit_bytes} bytes."),
                        "details": {"limit_bytes": limit_bytes},
                    }
                });
                (StatusCode::PAYLOAD_TOO_LARGE, Json(body)).into_response()
            }
            Self::Unreadable => {
                detail(StatusCode::BAD_REQUEST, "There was an error parsing the body")
            }
            Self::NotFound => not_found(),
        }
    }
}

/// The 422 body FastAPI's validation handler produced.
pub fn invalid_request(errors: Vec<Value>) -> Response {
    let error = LiminaError::new(ErrorKind::Invariant, INVALID_REQUEST_MESSAGE)
        .with_code("invalid_request")
        .with("errors", Value::Array(errors));
    error_response(&error)
}

/// `{"detail": message}` with `status`, as Starlette's HTTP exceptions render.
pub fn detail(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"detail": message}))).into_response()
}

/// The JSON 404 for paths no route serves.
pub fn not_found() -> Response {
    detail(StatusCode::NOT_FOUND, "Not Found")
}

/// Gives axum's bodiless `405 Method Not Allowed` responses FastAPI's JSON body; the `Allow`
/// header axum computed is kept.
pub async fn method_not_allowed_body(response: Response) -> Response {
    if response.status() != StatusCode::METHOD_NOT_ALLOWED
        || response.headers().contains_key(CONTENT_TYPE)
    {
        return response;
    }
    let (mut parts, _) = response.into_parts();
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    parts.headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let body = serde_json::to_vec(&json!({"detail": "Method Not Allowed"})).unwrap_or_default();
    Response::from_parts(parts, axum::body::Body::from(body))
}
