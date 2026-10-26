//! The scripted agent's client for the internal agent API (`/internal/v1/projects/{slug}/…`).
//!
//! It authenticates with the turn's capability (`LIMINA_INTERNAL_TOKEN`) at `LIMINA_INTERNAL_URL`,
//! both from the turn's environment, as the `limina _agent` commands of a real provider do.

use std::collections::BTreeMap;
use std::time::Duration;

use limina_core::{LiminaError, Result, internal, new_uuid};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::Value;

/// The Python script's `httpx.AsyncClient(timeout=10)`.
const TIMEOUT: Duration = Duration::from_secs(10);
/// A recovered turn can start before the server accepts connections: the first request retries
/// connection failures this many times …
const STARTUP_ATTEMPTS: usize = 40;
/// … this far apart (about ten seconds in all).
const STARTUP_RETRY: Duration = Duration::from_millis(250);

/// A random command ID in the form the script sent (`uuid4().hex`).
pub(super) fn hex_id() -> String {
    new_uuid().replace('-', "")
}

pub(super) struct InternalClient {
    http: reqwest::Client,
    /// The internal API URL without a trailing slash.
    base: String,
    slug: String,
}

impl InternalClient {
    /// A client for `slug` from the turn's environment.
    pub(super) fn new(runtime_env: &BTreeMap<String, String>, slug: &str) -> Result<Self> {
        let setting = |name: &str| {
            runtime_env.get(name).map(String::as_str).filter(|value| !value.is_empty()).ok_or_else(
                || {
                    LiminaError::transport(
                        "The scripted engine needs the internal agent API in its environment.",
                    )
                    .with("missing", name)
                },
            )
        };
        let base = setting("LIMINA_INTERNAL_URL")?.trim_end_matches('/').to_owned();
        let token = setting("LIMINA_INTERNAL_TOKEN")?;
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                LiminaError::transport("The internal capability is not a valid header value.")
            })?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(TIMEOUT)
            .build()
            .map_err(internal)?;
        Ok(Self { http, base, slug: slug.to_owned() })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/internal/v1/projects/{}/{path}", self.base, self.slug)
    }

    /// `GET /internal/v1/projects/{slug}/{path}`.
    pub(super) async fn get(&self, path: &str) -> Result<Value> {
        let response = self.http.get(self.url(path)).send().await;
        self.read("GET", path, response).await
    }

    /// `POST /internal/v1/projects/{slug}/{path}` with a fresh `Idempotency-Key`.
    pub(super) async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let response = self
            .http
            .post(self.url(path))
            .header("Idempotency-Key", hex_id())
            .json(body)
            .send()
            .await;
        self.read("POST", path, response).await
    }

    /// The project's artifacts (`GET …/artifacts`), waiting for the server to accept connections.
    pub(super) async fn artifacts(&self) -> Result<Vec<Value>> {
        let mut attempt = 1;
        let response = loop {
            match self.http.get(self.url("artifacts")).send().await {
                Err(error) if error.is_connect() && attempt < STARTUP_ATTEMPTS => {
                    attempt += 1;
                    tokio::time::sleep(STARTUP_RETRY).await;
                }
                response => break response,
            }
        };
        // The API returns a list; an `{"items": […]}` page is accepted too.
        Ok(match self.read("GET", "artifacts", response).await? {
            Value::Array(items) => items,
            Value::Object(mut page) => match page.remove("items") {
                Some(Value::Array(items)) => items,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        })
    }

    async fn read(
        &self,
        method: &str,
        path: &str,
        response: reqwest::Result<reqwest::Response>,
    ) -> Result<Value> {
        let unreachable = |error: reqwest::Error| {
            LiminaError::transport("The scripted engine could not reach the internal agent API.")
                .with("url", self.base.as_str())
                .with("reason", error.to_string())
        };
        let response = response.map_err(unreachable)?;
        let status = response.status();
        let body = response.bytes().await.map_err(unreachable)?;
        if !status.is_success() {
            let parsed = serde_json::from_slice::<Value>(&body).ok();
            let reason = parsed
                .as_ref()
                .and_then(|body| body.pointer("/error/message"))
                .and_then(Value::as_str)
                .map_or_else(
                    || String::from_utf8_lossy(&body).chars().take(500).collect(),
                    str::to_owned,
                );
            return Err(LiminaError::transport(format!(
                "The internal agent API rejected {method} {path} with HTTP {}.",
                status.as_u16()
            ))
            .with("status", status.as_u16())
            .with("reason", reason));
        }
        serde_json::from_slice(&body).map_err(|_| {
            LiminaError::transport("The internal agent API returned an invalid response.")
                .with("path", path)
        })
    }
}
