//! Limina's `can_use_tool` policy for Claude: no unsandboxed commands, file tools stay inside
//! the project workspace, and MCP tools come only from the servers connected to the project.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::mcp::claude_tool_server;
use crate::text::{py_str, truthy};

pub const UNSANDBOXED_COMMAND_DENIED: &str =
    "Limina does not permit commands outside the project sandbox.";
pub const OUTSIDE_WORKSPACE_DENIED: &str =
    "Limina restricts file access to this project's workspace.";
pub const UNKNOWN_MCP_SERVER_DENIED: &str =
    "Limina only permits the MCP servers connected to this project.";
const MAX_SYMLINKS: usize = 40;

/// The answer to one permission request.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolPermission {
    /// Run the tool with this input (the original input, unchanged).
    Allow {
        updated_input: Value,
    },
    Deny {
        message: String,
    },
}

impl ToolPermission {
    /// The control-protocol response body, as the SDK sends it.
    pub fn response(&self) -> Value {
        match self {
            Self::Allow { updated_input } => {
                json!({"behavior": "allow", "updatedInput": updated_input})
            }
            Self::Deny { message } => json!({"behavior": "deny", "message": message}),
        }
    }
}

/// Decides tool permissions for one workspace.
#[derive(Debug, Clone)]
pub struct ToolPolicy {
    workspace_root: PathBuf,
    /// The connected MCP servers whose tools the turn may call.
    mcp_servers: BTreeSet<String>,
}

impl ToolPolicy {
    /// `workspace_root` must already be resolved (see [`resolve_lenient`]).
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root, mcp_servers: BTreeSet::new() }
    }

    /// Allows the tools of these MCP servers (every other MCP tool is denied).
    #[must_use]
    pub fn with_mcp_servers(mut self, servers: impl IntoIterator<Item = String>) -> Self {
        self.mcp_servers = servers.into_iter().collect();
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Decides a request. Touches the filesystem (symlinks are resolved), so call it off the
    /// async runtime.
    pub fn decide(&self, tool_name: &str, input: &Value) -> ToolPermission {
        let empty = Map::new();
        let values = input.as_object().unwrap_or(&empty);
        if tool_name == "Bash" && values.get("dangerouslyDisableSandbox").is_some_and(truthy) {
            return ToolPermission::Deny { message: UNSANDBOXED_COMMAND_DENIED.to_owned() };
        }
        if let Some(server) = claude_tool_server(tool_name) {
            return if self.mcp_servers.contains(server) {
                ToolPermission::Allow { updated_input: input.clone() }
            } else {
                ToolPermission::Deny { message: UNKNOWN_MCP_SERVER_DENIED.to_owned() }
            };
        }
        let path_key = match tool_name {
            "Read" | "Write" | "Edit" => Some("file_path"),
            "NotebookEdit" => Some("notebook_path"),
            "Glob" | "Grep" => Some("path"),
            _ => None,
        };
        if let Some(raw_path) = path_key.and_then(|key| values.get(key)).filter(|raw| truthy(raw)) {
            let mut candidate = PathBuf::from(py_str(raw_path));
            if !candidate.is_absolute() {
                candidate = self.workspace_root.join(candidate);
            }
            if !resolve_lenient(&candidate).starts_with(&self.workspace_root) {
                return ToolPermission::Deny { message: OUTSIDE_WORKSPACE_DENIED.to_owned() };
            }
        }
        ToolPermission::Allow { updated_input: input.clone() }
    }
}

enum Part {
    Parent,
    Name(OsString),
}

fn push_parts(stack: &mut Vec<Part>, path: &Path) {
    let parts: Vec<Part> = path
        .components()
        .filter_map(|component| match component {
            Component::ParentDir => Some(Part::Parent),
            Component::Normal(name) => Some(Part::Name(name.to_os_string())),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
        })
        .collect();
    stack.extend(parts.into_iter().rev());
}

