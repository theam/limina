//! Engine configuration from the runtime's environment.
//!
//! - `LIMINA_CODEX_BIN` (`codex`): the Codex executable, a name on `PATH` or a path.
//! - `LIMINA_CODEX_MODEL` (`gpt-5.4`).
//! - `LIMINA_CODEX_SANDBOX` (`workspace-write`): `read-only`, `workspace-write` or `full-access`.
//! - `LIMINA_CODEX_AUTH_MODE` (`auto`): `auto`, `chatgpt`, `api-key` or `access-token`.
//! - `OPENAI_API_KEY`, `CODEX_ACCESS_TOKEN`: server credentials for Codex, materialized into its
//!   home and never inherited by the process.
//! - `CODEX_HOME` (`<state root>/codex`): the runtime's shared Codex home.
//! - `LIMINA_CODEX_INPUT_USD_PER_MILLION_TOKENS`, `LIMINA_CODEX_CACHED_INPUT_…` and
//!   `LIMINA_CODEX_OUTPUT_…`: operator prices for Codex cost estimates (all or none).
//! - `LIMINA_CLAUDE_BIN` (`claude`): the Claude executable.
//! - `LIMINA_CLAUDE_MODEL` (the CLI's default).
//! - `LIMINA_CLAUDE_CONFIG_DIR` (`<state root>/claude`): root of the per-project configuration.
//! - `LIMINA_CLAUDE_WEAKER_NESTED_SANDBOX` (off): `1`, `true` or `yes` inside containers without
//!   nested namespaces.
//!
//! Claude credentials (`ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, …) pass through to the CLI
//! by the environment policy in `limina_domain::environment`. `CLAUDE_CODE_STREAM_CLOSE_TIMEOUT`
//! lengthens the CLI handshake timeout, as in the SDK.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use limina_domain::usage::TokenPricing;

/// Every engine setting the runtime reads, resolved once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSettings {
    pub codex_bin: String,
    pub codex_model: String,
    pub codex_sandbox: String,
    pub codex_auth_mode: String,
    pub openai_api_key: Option<String>,
    pub codex_access_token: Option<String>,
    pub codex_home: Option<PathBuf>,
    pub codex_pricing: Option<TokenPricing>,
    pub claude_bin: String,
    pub claude_model: Option<String>,
    pub claude_config_dir: Option<PathBuf>,
    pub claude_weaker_nested_sandbox: bool,
}

impl EngineSettings {
    /// Reads the runtime's own environment.
    pub fn from_environment() -> Result<Self, String> {
        Self::from_map(&std::env::vars().collect())
    }

    /// Reads settings from an explicit environment map.
    pub fn from_map(env: &BTreeMap<String, String>) -> Result<Self, String> {
        let value = |name: &str| env.get(name).map(String::as_str).unwrap_or("");
        let present = |name: &str| Some(value(name)).filter(|v| !v.is_empty()).map(str::to_owned);
        let or_default = |name: &str, default: &str| {
            env.get(name).map_or_else(|| default.to_owned(), Clone::clone)
        };
        let pricing_names = [
            "LIMINA_CODEX_INPUT_USD_PER_MILLION_TOKENS",
            "LIMINA_CODEX_CACHED_INPUT_USD_PER_MILLION_TOKENS",
            "LIMINA_CODEX_OUTPUT_USD_PER_MILLION_TOKENS",
        ];
        let pricing_values: Vec<String> =
            pricing_names.iter().map(|name| value(name).trim().to_owned()).collect();
        Ok(Self {
            codex_bin: present("LIMINA_CODEX_BIN").unwrap_or_else(|| "codex".to_owned()),
            codex_model: or_default("LIMINA_CODEX_MODEL", "gpt-5.4"),
            codex_sandbox: or_default("LIMINA_CODEX_SANDBOX", "workspace-write"),
            codex_auth_mode: or_default("LIMINA_CODEX_AUTH_MODE", "auto"),
            openai_api_key: present("OPENAI_API_KEY"),
            codex_access_token: present("CODEX_ACCESS_TOKEN"),
            codex_home: env.get("CODEX_HOME").map(PathBuf::from),
            codex_pricing: TokenPricing::from_values(
                &pricing_names.map(str::to_owned),
                &pricing_values,
            )?,
            claude_bin: present("LIMINA_CLAUDE_BIN").unwrap_or_else(|| "claude".to_owned()),
            claude_model: present("LIMINA_CLAUDE_MODEL"),
            claude_config_dir: env.get("LIMINA_CLAUDE_CONFIG_DIR").map(PathBuf::from),
            claude_weaker_nested_sandbox: matches!(
                value("LIMINA_CLAUDE_WEAKER_NESTED_SANDBOX").to_lowercase().as_str(),
                "1" | "true" | "yes"
            ),
        })
    }

