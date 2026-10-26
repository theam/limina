//! One API call: its headers, query string and body, read in the order FastAPI resolved a
//! route's dependencies so that errors come out the same way.
//!
//! FastAPI parsed a JSON body first (a syntax error is a 422 before anything else), then resolved
//! the dependencies in parameter order — the `Idempotency-Key` header, then the caller (an
//! over-long `X-Limina-Actor` is a validation error and skips authentication; any other
//! authentication failure is answered at once) — then the route's query parameters and the body
//! fields. Validation errors are collected and answered together as one 422.

use axum::body::{Body as HttpBody, Bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use bytes::BytesMut;
use futures::StreamExt;
use limina_core::{LiminaError, Principal, internal};
use serde_json::{Value, json};

use super::error::{ApiError, ApiResult};
use super::validate::{Body, Errors, Int, Loc, Text, check_text, parse_bool, text_int};
use crate::auth::request::actor_hint;
use crate::auth::{ACTOR_HEADER, ACTOR_LIMIT, RequestAuth};

/// The request context every handler works with.
pub struct Call {
    pub parts: Parts,
    errors: Errors,
    query: Vec<(String, String)>,
}

impl Call {
    pub fn new(parts: Parts) -> Self {
        let query = url::form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
        Self { parts, errors: Errors::new(), query }
    }

    pub fn errors(&mut self) -> &mut Errors {
        &mut self.errors
    }

    /// Answers the collected validation errors, if any.
    pub fn check(&mut self) -> ApiResult<()> {
        if self.errors.is_empty() {
            return Ok(());
        }
        Err(ApiError::Invalid(std::mem::take(&mut self.errors).into_vec()))
    }

    /// The first value of a header, as text.
    pub fn header(&self, name: &str) -> Option<String> {
        self.parts
            .headers
            .get(name)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
    }

    /// `Idempotency-Key`: required, 1 to `limit` characters (55 on the public API, 64 on the
    /// internal one).
    pub fn command_id(&mut self, limit: usize) -> Option<String> {
        let loc = Loc::header("idempotency-key");
        let Some(value) = self.header("idempotency-key") else {
            self.errors.missing(&loc, Value::Null);
            return None;
        };
        check_text(&mut self.errors, &loc, &value, Text::between(1, limit)).then_some(value)
    }

    /// The authenticated caller (`require_auth`). `Ok(None)` when `X-Limina-Actor` is too long:
    /// that is a validation error, recorded here, and authentication does not run.
    pub async fn caller(&mut self) -> ApiResult<Option<Principal>> {
        if let Some(actor) = actor_hint(&self.parts.headers)
            && actor.chars().count() > ACTOR_LIMIT
        {
            self.errors.push_ctx(
                "string_too_long",
                &Loc::header(ACTOR_HEADER),
                format!("String should have at most {ACTOR_LIMIT} characters"),
                json!(actor),
                json!({"max_length": ACTOR_LIMIT}),
            );
            return Ok(None);
        }
        let context = self
            .parts
            .extensions
            .get::<RequestAuth>()
            .cloned()
            .ok_or_else(|| internal("the authentication layer is not installed"))?;
        Ok(Some(context.principal(&self.parts).await?))
    }

    /// A caller who is an instance administrator (`require_instance_admin`).
    pub async fn admin(&mut self) -> ApiResult<Option<Principal>> {
        let principal = self.caller().await?;
        if let Some(principal) = &principal
            && !principal.instance_admin
        {
            return Err(LiminaError::authorization_with(
                "Instance administrator access is required.",
            )
            .into());
        }
        Ok(principal)
    }

    /// The last value of a query parameter (Starlette's `QueryParams.get`).
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query.iter().rev().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    /// A boolean query parameter.
    pub fn query_bool(&mut self, name: &str, default: bool) -> Option<bool> {
        let Some(text) = self.query(name).map(str::to_owned) else { return Some(default) };
        let parsed = parse_bool(&text);
        if parsed.is_none() {
            self.errors.push(
                "bool_parsing",
                &Loc::query(name),
                "Input should be a valid boolean, unable to interpret input",
                json!(text),
            );
        }
        parsed
    }

    /// An integer query parameter with a default.
    pub fn query_int(&mut self, name: &str, default: i64, rule: Int) -> Option<i64> {
        match self.query(name).map(str::to_owned) {
            None => Some(default),
            Some(text) => text_int(&mut self.errors, &Loc::query(name), &text, rule),
        }
    }

    /// An optional integer query parameter.
    pub fn query_optional_int(&mut self, name: &str, rule: Int) -> Option<Option<i64>> {
        match self.query(name).map(str::to_owned) {
            None => Some(None),
            Some(text) => text_int(&mut self.errors, &Loc::query(name), &text, rule).map(Some),
        }
    }

    /// An optional text query parameter.
    pub fn query_text(&mut self, name: &str, rule: Text) -> Option<Option<String>> {
        match self.query(name).map(str::to_owned) {
            None => Some(None),
            Some(text) => {
                check_text(&mut self.errors, &Loc::query(name), &text, rule).then_some(Some(text))
            }
        }
    }

    /// A required text query parameter.
    pub fn query_required_text(&mut self, name: &str, rule: Text) -> Option<String> {
        let loc = Loc::query(name);
        match self.query(name).map(str::to_owned) {
            None => {
                self.errors.missing(&loc, Value::Null);
                None
            }
            Some(text) => check_text(&mut self.errors, &loc, &text, rule).then_some(text),
        }
    }
}

/// A value every validator produced once the errors were checked.
pub trait Checked<T> {
    fn checked(self) -> ApiResult<T>;
}

impl<T> Checked<T> for Option<T> {
    fn checked(self) -> ApiResult<T> {
        self.ok_or_else(|| ApiError::Limina(internal("request validation was inconsistent")))
    }
}

/// Reads a JSON route's body (at most `limit` bytes) the way FastAPI did: only `application/json`
/// and `application/*+json` bodies are parsed; any other body stays text for the error's
/// `input`. A JSON syntax error is answered immediately.
pub async fn json_body(parts: &Parts, body: HttpBody, limit: usize) -> ApiResult<Body> {
    let bytes = read_body(body, limit).await?;
    if bytes.is_empty() {
        return Ok(Body::Missing);
    }
    if !is_json(parts) {
        return Ok(Body::Text(String::from_utf8_lossy(&bytes).into_owned()));
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Err(ApiError::Unreadable);
    };
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Null) => Ok(Body::Missing),
        Ok(value) => Ok(Body::Json(value)),
        Err(error) => Err(ApiError::Invalid(vec![json_syntax_error(text, &error)])),
    }
}

