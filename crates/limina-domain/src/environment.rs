//! Environment policy for provider processes and project-supplied resources.
//!
//! Provider processes start from an empty environment: only the names in a safe list are
//! inherited from the runtime, then the project's resources and the runtime's overlays are added.
//! Control-plane secrets (database URLs, API tokens, WorkOS keys) never reach a provider.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SAFE_PROCESS_ENV: &[&str] = &[
    "HOME",
    "LANG",
    "LC_ALL",
    "LOGNAME",
    "COLORTERM",
    "PATH",
    "SHELL",
    "SSL_CERT_FILE",
    "TERM",
    "TMPDIR",
    "USER",
];

pub const SAFE_CODEX_ENV: &[&str] = &[
    "CODEX_CA_CERTIFICATE",
    "CODEX_CI",
    "CODEX_HOME",
    "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
    "CODEX_PERMISSION_PROFILE",
    "CODEX_SHELL",
];

pub const SAFE_CLAUDE_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_VERTEX",
];

/// Server-side Claude credentials, in the order they are reported. Only names are exposed.
pub const CLAUDE_CREDENTIAL_ENV: &[&str] =
    &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"];

pub const CLAUDE_PROVIDER_ENV: &[(&str, &str)] = &[
    ("CLAUDE_CODE_USE_BEDROCK", "Amazon Bedrock"),
    ("CLAUDE_CODE_USE_VERTEX", "Google Vertex AI"),
    ("CLAUDE_CODE_USE_FOUNDRY", "Microsoft Foundry"),
];

/// Project resources are environment variables inside a provider process. These names can
/// execute code, replace network/auth configuration, or reach outside the managed workspace
/// before the provider's own sandbox starts. The policy applies at write and read time.
pub const RESERVED_RESOURCE_NAMES: &[&str] = &[
    "ALL_PROXY",
    "BASH_ENV",
    "ENV",
    "GIT_ASKPASS",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_SSH_COMMAND",
    "HOME",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "NODE_OPTIONS",
    "NO_PROXY",
    "PATH",
    "PERL5LIB",
    "PERL5OPT",
    "PROMPT_COMMAND",
    "PYTHONHOME",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "RUBYOPT",
    "SHELL",
    "SSL_CERT_DIR",
    "SSL_CERT_FILE",
    "SSH_ASKPASS",
    "ZDOTDIR",
];

pub const RESERVED_RESOURCE_PREFIXES: &[&str] =
    &["ANTHROPIC_", "CLAUDE_", "CODEX_", "DYLD_", "LD_", "LIMINA_", "OPENAI_"];

pub fn is_reserved_resource_name(name: &str) -> bool {
    let normalized = name.trim().to_uppercase();
    RESERVED_RESOURCE_NAMES.contains(&normalized.as_str())
        || RESERVED_RESOURCE_PREFIXES.iter().any(|prefix| normalized.starts_with(prefix))
}

/// Drops unsafe rows even if they predate the current write-time policy. Returns the safe values
/// and the sorted names that were blocked.
pub fn sanitize_project_environment(
    values: BTreeMap<String, String>,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut safe = BTreeMap::new();
    let mut blocked = Vec::new();
    for (name, value) in values {
        if is_reserved_resource_name(&name) {
            blocked.push(name);
        } else {
            safe.insert(name, value);
        }
    }
    blocked.sort();
    (safe, blocked)
}

/// Which server credential Claude will use, without reading its value out.
pub fn claude_credential_source(environ: &BTreeMap<String, String>) -> Option<String> {
    for name in CLAUDE_CREDENTIAL_ENV {
        if environ.get(*name).is_some_and(|value| !value.trim().is_empty()) {
            return Some((*name).to_owned());
        }
    }
    for (name, provider) in CLAUDE_PROVIDER_ENV {
        let value = environ.get(*name).map(|v| v.trim().to_lowercase()).unwrap_or_default();
        if !matches!(value.as_str(), "" | "0" | "false" | "no") {
            return Some((*provider).to_owned());
        }
    }
    None
}

/// The runtime's own environment as a map.
pub fn process_environment() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

pub fn ensure_private_directory(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path.to_path_buf())
}

