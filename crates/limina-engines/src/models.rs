//! Live provider model metadata. Discovery completes the CLI handshake and sends no prompt.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use limina_core::{LiminaError, Result, internal};
use limina_domain::environment::{claude_environment, codex_environment, process_environment};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::claude::connection::{CLIENT_APP, ClaudeConnection, ClaudeLaunch};
use crate::claude::permissions::ToolPolicy;
use crate::codex::rpc::{AppServerClient, CodexLaunch};
use crate::process::blocking;

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    /// The concrete model behind a provider alias, when advertised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    pub reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub default_model: Option<String>,
    pub models: Vec<ModelOption>,
}

fn unavailable() -> LiminaError {
    LiminaError::transport("The provider did not return a model catalog. Refresh to try again.")
}

/// The account's Codex home determines the catalog, just as it does when running a turn.
pub async fn codex_models(binary: &str, home: &Path) -> Result<Vec<ModelOption>> {
    let home = home.to_owned();
    let env = blocking(move || codex_environment(&process_environment(), &BTreeMap::new(), &home))
        .await
        .map_err(internal)?;
    let launch = CodexLaunch {
        binary: binary.to_owned(),
        config_overrides: Vec::new(),
        cwd: None,
        env,
        mcp_servers: Default::default(),
    };
    let discovery = async {
        let client = AppServerClient::start(&launch).await.map_err(|_| unavailable())?;
        let result = async {
            let mut models = Vec::new();
            let mut cursor = None::<String>;
            for _ in 0..10 {
                let response = client
                    .request(
                        "model/list",
                        Some(json!({"limit": 100, "cursor": cursor, "includeHidden": false})),
                    )
                    .await
                    .map_err(|_| unavailable())?;
                models.extend(parse_codex_models(&response)?);
                cursor = response.get("nextCursor").and_then(Value::as_str).map(str::to_owned);
                if cursor.is_none() {
                    return Ok(models);
                }
            }
            Err(unavailable())
        }
        .await;
        client.shutdown().await;
        result
    };
    tokio::time::timeout(DISCOVERY_TIMEOUT, discovery).await.map_err(|_| unavailable())?
}

/// Claude's initialize response is the same live server info exposed by the official Agent
/// SDK. It includes model IDs/aliases and each model's supportedEffortLevels. Never query a
/// model to infer capabilities: a missing list means no advertised effort options.
pub async fn claude_models(
    binary: &str,
    config_dir: &Path,
    overlay: &BTreeMap<String, String>,
) -> Result<Vec<ModelOption>> {
    let cwd = config_dir.to_owned();
    let config_dir = config_dir.to_owned();
    let overlay = overlay.clone();
    let mut env =
        blocking(move || claude_environment(&process_environment(), &overlay, &config_dir))
            .await
            .map_err(internal)?;
    env.insert("CLAUDE_AGENT_SDK_CLIENT_APP".to_owned(), CLIENT_APP.to_owned());
    let args = [
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--setting-sources=",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--strict-mcp-config",
        "--tools",
        "",
        "--disable-slash-commands",
        "--no-session-persistence",
    ]
    .map(str::to_owned)
    .to_vec();
    let launch = ClaudeLaunch { binary: binary.to_owned(), args, env, cwd: cwd.clone() };
    let discovery = async {
        let connection = ClaudeConnection::connect(&launch, Arc::new(ToolPolicy::new(cwd)))
            .await
            .map_err(|_| unavailable())?;
        let result = parse_claude_models(connection.server_info());
        connection.disconnect().await;
        result
    };
    tokio::time::timeout(DISCOVERY_TIMEOUT, discovery).await.map_err(|_| unavailable())?
}

fn parse_codex_models(response: &Value) -> Result<Vec<ModelOption>> {
    let models = response.get("data").and_then(Value::as_array).ok_or_else(unavailable)?;
    Ok(models
        .iter()
        .filter_map(|model| {
            if model.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
                return None;
            }
            let id = model.get("model").and_then(Value::as_str).filter(|id| !id.is_empty())?;
            Some(ModelOption {
                id: id.to_owned(),
                label: model.get("displayName").and_then(Value::as_str).unwrap_or(id).to_owned(),
                resolved_model: None,
                reasoning_efforts: model
                    .get("supportedReasoningEfforts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        option
                            .get("reasoningEffort")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                            .map(str::to_owned)
                    })
                    .collect(),
                default_reasoning_effort: model
                    .get("defaultReasoningEffort")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect())
}

fn parse_claude_models(response: &Value) -> Result<Vec<ModelOption>> {
    let models = response.get("models").and_then(Value::as_array).ok_or_else(unavailable)?;
    Ok(models
        .iter()
        .filter_map(|model| {
            let id = model.get("value").and_then(Value::as_str).filter(|id| !id.is_empty())?;
            Some(ModelOption {
                id: id.to_owned(),
                label: model.get("displayName").and_then(Value::as_str).unwrap_or(id).to_owned(),
                resolved_model: model
                    .get("resolvedModel")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                reasoning_efforts: if model.get("supportsEffort") == Some(&Value::Bool(false)) {
                    Vec::new()
                } else {
                    model
                        .get("supportedEffortLevels")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|value| {
                            value.as_str().filter(|value| !value.is_empty()).map(str::to_owned)
                        })
                        .collect()
                },
                default_reasoning_effort: model
                    .get("defaultReasoningEffort")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_keeps_advertised_efforts_in_order_without_a_local_enum() {
        let models = parse_codex_models(&json!({"data": [
            {"model":"new-model", "defaultReasoningEffort":"adaptive", "supportedReasoningEfforts":[{"reasoningEffort":"adaptive"},{"reasoningEffort":"future"}]},
            {"model":"hidden", "hidden":true}, {"id":"malformed"}
        ]})).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].reasoning_efforts, ["adaptive", "future"]);
        assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("adaptive"));
    }

    #[test]
    fn claude_preserves_per_model_capabilities_and_unknown_future_levels() {
        let models = parse_claude_models(&json!({"models":[
            {"value":"new-alias", "displayName":"New model", "resolvedModel":"new-version", "supportsEffort":true, "supportedEffortLevels":["low","future"]},
            {"value":"fast-model"},
            {"value":"disabled", "supportsEffort":false, "supportedEffortLevels":["high"]}
        ]})).unwrap();
        assert_eq!(models[0].id, "new-alias");
        assert_eq!(models[0].resolved_model.as_deref(), Some("new-version"));
        assert_eq!(models[0].reasoning_efforts, ["low", "future"]);
        assert!(models[1].reasoning_efforts.is_empty());
        assert!(models[2].reasoning_efforts.is_empty());
        assert!(parse_claude_models(&json!({})).is_err());
        assert!(parse_codex_models(&json!({})).is_err());
    }
}
