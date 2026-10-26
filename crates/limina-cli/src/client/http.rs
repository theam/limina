//! `HttpRuntimeClient`: the public REST API and the internal agent API over HTTP.

use std::time::Duration;

use limina_core::{ErrorKind, LiminaError, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use serde::Serialize;
use serde_json::{Map, Value};

/// The per-operation timeout the Python client used (`httpx.Client(timeout=30.0)`).
pub const TIMEOUT: Duration = Duration::from_secs(30);

const ACTOR_HEADER: &str = "x-limina-actor";
const LANE_HEADER: &str = "x-limina-agent-lane";
const IDEMPOTENCY_HEADER: &str = "idempotency-key";

/// A client for one Limina instance. Every write carries an `Idempotency-Key` (the command ID)
/// and, for project commands, the `X-Limina-Actor` hint.
#[derive(Debug, Clone)]
pub struct HttpRuntimeClient {
    /// The instance URL without a trailing slash, as given.
    base: String,
    /// The parsed base URL, or why it could not be parsed (reported on first use, like httpx).
    parsed: std::result::Result<Url, String>,
    client: std::result::Result<reqwest::Client, String>,
}

/// A request's write metadata.
#[derive(Debug, Clone, Copy, Default)]
pub struct Command<'a> {
    pub actor: Option<&'a str>,
    pub command_id: Option<&'a str>,
}

impl<'a> Command<'a> {
    /// A project command by `actor`, idempotent on `command_id`.
    pub fn by(actor: &'a str, command_id: &'a str) -> Self {
        Self { actor: Some(actor), command_id: Some(command_id) }
    }

    /// An administrative command without an actor hint.
    pub fn id(command_id: &'a str) -> Self {
        Self { actor: None, command_id: Some(command_id) }
    }
}

