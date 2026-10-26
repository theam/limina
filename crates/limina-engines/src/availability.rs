//! Engine readiness shown before kickoff: provider installed and server-side credentials.
//!
//! Only credential *sources* are reported (an account e-mail or an environment variable name);
//! secret values never leave the server.

use std::path::{Path, PathBuf};

use limina_domain::Engine;
use limina_domain::environment::CLAUDE_CREDENTIAL_ENV;
use serde::{Deserialize, Serialize};

use crate::codex::auth::CodexAuthStatus;

/// One engine's readiness. Field order matches the Python dict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineAvailability {
    pub engine: String,
    pub label: String,
    pub installed: bool,
    pub credentials: bool,
    pub detail: String,
    pub model: Option<String>,
}

fn availability(
    engine: Engine,
    installed: bool,
    credentials: bool,
    detail: String,
    model: Option<&str>,
) -> EngineAvailability {
    EngineAvailability {
        engine: engine.id().to_owned(),
        label: engine.label().to_owned(),
        installed,
        credentials,
        detail,
        model: model.map(str::to_owned),
    }
}

/// `auth_status` is `None` while a login or logout owns the credential store.
pub fn codex_availability(
    installed: bool,
    auth_status: Option<&CodexAuthStatus>,
    model: Option<&str>,
) -> EngineAvailability {
    let credentials = installed && auth_status.is_some_and(|status| status.configured);
    let detail = if !installed {
        "The Codex CLI is not installed in this runtime.".to_owned()
    } else if let Some(status) = auth_status.filter(|_| credentials) {
        if status.active_method.as_deref() == Some("chatgpt") {
            match status.account_email.as_deref() {
                Some(email) if !email.is_empty() => format!("Signed in with ChatGPT ({email})"),
                _ => "Signed in with ChatGPT".to_owned(),
            }
        } else {
            "Authenticated with an OpenAI API key.".to_owned()
        }
    } else if auth_status.is_none() {
        "Codex sign-in is being updated; check again in a moment.".to_owned()
    } else {
        "Sign in with ChatGPT, or set OPENAI_API_KEY or CODEX_ACCESS_TOKEN, to enable Codex."
            .to_owned()
    };
    availability(Engine::Codex, installed, credentials, detail, model)
}

/// `credential_source` is a credential variable name or a cloud provider name
/// (`limina_domain::environment::claude_credential_source`).
pub fn claude_availability(
    installed: bool,
    credential_source: Option<&str>,
    model: Option<&str>,
) -> EngineAvailability {
    let credentials = installed && credential_source.is_some_and(|source| !source.is_empty());
    let detail = match credential_source {
        _ if !installed => "The Claude CLI is not installed in this runtime.".to_owned(),
        None => {
            "Connect an account to the project, or set ANTHROPIC_API_KEY on the runtime.".to_owned()
        }
        Some(source) if CLAUDE_CREDENTIAL_ENV.contains(&source) => {
            format!("Using {source} from the server environment.")
        }
        Some(source) => format!("Using {source}."),
    };
    availability(Engine::ClaudeCode, installed, credentials, detail, model)
}

/// True when `binary` names an executable file: a path as given, or a name found on `PATH`.
pub async fn binary_installed(binary: &str) -> bool {
    let binary = binary.to_owned();
    let path = std::env::var_os("PATH");
    tokio::task::spawn_blocking(move || find_binary(&binary, path.as_deref()).is_some())
        .await
        .unwrap_or(false)
}

fn find_binary(binary: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    if binary.is_empty() {
        return None;
    }
    if binary.contains(std::path::MAIN_SEPARATOR) {
        let candidate = PathBuf::from(binary);
        return is_executable(&candidate).then_some(candidate);
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(binary))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else { return false };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(method: Option<&str>, email: Option<&str>) -> CodexAuthStatus {
        CodexAuthStatus {
            engine: "codex".into(),
            configured_mode: "auto".into(),
            configured: method.is_some(),
            active_method: method.map(str::to_owned),
            account_email: email.map(str::to_owned),
            account_plan: None,
            source: "cached_chatgpt".into(),
            error: None,
            single_runtime_node: true,
        }
    }

    #[test]
    fn codex_details_follow_the_auth_state() {
        let ready = codex_availability(
            true,
            Some(&status(Some("chatgpt"), Some("o@example.test"))),
            Some("gpt-5.4"),
        );
        assert_eq!(ready.detail, "Signed in with ChatGPT (o@example.test)");
        assert!(ready.credentials);
        assert_eq!((ready.engine.as_str(), ready.label.as_str()), ("codex", "Codex"));
        assert_eq!(ready.model.as_deref(), Some("gpt-5.4"));
        let api_key = codex_availability(true, Some(&status(Some("api-key"), None)), None);
        assert_eq!(api_key.detail, "Authenticated with an OpenAI API key.");
        let busy = codex_availability(true, None, None);
        assert!(busy.detail.contains("being updated"));
        assert!(!busy.credentials);
        let signed_out = codex_availability(true, Some(&status(None, None)), None);
        assert!(signed_out.detail.starts_with("Sign in with ChatGPT"));
        assert!(!codex_availability(false, Some(&status(Some("chatgpt"), None)), None).credentials);
    }

    #[test]
    fn claude_details_name_the_credential_source_only() {
        let env = claude_availability(true, Some("ANTHROPIC_API_KEY"), None);
        assert_eq!(env.detail, "Using ANTHROPIC_API_KEY from the server environment.");
        assert_eq!((env.engine.as_str(), env.label.as_str()), ("claude-code", "Claude"));
        assert_eq!(
            claude_availability(true, Some("Amazon Bedrock"), None).detail,
            "Using Amazon Bedrock."
        );
        let missing = claude_availability(true, None, None);
        assert!(!missing.credentials);
        assert!(missing.detail.starts_with("Connect an account"));
        assert!(!claude_availability(false, Some("ANTHROPIC_API_KEY"), None).credentials);
    }

    #[test]
    fn binaries_are_found_on_the_path() {
        let path = std::env::var_os("PATH");
        assert!(find_binary("sh", path.as_deref()).is_some());
        assert!(find_binary("limina-no-such-binary", path.as_deref()).is_none());
        assert!(find_binary("/bin/sh", None).is_some());
    }
}
