//! Remote MCP servers a turn may use: the connections a project's owners enabled for it.
//!
//! The supervisor resolves them before each turn (with fresh credentials) and passes them in
//! [`crate::TurnRequest::mcp_servers`]. Each engine hands them to its CLI its own way:
//!
//! - **Claude**: a private `--mcp-config` file ([`CLAUDE_MCP_CONFIG_FILE`], mode `0600`) in the
//!   project's Claude configuration directory, which the sandbox never lets commands read, with
//!   `--strict-mcp-config` so no other MCP configuration applies. Limina's `can_use_tool` policy
//!   allows those servers' tools and refuses any other MCP tool.
//! - **Codex**: one `--config mcp_servers.<name>={…}` override per server ([`codex_mcp_launch`])
//!   that names environment variables for the credentials (`bearer_token_env_var`,
//!   `env_http_headers`). The values are set only in the app-server's own environment, excluded
//!   from the commands Codex runs (`shell_environment_policy.exclude`), and the servers' tools run
//!   without approval prompts (`default_tools_approval_mode = "approve"`).
//!
//! Credential values never appear on a command line, and [`McpServer`]'s `Debug` hides them.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value, json};

/// The private MCP configuration file in a project's Claude configuration directory.
pub const CLAUDE_MCP_CONFIG_FILE: &str = "limina-mcp.json";
/// The longest server name: it becomes part of every tool name (`mcp__<name>__<tool>`).
pub const SERVER_NAME_LIMIT: usize = 48;
/// How long Codex waits for a server to start answering.
const CODEX_STARTUP_TIMEOUT_SECONDS: u32 = 30;
/// How long one tool call may take.
const CODEX_TOOL_TIMEOUT_SECONDS: u32 = 300;
/// The environment variables that carry credentials to Codex.
pub const CODEX_ENV_PREFIX: &str = "LIMINA_MCP_";

/// One Streamable HTTP MCP server for a turn.
#[derive(Clone, PartialEq, Eq)]
pub struct McpServer {
    /// The name the engine shows the server's tools under: a lower-case letter, then lower-case
    /// letters, digits and single `_` (see [`is_server_name`]).
    pub name: String,
    pub url: String,
    /// Sent as `Authorization: Bearer <token>`.
    pub bearer_token: Option<String>,
    /// Other headers the server needs (an API key header), name → secret value.
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for McpServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServer")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("bearer_token", &self.bearer_token.as_ref().map(|_| "[REDACTED]"))
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl McpServer {
    /// A server without credentials.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self { name: name.into(), url: url.into(), bearer_token: None, headers: BTreeMap::new() }
    }

    /// Every credential value, for redaction.
    pub fn secrets(&self) -> impl Iterator<Item = &String> {
        self.bearer_token.iter().chain(self.headers.values())
    }
}

/// Whether `name` can name a server: `[a-z][a-z0-9_]*` without `__` or a trailing `_`, at most
/// [`SERVER_NAME_LIMIT`] characters. Engines split tool names on `__`.
pub fn is_server_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && name.len() <= SERVER_NAME_LIMIT
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.contains("__")
        && !name.ends_with('_')
}

/// The server a Claude MCP tool belongs to (`mcp__<server>__<tool>`).
pub fn claude_tool_server(tool_name: &str) -> Option<&str> {
    let rest = tool_name.strip_prefix("mcp__")?;
    Some(rest.split_once("__").map_or(rest, |(server, _)| server))
}

/// The `--mcp-config` document for Claude.
pub fn claude_mcp_config(servers: &[McpServer]) -> Value {
    let entries: Map<String, Value> = servers
        .iter()
        .map(|server| {
            let mut headers: Map<String, Value> = server
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), Value::from(value.as_str())))
                .collect();
            if let Some(token) = &server.bearer_token {
                headers.insert("Authorization".to_owned(), Value::from(format!("Bearer {token}")));
            }
            let mut entry = json!({"type": "http", "url": server.url});
            if !headers.is_empty() {
                entry["headers"] = Value::Object(headers);
            }
            (server.name.clone(), entry)
        })
        .collect();
    json!({"mcpServers": entries})
}

/// What a Codex app-server needs for the turn's servers.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CodexMcpLaunch {
    /// `--config` overrides, in order.
    pub config_overrides: Vec<String>,
    /// Credential variables for the app-server's environment.
    pub env: BTreeMap<String, String>,
}

impl fmt::Debug for CodexMcpLaunch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodexMcpLaunch")
            .field("config_overrides", &self.config_overrides)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// The Codex configuration for the turn's servers: one inline table per server, credentials by
