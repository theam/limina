//! Runtime clients: the public HTTP client, and the agent-side client that talks either to the
//! managed runtime's internal API or, in embedded mode, straight to the database.

pub mod http;
pub mod local;
pub mod protocol;

use std::path::{Component, Path, PathBuf};

use limina_core::{LiminaError, Result};
use serde_json::Value;

pub use http::{Command, HttpRuntimeClient};
pub use local::LocalRuntimeClient;
use protocol::{
    ExperimentBody, ExperimentClaimBody, ExperimentCompletionBody, FindingBody, HypothesisBody,
    HypothesisDecisionBody, ObservationBody,
};

/// The client `_agent` commands use: the internal API during a managed turn
/// (`LIMINA_INTERNAL_URL` + `LIMINA_INTERNAL_TOKEN`), otherwise the local database.
#[derive(Debug, Clone)]
pub enum AgentClient {
    Http(HttpRuntimeClient),
    Local(LocalRuntimeClient),
}

impl AgentClient {
    pub async fn status(&self, slug: &str) -> Result<Value> {
        match self {
            Self::Http(client) => client.status(slug).await,
            Self::Local(client) => client.status(slug).await,
        }
    }

    pub async fn artifacts(&self, slug: &str, kind: Option<&str>) -> Result<Value> {
        match self {
            Self::Http(client) => client.artifacts(slug, kind).await,
            Self::Local(client) => client.artifacts(slug, kind).await,
        }
    }

    pub async fn artifact(&self, slug: &str, artifact_id: &str) -> Result<Value> {
        match self {
            Self::Http(client) => client.artifact(slug, artifact_id).await,
            Self::Local(client) => client.artifact(slug, artifact_id).await,
        }
    }

    pub async fn create_hypothesis(
        &self,
        slug: &str,
        body: HypothesisBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        match self {
            Self::Http(client) => {
                client
                    .internal_command(slug, &["hypotheses"], &body, Command::by(actor, command_id))
                    .await
            }
            Self::Local(client) => client.create_hypothesis(slug, body, actor, command_id).await,
        }
    }

    pub async fn decide_hypothesis(
        &self,
        slug: &str,
        artifact_id: &str,
        body: HypothesisDecisionBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        match self {
            Self::Http(client) => {
                client
                    .internal_command(
                        slug,
                        &["hypotheses", artifact_id, "decision"],
                        &body,
                        Command::by(actor, command_id),
                    )
                    .await
            }
            Self::Local(client) => {
                client.decide_hypothesis(slug, artifact_id, body, actor, command_id).await
            }
        }
    }

    pub async fn create_experiment(
        &self,
        slug: &str,
        body: ExperimentBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        match self {
            Self::Http(client) => {
                client
                    .internal_command(slug, &["experiments"], &body, Command::by(actor, command_id))
                    .await
            }
            Self::Local(client) => client.create_experiment(slug, body, actor, command_id).await,
        }
    }

    pub async fn claim_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: ExperimentClaimBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        match self {
            Self::Http(client) => {
                client
                    .internal_command(
                        slug,
                        &["experiments", artifact_id, "claim"],
                        &body,
                        Command::by(actor, command_id),
                    )
                    .await
            }
            Self::Local(client) => {
                client.claim_experiment(slug, artifact_id, body, actor, command_id).await
            }
        }
    }

    pub async fn observe_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: ObservationBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        match self {
            Self::Http(client) => {
                client
                    .internal_command(
                        slug,
                        &["experiments", artifact_id, "observations"],
                        &body,
                        Command::by(actor, command_id),
                    )
                    .await
            }
            Self::Local(client) => {
                client.observe_experiment(slug, artifact_id, body, actor, command_id).await
            }
        }
    }

    pub async fn complete_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: ExperimentCompletionBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        match self {
            Self::Http(client) => {
                client
                    .internal_command(
                        slug,
                        &["experiments", artifact_id, "complete"],
                        &body,
                        Command::by(actor, command_id),
                    )
                    .await
            }
            Self::Local(client) => {
                client.complete_experiment(slug, artifact_id, body, actor, command_id).await
            }
        }
    }

    pub async fn publish_finding(
        &self,
        slug: &str,
        body: FindingBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        match self {
            Self::Http(client) => {
                client
                    .internal_command(slug, &["findings"], &body, Command::by(actor, command_id))
                    .await
            }
            Self::Local(client) => client.publish_finding(slug, body, actor, command_id).await,
        }
    }

    pub async fn close(&self) {
        if let Self::Local(client) = self {
            client.close().await;
        }
    }
}

/// Writes an export into `target`, which must be missing or empty so stale artifacts cannot
/// survive. Files are written in path order; returns the written paths.
pub async fn write_snapshot<I, P, C>(files: I, target: &Path) -> Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = (P, C)>,
    P: AsRef<str>,
    C: AsRef<[u8]>,
{
    let mut files: Vec<(P, C)> = files.into_iter().collect();
    files.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
    if tokio::fs::try_exists(target).await.map_err(|error| io_error(target, &error))? {
        let mut entries =
            tokio::fs::read_dir(target).await.map_err(|error| io_error(target, &error))?;
        if entries.next_entry().await.map_err(|error| io_error(target, &error))?.is_some() {
            return Err(LiminaError::conflict(format!(
                "Export target '{}' is not empty.",
                target.display()
            ))
            .with("target", target.display().to_string())
            .with(
                "suggestion",
                "Choose an empty directory so stale artifacts cannot survive the export.",
            ));
        }
    }
    let mut written = Vec::with_capacity(files.len());
    for (relative, content) in files {
        let relative = relative.as_ref();
        if !is_relative_inside(Path::new(relative)) {
            return Err(LiminaError::invariant(format!(
                "The export contains an unsafe path '{relative}'."
            ))
            .with("path", relative));
        }
        let path = target.join(relative);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| io_error(parent, &error))?;
        }
        tokio::fs::write(&path, content.as_ref()).await.map_err(|error| io_error(&path, &error))?;
        written.push(path);
    }
    Ok(written)
}

/// True for a non-empty relative path that stays below its base (no `..`, root or prefix).
fn is_relative_inside(path: &Path) -> bool {
    path.components().next().is_some()
        && path.components().all(|component| matches!(component, Component::Normal(_)))
}

fn io_error(path: &Path, error: &std::io::Error) -> LiminaError {
    LiminaError::generic(format!("Cannot write '{}': {error}.", path.display()))
        .with("path", path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshots_need_an_empty_target_and_safe_paths() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("kb");
        let written =
            write_snapshot([("research/b.md", "B"), ("ACTIVE.md", "A")], &target).await.unwrap();
        assert_eq!(written, vec![target.join("ACTIVE.md"), target.join("research/b.md")]);
        assert_eq!(std::fs::read_to_string(target.join("research/b.md")).unwrap(), "B");

        let error = write_snapshot([("ACTIVE.md", "A")], &target).await.unwrap_err();
        assert_eq!(error.code(), "version_conflict");
        assert_eq!(error.exit_code(), 4);

        let error =
            write_snapshot([("../escape.md", "x")], &dir.path().join("other")).await.unwrap_err();
        assert_eq!(error.code(), "invariant_violation");
    }
}