impl HttpRuntimeClient {
    /// A client for `base_url`, authenticated with `token` when one is given. `agent_lane` names
    /// the managed agent lane on internal requests.
    pub fn new(base_url: &str, token: Option<&str>, agent_lane: Option<&str>) -> Self {
        let base = base_url.trim_end_matches('/').to_owned();
        let parsed = Url::parse(&base).map_err(|error| error.to_string()).and_then(|url| {
            if url.cannot_be_a_base() {
                Err("the URL cannot be a base".to_owned())
            } else {
                Ok(url)
            }
        });
        let mut headers = HeaderMap::new();
        let mut header_error = None;
        if let Some(token) = token.filter(|token| !token.is_empty()) {
            match HeaderValue::from_str(&format!("Bearer {token}")) {
                Ok(mut value) => {
                    value.set_sensitive(true);
                    headers.insert(AUTHORIZATION, value);
                }
                Err(_) => header_error = Some("the access token is not a valid header value"),
            }
        }
        if let Some(lane) = agent_lane.filter(|lane| !lane.is_empty()) {
            match HeaderValue::from_bytes(lane.as_bytes()) {
                Ok(value) => {
                    headers.insert(HeaderName::from_static(LANE_HEADER), value);
                }
                Err(_) => header_error = Some("the agent lane is not a valid header value"),
            }
        }
        let client = match header_error {
            Some(error) => Err(error.to_owned()),
            None => reqwest::Client::builder()
                .default_headers(headers)
                .user_agent(concat!("limina/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(TIMEOUT)
                .read_timeout(TIMEOUT)
                .build()
                .map_err(|error| error.to_string()),
        };
        Self { base, parsed, client }
    }

    /// The instance URL as error messages show it (httpx kept a trailing slash on base paths).
    pub fn display_url(&self) -> String {
        match &self.parsed {
            Ok(url) if url.path() != "/" && !url.path().is_empty() => format!("{}/", self.base),
            _ => self.base.clone(),
        }
    }

    fn unreachable(&self, reason: impl Into<Value>) -> LiminaError {
        LiminaError::transport(format!("Cannot reach Limina runtime at {}.", self.display_url()))
            .with("reason", reason)
    }

    /// The URL of `segments` under the base path, each segment percent-encoded.
    fn url(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Url> {
        let mut url = self.parsed.clone().map_err(|reason| self.unreachable(reason))?;
        {
            let mut path =
                url.path_segments_mut().map_err(|()| self.unreachable("invalid base URL"))?;
            path.pop_if_empty();
            path.extend(segments);
        }
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        Ok(url)
    }

    /// One request: JSON in, JSON out, errors mapped back to [`LiminaError`].
    pub async fn request(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
        body: Option<&Value>,
        command: Command<'_>,
    ) -> Result<Value> {
        let url = self.url(segments, query)?;
        let client = self.client.as_ref().map_err(|reason| self.unreachable(reason.as_str()))?;
        let mut request = client.request(method, url);
        if let Some(actor) = command.actor {
            let value = HeaderValue::from_bytes(actor.as_bytes()).map_err(|_| {
                LiminaError::invariant("The actor is not a valid HTTP header value.")
                    .with("actor", actor)
            })?;
            request = request.header(ACTOR_HEADER, value);
        }
        if let Some(command_id) = command.command_id {
            request = request.header(IDEMPOTENCY_HEADER, command_id);
        }
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(body).map_err(limina_core::internal)?);
        }
        let response =
            request.send().await.map_err(|error| self.unreachable(error_chain(&error)))?;
        let status = response.status();
        let bytes =
            response.bytes().await.map_err(|error| self.unreachable(error_chain(&error)))?;
        if status.is_success() {
            return serde_json::from_slice(&bytes).map_err(|error| {
                LiminaError::transport(format!(
                    "Runtime returned HTTP {} with an invalid JSON body.",
                    status.as_u16()
                ))
                .with("reason", error.to_string())
            });
        }
        Err(error_from_response(status.as_u16(), &bytes))
    }

    async fn get(&self, segments: &[&str]) -> Result<Value> {
        self.request(Method::GET, segments, &[], None, Command::default()).await
    }

    async fn send<T: Serialize>(
        &self,
        method: Method,
        segments: &[&str],
        body: &T,
        command: Command<'_>,
    ) -> Result<Value> {
        let body = serde_json::to_value(body).map_err(limina_core::internal)?;
        self.request(method, segments, &[], Some(&body), command).await
    }

    // Instance.

    pub async fn health(&self) -> Result<Value> {
        self.get(&["healthz"]).await
    }

    pub async fn codex_auth_status(&self) -> Result<Value> {
        self.get(&["v1", "runtime", "engines", "codex", "auth"]).await
    }

    pub async fn codex_login(&self, method: &str, command_id: &str) -> Result<Value> {
        self.send(
            Method::POST,
            &["v1", "runtime", "engines", "codex", "auth", "login"],
            &serde_json::json!({"method": method}),
            Command::id(command_id),
        )
        .await
    }

    pub async fn codex_login_attempt(&self, login_id: &str) -> Result<Value> {
        self.get(&["v1", "runtime", "engines", "codex", "auth", "login", login_id]).await
    }

    pub async fn cancel_codex_login(&self, login_id: &str) -> Result<Value> {
        self.request(
            Method::DELETE,
            &["v1", "runtime", "engines", "codex", "auth", "login", login_id],
            &[],
            None,
            Command::default(),
        )
        .await
    }

    pub async fn codex_logout(&self) -> Result<Value> {
        self.request(
            Method::DELETE,
            &["v1", "runtime", "engines", "codex", "auth"],
            &[],
            None,
            Command::default(),
        )
        .await
    }

    // Projects.

    pub async fn create_project(&self, payload: &Value, command: Command<'_>) -> Result<Value> {
        self.send(Method::POST, &["v1", "projects"], payload, command).await
    }

    /// Every project, following `next_cursor` pages of 200.
    pub async fn projects(&self, include_archived: bool) -> Result<Value> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut query = vec![
                ("include_archived", include_archived.to_string()),
                ("limit", "200".to_owned()),
            ];
            if let Some(cursor) = &cursor {
                query.push(("cursor", cursor.clone()));
            }
            let page = self
                .request(Method::GET, &["v1", "projects"], &query, None, Command::default())
                .await?;
            if let Some(Value::Array(page_items)) = page.get("items") {
                items.extend(page_items.iter().cloned());
            }
            cursor = match page.get("next_cursor") {
                Some(Value::String(next)) if !next.is_empty() => Some(next.clone()),
                _ => None,
            };
            if cursor.is_none() {
                return Ok(Value::Array(items));
            }
        }
    }

    pub async fn project(&self, slug: &str) -> Result<Value> {
        self.get(&["v1", "projects", slug]).await
    }

    pub async fn project_status(&self, slug: &str) -> Result<Value> {
        self.get(&["v1", "projects", slug, "status"]).await
    }

    pub async fn project_action(
        &self,
        slug: &str,
        action: &str,
        command: Command<'_>,
    ) -> Result<Value> {
        let body = Value::Object(Map::new());
        self.send(Method::POST, &["v1", "projects", slug, "actions", action], &body, command).await
    }

    pub async fn steer_project(
        &self,
        slug: &str,
        payload: &Value,
        command: Command<'_>,
    ) -> Result<Value> {
        self.send(Method::POST, &["v1", "projects", slug, "steering"], payload, command).await
    }

    pub async fn review(&self, slug: &str) -> Result<Value> {
        self.get(&["v1", "projects", slug, "review"]).await
    }

    pub async fn knowledge(&self, slug: &str, artifact_id: &str) -> Result<Value> {
        self.get(&["v1", "projects", slug, "knowledge", artifact_id]).await
    }

    pub async fn activity(&self, slug: &str, after: i64, limit: i64) -> Result<Value> {
        let query = [("after", after.to_string()), ("limit", limit.to_string())];
        self.request(
            Method::GET,
            &["v1", "projects", slug, "events"],
            &query,
            None,
            Command::default(),
        )
        .await
    }

    pub async fn set_variable(
        &self,
        slug: &str,
        name: &str,
        value: &str,
        command: Command<'_>,
    ) -> Result<Value> {
        let body = serde_json::json!({"value": value});
        let path = ["v1", "projects", slug, "resources", "variables", name];
        self.send(Method::PUT, &path, &body, command).await
    }

    pub async fn set_secret(
        &self,
        slug: &str,
        name: &str,
        value: &str,
        command: Command<'_>,
    ) -> Result<Value> {
        let body = serde_json::json!({"value": value});
        let path = ["v1", "projects", slug, "resources", "secrets", name];
        self.send(Method::PUT, &path, &body, command).await
    }

    pub async fn resources(&self, slug: &str) -> Result<Value> {
        self.get(&["v1", "projects", slug, "resources"]).await
    }

    pub async fn remove_resource(
        &self,
        slug: &str,
        name: &str,
        command: Command<'_>,
    ) -> Result<Value> {
        self.request(
            Method::DELETE,
            &["v1", "projects", slug, "resources", name],
            &[],
            None,
            command,
        )
        .await
    }

    /// The Markdown export as `relative path → content`.
    pub async fn snapshot(&self, slug: &str) -> Result<Map<String, Value>> {
        let result = self.get(&["v1", "projects", slug, "snapshot"]).await?;
        match result.get("files") {
            Some(Value::Object(files)) => Ok(files.clone()),
            _ => Err(LiminaError::transport("Runtime returned a snapshot without files.")),
        }
    }

    // The internal agent protocol (`/internal/v1`), authenticated by a project capability.

    pub async fn status(&self, slug: &str) -> Result<Value> {
        self.get(&["internal", "v1", "projects", slug, "status"]).await
    }

    pub async fn artifacts(&self, slug: &str, kind: Option<&str>) -> Result<Value> {
        let query: Vec<(&str, String)> = kind
            .filter(|kind| !kind.is_empty())
            .map(|kind| vec![("kind", kind.to_owned())])
            .unwrap_or_default();
        self.request(
            Method::GET,
            &["internal", "v1", "projects", slug, "artifacts"],
            &query,
            None,
            Command::default(),
        )
        .await
    }

    pub async fn artifact(&self, slug: &str, artifact_id: &str) -> Result<Value> {
        self.get(&["internal", "v1", "projects", slug, "artifacts", artifact_id]).await
    }

    /// `POST /internal/v1/projects/{slug}/{path…}` with a command body.
    pub async fn internal_command<T: Serialize>(
        &self,
        slug: &str,
        path: &[&str],
        body: &T,
        command: Command<'_>,
    ) -> Result<Value> {
        let mut segments = vec!["internal", "v1", "projects", slug];
        segments.extend_from_slice(path);
        self.send(Method::POST, &segments, body, command).await
    }
}