/// environment variable name only.
pub fn codex_mcp_launch(servers: &[McpServer]) -> CodexMcpLaunch {
    let mut launch = CodexMcpLaunch::default();
    for server in servers {
        let stem = format!("{CODEX_ENV_PREFIX}{}", server.name.to_ascii_uppercase());
        let mut fields = vec![format!("url = {}", toml_string(&server.url))];
        if let Some(token) = &server.bearer_token {
            let variable = format!("{stem}_TOKEN");
            fields.push(format!("bearer_token_env_var = {}", toml_string(&variable)));
            launch.env.insert(variable, token.clone());
        }
        if !server.headers.is_empty() {
            let headers: Vec<String> = server
                .headers
                .iter()
                .enumerate()
                .map(|(index, (name, value))| {
                    let variable = format!("{stem}_HEADER_{}", index + 1);
                    launch.env.insert(variable.clone(), value.clone());
                    format!("{} = {}", toml_string(name), toml_string(&variable))
                })
                .collect();
            fields.push(format!("env_http_headers = {{ {} }}", headers.join(", ")));
        }
        fields.push("default_tools_approval_mode = \"approve\"".to_owned());
        fields.push(format!("startup_timeout_sec = {CODEX_STARTUP_TIMEOUT_SECONDS}"));
        fields.push(format!("tool_timeout_sec = {CODEX_TOOL_TIMEOUT_SECONDS}"));
        launch.config_overrides.push(format!(
            "mcp_servers.{}={{ {} }}",
            server.name,
            fields.join(", ")
        ));
    }
    if !launch.env.is_empty() {
        launch
            .config_overrides
            .push(format!("shell_environment_policy.exclude=[\"{CODEX_ENV_PREFIX}*\"]"));
    }
    launch
}

/// A TOML basic string.
fn toml_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c if c.is_control() => quoted.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes() -> McpServer {
        McpServer {
            bearer_token: Some("tok-123".into()),
            ..McpServer::new("notes", "https://mcp.example.test/mcp")
        }
    }

    fn keyed() -> McpServer {
        let mut server = McpServer::new("crm", "https://crm.example.test/mcp?team=\"a\"");
        server.headers.insert("X-Api-Key".into(), "key-456".into());
        server
    }

    #[test]
    fn server_names_are_safe_tool_prefixes() {
        for good in ["notes", "team_notes", "a1", "crm_2"] {
            assert!(is_server_name(good), "{good}");
        }
        for bad in ["", "Notes", "1notes", "team__notes", "notes_", "team-notes", "nötes"] {
            assert!(!is_server_name(bad), "{bad}");
        }
        assert!(!is_server_name(&"a".repeat(SERVER_NAME_LIMIT + 1)));
        assert_eq!(claude_tool_server("mcp__notes__search"), Some("notes"));
        assert_eq!(claude_tool_server("mcp__notes"), Some("notes"));
        assert_eq!(claude_tool_server("Bash"), None);
    }

    #[test]
    fn claude_gets_http_servers_with_their_headers() {
        let config = claude_mcp_config(&[
            notes(),
            keyed(),
            McpServer::new("open", "http://127.0.0.1:9/mcp"),
        ]);
        assert_eq!(
            config,
            json!({"mcpServers": {
                "notes": {"type": "http", "url": "https://mcp.example.test/mcp",
                          "headers": {"Authorization": "Bearer tok-123"}},
                "crm": {"type": "http", "url": "https://crm.example.test/mcp?team=\"a\"",
                        "headers": {"X-Api-Key": "key-456"}},
                "open": {"type": "http", "url": "http://127.0.0.1:9/mcp"},
            }})
        );
    }

    #[test]
    fn codex_gets_variables_by_name_and_never_the_values() {
        let launch = codex_mcp_launch(&[notes(), keyed()]);
        assert_eq!(
            launch.config_overrides,
            vec![
                "mcp_servers.notes={ url = \"https://mcp.example.test/mcp\", \
                 bearer_token_env_var = \"LIMINA_MCP_NOTES_TOKEN\", \
                 default_tools_approval_mode = \"approve\", startup_timeout_sec = 30, \
                 tool_timeout_sec = 300 }"
                    .to_owned(),
                "mcp_servers.crm={ url = \"https://crm.example.test/mcp?team=\\\"a\\\"\", \
                 env_http_headers = { \"X-Api-Key\" = \"LIMINA_MCP_CRM_HEADER_1\" }, \
                 default_tools_approval_mode = \"approve\", startup_timeout_sec = 30, \
                 tool_timeout_sec = 300 }"
                    .to_owned(),
                "shell_environment_policy.exclude=[\"LIMINA_MCP_*\"]".to_owned(),
            ]
        );
        assert_eq!(
            launch.env,
            BTreeMap::from([
                ("LIMINA_MCP_CRM_HEADER_1".to_owned(), "key-456".to_owned()),
                ("LIMINA_MCP_NOTES_TOKEN".to_owned(), "tok-123".to_owned()),
            ])
        );
        assert!(
            launch
                .config_overrides
                .iter()
                .all(|o| !o.contains("tok-123") && !o.contains("key-456"))
        );
        assert!(!format!("{launch:?}").contains("tok-123"));
        assert!(!format!("{:?}", notes()).contains("tok-123"));
        let open = codex_mcp_launch(&[McpServer::new("open", "http://127.0.0.1:9/mcp")]);
        assert_eq!(open.config_overrides.len(), 1, "no credentials, nothing to exclude");
        assert!(codex_mcp_launch(&[]).config_overrides.is_empty());
    }

    #[test]
    fn toml_strings_escape_quotes_and_controls() {
        assert_eq!(toml_string("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }
}