/// The complete environment for a child process (spawn it with `env_clear()`): safe inherited
/// names, then the overlay. Empty overlay values remove a name, so an overlay can withhold an
/// inherited credential.
pub fn isolated_environment(
    safe_names: &[&[&str]],
    inherited: &BTreeMap<String, String>,
    overlay: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = inherited
        .iter()
        .filter(|(name, _)| safe_names.iter().any(|list| list.contains(&name.as_str())))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    for (name, value) in overlay {
        if value.is_empty() {
            env.remove(name);
        } else {
            env.insert(name.clone(), value.clone());
        }
    }
    env
}

/// Codex's environment: its state pinned to a private home, raw provider credentials removed.
pub fn codex_environment(
    inherited: &BTreeMap<String, String>,
    overlay: &BTreeMap<String, String>,
    codex_home: &Path,
) -> std::io::Result<BTreeMap<String, String>> {
    let mut env = isolated_environment(&[SAFE_PROCESS_ENV, SAFE_CODEX_ENV], inherited, overlay);
    env.insert(
        "CODEX_HOME".to_owned(),
        ensure_private_directory(codex_home)?.to_string_lossy().into_owned(),
    );
    for name in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"] {
        env.remove(name);
    }
    Ok(env)
}

/// Claude's environment: its configuration in a private directory, subprocess credentials scrubbed.
pub fn claude_environment(
    inherited: &BTreeMap<String, String>,
    overlay: &BTreeMap<String, String>,
    config_dir: &Path,
) -> std::io::Result<BTreeMap<String, String>> {
    let mut env = isolated_environment(&[SAFE_PROCESS_ENV, SAFE_CLAUDE_ENV], inherited, overlay);
    env.insert(
        "CLAUDE_CONFIG_DIR".to_owned(),
        ensure_private_directory(config_dir)?.to_string_lossy().into_owned(),
    );
    env.insert("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB".to_owned(), "1".to_owned());
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn reserved_names_cover_code_execution_and_provider_credentials() {
        assert!(is_reserved_resource_name("ld_preload"));
        assert!(is_reserved_resource_name("OPENAI_API_KEY"));
        assert!(is_reserved_resource_name("LIMINA_DATABASE_URL"));
        assert!(!is_reserved_resource_name("DATASET_URL"));
        let (safe, blocked) =
            sanitize_project_environment(map(&[("PATH", "/x"), ("DATASET_URL", "s3://d")]));
        assert_eq!(blocked, vec!["PATH"]);
        assert_eq!(safe.len(), 1);
    }

    #[test]
    fn child_environments_never_inherit_control_plane_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let inherited = map(&[
            ("PATH", "/bin"),
            ("LIMINA_DATABASE_URL", "postgres://secret"),
            ("WORKOS_API_KEY", "sk_test"),
            ("OPENAI_API_KEY", "sk-openai"),
            ("ANTHROPIC_API_KEY", "sk-ant-runtime"),
        ]);
        let codex = codex_environment(
            &inherited,
            &map(&[("DATASET_URL", "s3://d")]),
            &dir.path().join("codex"),
        )
        .unwrap();
        assert_eq!(codex.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(codex.get("DATASET_URL").map(String::as_str), Some("s3://d"));
        for secret in
            ["LIMINA_DATABASE_URL", "WORKOS_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"]
        {
            assert!(!codex.contains_key(secret), "{secret} leaked into Codex");
        }
        let claude = claude_environment(
            &inherited,
            &map(&[("ANTHROPIC_API_KEY", ""), ("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat")]),
            &dir.path().join("claude"),
        )
        .unwrap();
        assert!(
            !claude.contains_key("ANTHROPIC_API_KEY"),
            "an empty overlay withholds the runtime key"
        );
        assert_eq!(claude.get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str), Some("sk-ant-oat"));
        assert_eq!(claude.get("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB").map(String::as_str), Some("1"));
    }

    #[test]
    fn claude_credentials_are_reported_by_name() {
        assert_eq!(
            claude_credential_source(&map(&[("CLAUDE_CODE_OAUTH_TOKEN", "x")])).as_deref(),
            Some("CLAUDE_CODE_OAUTH_TOKEN")
        );
        assert_eq!(
            claude_credential_source(&map(&[("CLAUDE_CODE_USE_BEDROCK", "1")])).as_deref(),
            Some("Amazon Bedrock")
        );
        assert_eq!(claude_credential_source(&map(&[("CLAUDE_CODE_USE_BEDROCK", "false")])), None);
    }
}