/// The error codes whose kind the CLI restores, so exit codes match the server-side error.
fn kind_for_code(code: &str) -> Option<ErrorKind> {
    Some(match code {
        "not_found" => ErrorKind::NotFound,
        "version_conflict" => ErrorKind::Conflict,
        "invariant_violation" => ErrorKind::Invariant,
        "lease_conflict" => ErrorKind::LeaseConflict,
        "authentication_required" => ErrorKind::Authentication,
        "permission_denied" => ErrorKind::Authorization,
        "rate_limited" => ErrorKind::RateLimited,
        _ => return None,
    })
}

/// Maps a non-2xx response body `{"error": {code, message, details}}` back to a [`LiminaError`].
/// Codes without a CLI kind (validation errors, database errors, …) become transport errors that
/// record the HTTP status, as the Python client did.
pub fn error_from_response(status: u16, body: &[u8]) -> LiminaError {
    let Ok(parsed) = serde_json::from_slice::<Value>(body) else {
        return LiminaError::transport(format!(
            "Runtime returned HTTP {status} without a valid error body."
        ));
    };
    let error = parsed.get("error").and_then(Value::as_object).cloned().unwrap_or_default();
    let code = error.get("code").and_then(Value::as_str).unwrap_or("transport_error");
    let message = match error.get("message") {
        Some(Value::String(message)) => message.clone(),
        Some(other) => crate::output::display(other),
        None => format!("Runtime returned HTTP {status}."),
    };
    let details = match error.get("details") {
        Some(Value::Object(details)) => details.clone(),
        _ => Map::new(),
    };
    match kind_for_code(code) {
        Some(kind) => {
            let mut error = LiminaError::new(kind, message);
            error.details = details;
            error
        }
        None => {
            let mut error = LiminaError::transport(message).with("status", status);
            for (key, value) in details {
                error.details.entry(key).or_insert(value);
            }
            error
        }
    }
}