/// The whole body, or 413 past `limit` (checked while reading, so nothing larger is buffered).
pub async fn read_body(body: HttpBody, limit: usize) -> ApiResult<Bytes> {
    let mut stream = body.into_data_stream();
    let mut buffer = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ApiError::Unreadable)?;
        if buffer.len().saturating_add(chunk.len()) > limit {
            return Err(ApiError::TooLarge {
                limit_bytes: u64::try_from(limit).unwrap_or(u64::MAX),
            });
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok(buffer.freeze())
}

fn is_json(parts: &Parts) -> bool {
    let Some(value) = parts.headers.get(CONTENT_TYPE) else { return false };
    let value = String::from_utf8_lossy(value.as_bytes()).to_ascii_lowercase();
    let essence = value.split(';').next().unwrap_or_default().trim();
    let Some((main, sub)) = essence.split_once('/') else { return false };
    main.trim() == "application" && (sub.trim() == "json" || sub.trim().ends_with("+json"))
}

/// FastAPI's `json_invalid` entry. The position is a character offset, like Python's
/// `JSONDecodeError.pos`; the message names the problem the way Python's `json` module does.
fn json_syntax_error(text: &str, error: &serde_json::Error) -> Value {
    let line_start: usize =
        text.split_inclusive('\n').take(error.line().saturating_sub(1)).map(str::len).sum();
    let byte = (line_start + error.column().saturating_sub(1)).min(text.len());
    let byte = (0..=byte).rev().find(|index| text.is_char_boundary(*index)).unwrap_or(0);
    let position = text[..byte].chars().count();
    let described = error.to_string();
    let reason = described.split(" at line ").next().unwrap_or(&described);
    json!({
        "type": "json_invalid",
        "loc": Loc::body().index(position).to_value(),
        "msg": "JSON decode error",
        "input": {},
        "ctx": {"error": python_json_message(reason)},
    })
}

fn python_json_message(reason: &str) -> String {
    let known = [
        ("key must be a string", "Expecting property name enclosed in double quotes"),
        ("trailing comma", "Illegal trailing comma before end of object"),
        ("expected `:`", "Expecting ':' delimiter"),
        ("expected `,` or `}`", "Expecting ',' delimiter"),
        ("expected `,` or `]`", "Expecting ',' delimiter"),
        ("expected value", "Expecting value"),
        ("expected ident", "Expecting value"),
        ("EOF while parsing a string", "Unterminated string starting at"),
        ("EOF while parsing", "Expecting value"),
        ("trailing characters", "Extra data"),
        ("invalid escape", "Invalid \\escape"),
        ("control character", "Invalid control character at"),
    ];
    known
        .iter()
        .find(|(prefix, _)| reason.starts_with(prefix))
        .map_or_else(|| reason.to_owned(), |(_, message)| (*message).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_syntax_errors_point_at_the_character() {
        let error = serde_json::from_str::<Value>("{not json").unwrap_err();
        let entry = json_syntax_error("{not json", &error);
        assert_eq!(entry["loc"], json!(["body", 1]));
        assert_eq!(entry["ctx"]["error"], "Expecting property name enclosed in double quotes");
        let text = "{\"é\": [1,}";
        let error = serde_json::from_str::<Value>(text).unwrap_err();
        assert_eq!(json_syntax_error(text, &error)["loc"][1].as_u64().unwrap(), 9);
    }
}