/// Python's `Path.resolve()` (non-strict `realpath`): made absolute, symlinks followed component
/// by component, `..` applied to the resolved prefix, and missing components kept as written.
pub fn resolve_lenient(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")).join(path)
    };
    let mut stack = Vec::new();
    push_parts(&mut stack, &absolute);
    let mut resolved = PathBuf::from("/");
    let mut links_left = MAX_SYMLINKS;
    while let Some(part) = stack.pop() {
        let name = match part {
            Part::Parent => {
                resolved.pop();
                continue;
            }
            Part::Name(name) => name,
        };
        let candidate = resolved.join(&name);
        let is_link = std::fs::symlink_metadata(&candidate)
            .is_ok_and(|metadata| metadata.file_type().is_symlink());
        if is_link && links_left > 0 {
            links_left -= 1;
            if let Ok(target) = std::fs::read_link(&candidate) {
                if target.is_absolute() {
                    resolved = PathBuf::from("/");
                }
                push_parts(&mut stack, &target);
                continue;
            }
        }
        resolved = candidate;
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_components_and_parents_resolve_like_python() {
        let root = tempfile::tempdir().unwrap();
        let base = resolve_lenient(root.path());
        assert_eq!(resolve_lenient(&base.join("a/./b/../c")), base.join("a/c"));
        assert_eq!(
            resolve_lenient(&base.join("missing/../../x")),
            base.parent().unwrap().join("x")
        );
        assert_eq!(resolve_lenient(Path::new("/..")), PathBuf::from("/"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_followed_before_parent_components() {
        let root = tempfile::tempdir().unwrap();
        let base = resolve_lenient(root.path());
        let workspace = base.join("workspace");
        let outside = base.join("outside");
        std::fs::create_dir_all(workspace.join("sub")).unwrap();
        std::fs::create_dir_all(outside.join("deep")).unwrap();
        std::os::unix::fs::symlink(outside.join("deep"), workspace.join("escape")).unwrap();
        // `escape/..` is `outside`, not the workspace: the link resolves before `..` applies.
        assert_eq!(resolve_lenient(&workspace.join("escape/../secret")), outside.join("secret"));

        let policy = ToolPolicy::new(workspace.clone());
        let read = |path: &Path| {
            policy.decide("Read", &serde_json::json!({"file_path": path.to_string_lossy()}))
        };
        assert!(matches!(read(&workspace.join("sub/file.txt")), ToolPermission::Allow { .. }));
        assert!(matches!(read(Path::new("relative/inside.txt")), ToolPermission::Allow { .. }));
        assert_eq!(
            read(&workspace.join("escape/file.txt")),
            ToolPermission::Deny { message: OUTSIDE_WORKSPACE_DENIED.to_owned() }
        );
        assert!(matches!(read(&workspace.join("../outside/x")), ToolPermission::Deny { .. }));
    }

    #[test]
    fn unsandboxed_commands_are_denied_and_other_tools_allowed() {
        let policy = ToolPolicy::new(PathBuf::from("/workspace"));
        let input = serde_json::json!({"command": "ls", "dangerouslyDisableSandbox": true});
        assert_eq!(
            policy.decide("Bash", &input).response(),
            serde_json::json!({"behavior": "deny", "message": UNSANDBOXED_COMMAND_DENIED})
        );
        let input = serde_json::json!({"command": "ls", "dangerouslyDisableSandbox": false});
        assert_eq!(
            policy.decide("Bash", &input).response(),
            serde_json::json!({"behavior": "allow", "updatedInput": input})
        );
        let input = serde_json::json!({"url": "https://example.test"});
        assert!(matches!(policy.decide("WebFetch", &input), ToolPermission::Allow { .. }));
        let input = serde_json::json!({"pattern": "x", "path": ""});
        assert!(matches!(policy.decide("Grep", &input), ToolPermission::Allow { .. }));
    }

    #[test]
    fn only_connected_mcp_servers_are_allowed() {
        let input = serde_json::json!({"query": "roadmap"});
        let policy = ToolPolicy::new(PathBuf::from("/workspace"));
        assert_eq!(
            policy.decide("mcp__notes__search", &input),
            ToolPermission::Deny { message: UNKNOWN_MCP_SERVER_DENIED.to_owned() }
        );
        let policy = policy.with_mcp_servers(["notes".to_owned()]);
        assert_eq!(
            policy.decide("mcp__notes__search", &input),
            ToolPermission::Allow { updated_input: input.clone() }
        );
        assert!(matches!(policy.decide("mcp__other__search", &input), ToolPermission::Deny { .. }));
        assert!(matches!(policy.decide("mcp__notes_x__a", &input), ToolPermission::Deny { .. }));
    }
}
