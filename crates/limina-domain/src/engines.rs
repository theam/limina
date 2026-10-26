//! Stable project-level runtime engine identifiers.

use std::fmt;

use limina_core::LiminaError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Engine {
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "claude-code")]
    ClaudeCode,
}

pub const SUPPORTED_ENGINES: [Engine; 2] = [Engine::Codex, Engine::ClaudeCode];

impl Engine {
    /// The persisted identifier (`codex`, `claude-code`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
        }
    }

    /// The user-facing product name. Plain names only, never marks; the Agent SDK branding
    /// rules ask integrators to say "Claude" rather than "Claude Code".
    pub const fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude",
        }
    }

    /// Validates an extensible provider-advertised reasoning name without a local allowlist.
    pub fn reasoning_choice(self, value: Option<&str>) -> Result<Option<String>, LiminaError> {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        if value.len() > 64 || !value.bytes().all(|c| c.is_ascii_lowercase() || c == b'-') {
            return Err(LiminaError::invariant("Invalid reasoning effort."));
        }
        Ok(Some(value.to_owned()))
    }

    /// Normalizes an engine name without silently accepting unknown providers.
    pub fn parse(value: &str) -> Result<Self, LiminaError> {
        match value.trim().to_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude-code" => Ok(Self::ClaudeCode),
            _ => Err(LiminaError::invariant("Runtime engine must be one of: claude-code, codex.")
                .with("runtime", value)),
        }
    }
}

/// Empty choices inherit the operator's default. Model IDs are arguments, never shell code.
pub fn model_choice(value: Option<&str>) -> Result<Option<String>, LiminaError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.len() > 200
        || value.starts_with('-')
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(LiminaError::invariant(
            "Model must be a model ID or alias of at most 200 characters.",
        ));
    }
    Ok(Some(value.to_owned()))
}

impl fmt::Display for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_parse_and_label_without_marks() {
        assert_eq!(Engine::parse(" Claude-Code ").unwrap(), Engine::ClaudeCode);
        assert_eq!(Engine::ClaudeCode.label(), "Claude");
        assert_eq!(serde_json::to_string(&Engine::ClaudeCode).unwrap(), "\"claude-code\"");
        assert!(Engine::parse("gemini").is_err());
    }
}
