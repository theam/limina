//! Read-only mission assessments and deliberately published workspace deliverables.

use limina_core::{LiminaError, Principal, Result};
use limina_domain::collaboration::Role;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use super::ProjectOperations;

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const PREVIEW_BYTES: usize = 128 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 1000;

impl ProjectOperations {
    pub async fn knowledge_history(
        &self,
        slug: &str,
        id: &str,
        before: i32,
        principal: &Principal,
    ) -> Result<Value> {
        self.require(slug, principal, Role::Viewer).await?;
        let mut items = self.collaboration.revisions_before(slug, id, before).await?;
        let next = if items.len() > 25 {
            items.truncate(25);
            items.last().map(|item| item.version)
        } else {
            None
        };
        Ok(json!({"items": items, "next_before": next}))
    }

    pub async fn workspace_assessment(&self, slug: &str, principal: &Principal) -> Result<Value> {
        self.require(slug, principal, Role::Viewer).await?;
        let event = self.service.latest_event(slug, "runtime.turn_completed").await?;
        Ok(match event {
            Some(event) => json!({
                "sequence": event.sequence, "created_at": limina_core::iso(&event.created_at),
                "summary": event.payload.get("summary"), "mission_update": event.payload.get("mission_update"),
            }),
            None => Value::Null,
        })
    }

    /// Flat, bounded directory intentionally published by the agent. Never lists the workspace.
    pub async fn project_outputs(
        &self,
        slug: &str,
        offset: usize,
        principal: &Principal,
    ) -> Result<Value> {
        self.require(slug, principal, Role::Viewer).await?;
        let directory = self.output_directory(slug).await?;
        let mut entries = match tokio::fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"items": [], "next_offset": null, "truncated": false}));
            }
            Err(error) => return Err(limina_core::internal(error)),
        };
        let mut files = Vec::new();
        let mut scanned = 0;
        while scanned < MAX_DIRECTORY_ENTRIES {
            let Some(entry) = entries.next_entry().await.map_err(limina_core::internal)? else {
                break;
            };
            scanned += 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !valid_name(&name)
                || !entry.file_type().await.map_err(limina_core::internal)?.is_file()
            {
                continue;
            }
            let metadata = entry.metadata().await.map_err(limina_core::internal)?;
            if metadata.len() <= MAX_FILE_BYTES {
                files.push(json!({"name": name, "bytes": metadata.len()}));
            }
        }
        files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let next = (offset.saturating_add(25) < files.len()).then(|| offset.saturating_add(25));
        Ok(json!({"items": files.into_iter().skip(offset).take(25).collect::<Vec<_>>(),
            "next_offset": next, "truncated": scanned == MAX_DIRECTORY_ENTRIES}))
    }

    pub async fn output_preview(
        &self,
        slug: &str,
        name: &str,
        principal: &Principal,
    ) -> Result<Value> {
        let bytes = self.read_output(slug, name, principal).await?;
        let end = bytes.len().min(PREVIEW_BYTES);
        let text = std::str::from_utf8(&bytes).ok().map(|text| {
            let mut end = end;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text[..end].to_owned()
        });
        Ok(
            json!({"name": name, "text": text, "bytes": bytes.len(), "truncated": bytes.len() > PREVIEW_BYTES}),
        )
    }

    pub async fn read_output(
        &self,
        slug: &str,
        name: &str,
        principal: &Principal,
    ) -> Result<Vec<u8>> {
        self.require(slug, principal, Role::Viewer).await?;
        if !valid_name(name) {
            return Err(LiminaError::not_found("Output not found."));
        }
        let directory = self.output_directory(slug).await?;
        let path = directory.join(name);
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| LiminaError::not_found("Output not found."))?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(LiminaError::not_found("Output not found."));
        }
        // O_NOFOLLOW closes the check/open symlink race on the supported Unix runtime.
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file =
            options.open(&path).await.map_err(|_| LiminaError::not_found("Output not found."))?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(limina_core::internal)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(LiminaError::invariant("Output exceeds the download limit."));
        }
        Ok(bytes)
    }

    async fn output_directory(&self, slug: &str) -> Result<std::path::PathBuf> {
        let project = self.supervisor.workspace_root().join(slug.to_lowercase());
        let directory = project.join("outputs");
        if let Ok(metadata) = tokio::fs::symlink_metadata(&directory).await
            && !metadata.is_dir()
        {
            return Err(LiminaError::not_found("Outputs not found."));
        }
        // Also reject a symlinked project directory, keeping every read in this project.
        if let Ok(metadata) = tokio::fs::symlink_metadata(&project).await
            && !metadata.is_dir()
        {
            return Err(LiminaError::not_found("Outputs not found."));
        }
        Ok(directory)
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_names_cannot_escape_or_expose_hidden_files() {
        for name in ["", "..", ".env", "../secret", "sub/file", "sub\\file", "file\nname"] {
            assert!(!valid_name(name));
        }
        for name in ["comparison.csv", "research report.md", "síntesis.json"] {
            assert!(valid_name(name));
        }
    }
}