/// A transport error with its causes (`error sending request: connection refused`).
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(code: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "error": {"code": code, "message": "Nope.", "details": {"artifact_id": "H001"}}
        }))
        .unwrap()
    }

    #[test]
    fn known_error_codes_keep_their_kind_and_exit_code() {
        let cases = [
            (404, "not_found", 3),
            (409, "version_conflict", 4),
            (422, "invariant_violation", 5),
            (409, "lease_conflict", 6),
            (401, "authentication_required", 7),
            (403, "permission_denied", 7),
            (429, "rate_limited", 9),
        ];
        for (status, code, exit_code) in cases {
            let error = error_from_response(status, &body(code));
            assert_eq!(error.code(), code);
            assert_eq!(error.exit_code(), exit_code, "{code}");
            assert_eq!(error.message, "Nope.");
            assert_eq!(error.details["artifact_id"], "H001");
        }
    }

    #[test]
    fn unknown_codes_become_transport_errors_with_the_status() {
        let error = error_from_response(422, &body("invalid_request"));
        assert_eq!(error.code(), "transport_error");
        assert_eq!(error.exit_code(), 8);
        assert_eq!(
            serde_json::to_string(&error.details).unwrap(),
            r#"{"status":422,"artifact_id":"H001"}"#
        );

        let error = error_from_response(500, b"<html>oops</html>");
        assert_eq!(error.message, "Runtime returned HTTP 500 without a valid error body.");
        assert_eq!(error.exit_code(), 8);

        let error = error_from_response(502, b"{}");
        assert_eq!(error.message, "Runtime returned HTTP 502.");
        assert_eq!(error.details["status"], 502);
    }

    #[test]
    fn urls_encode_segments_under_the_base_path() {
        let client = HttpRuntimeClient::new("http://example.test/limina/", None, None);
        let url = client.url(&["v1", "projects", "a b/c"], &[("kind", "H".into())]).unwrap();
        assert_eq!(url.as_str(), "http://example.test/limina/v1/projects/a%20b%2Fc?kind=H");
        assert_eq!(client.display_url(), "http://example.test/limina/");
        let client = HttpRuntimeClient::new("http://127.0.0.1:7433/", None, None);
        assert_eq!(client.display_url(), "http://127.0.0.1:7433");
        let client = HttpRuntimeClient::new("localhost:7433", None, None);
        assert!(client.url(&["healthz"], &[]).is_err());
    }
}