    /// The runtime's shared Codex home: `CODEX_HOME`, else `<state root>/codex`.
    pub fn codex_home_or(&self, state_root: &Path) -> PathBuf {
        self.codex_home.clone().unwrap_or_else(|| state_root.join("codex"))
    }

    /// The root of per-project Claude configuration: `LIMINA_CLAUDE_CONFIG_DIR`, else
    /// `<state root>/claude`.
    pub fn claude_config_root_or(&self, state_root: &Path) -> PathBuf {
        self.claude_config_dir.clone().unwrap_or_else(|| state_root.join("claude"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn defaults_match_the_python_runtime() {
        let settings = EngineSettings::from_map(&BTreeMap::new()).unwrap();
        assert_eq!(settings.codex_bin, "codex");
        assert_eq!(settings.codex_model, "gpt-5.4");
        assert_eq!(settings.codex_sandbox, "workspace-write");
        assert_eq!(settings.codex_auth_mode, "auto");
        assert_eq!(settings.claude_bin, "claude");
        assert_eq!(settings.claude_model, None);
        assert!(!settings.claude_weaker_nested_sandbox);
        assert_eq!(settings.codex_pricing, None);
        assert_eq!(settings.codex_home_or(Path::new("/state")), PathBuf::from("/state/codex"));
        assert_eq!(
            settings.claude_config_root_or(Path::new("/state")),
            PathBuf::from("/state/claude")
        );
    }

    #[test]
    fn overrides_and_flags_are_read() {
        let settings = EngineSettings::from_map(&map(&[
            ("LIMINA_CODEX_BIN", "/opt/codex"),
            ("LIMINA_CLAUDE_MODEL", ""),
            ("LIMINA_CLAUDE_WEAKER_NESTED_SANDBOX", "TRUE"),
            ("LIMINA_CLAUDE_CONFIG_DIR", "/claude"),
            ("OPENAI_API_KEY", ""),
            ("CODEX_ACCESS_TOKEN", "token"),
            ("LIMINA_CODEX_INPUT_USD_PER_MILLION_TOKENS", "1"),
            ("LIMINA_CODEX_CACHED_INPUT_USD_PER_MILLION_TOKENS", "0.1"),
            ("LIMINA_CODEX_OUTPUT_USD_PER_MILLION_TOKENS", "4"),
        ]))
        .unwrap();
        assert_eq!(settings.codex_bin, "/opt/codex");
        assert_eq!(settings.claude_model, None);
        assert!(settings.claude_weaker_nested_sandbox);
        assert_eq!(settings.openai_api_key, None);
        assert_eq!(settings.codex_access_token.as_deref(), Some("token"));
        assert!(settings.codex_pricing.is_some());
        assert_eq!(settings.claude_config_root_or(Path::new("/s")), PathBuf::from("/claude"));
        let partial =
            EngineSettings::from_map(&map(&[("LIMINA_CODEX_INPUT_USD_PER_MILLION_TOKENS", "1")]));
        assert!(partial.unwrap_err().contains("missing"));
    }
}
