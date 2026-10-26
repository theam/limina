//! `LocalRuntimeClient`: embedded mode, straight on the database through `limina-domain`.

use std::collections::BTreeMap;

use limina_core::{LiminaError, Result};
use limina_domain::exporter::MarkdownExporter;
use limina_domain::service::ChallengeService;
use limina_store::Db;
use serde::Serialize;
use serde_json::Value;

use super::protocol::{
    ExperimentBody, ExperimentClaimBody, ExperimentCompletionBody, FindingBody, HypothesisBody,
    HypothesisDecisionBody, ObservationBody,
};

/// The research service on a local database, for `_agent` commands outside a managed turn.
#[derive(Debug, Clone)]
pub struct LocalRuntimeClient {
    db: Db,
    service: ChallengeService,
}

impl LocalRuntimeClient {
    /// Connects to `database_url` and prepares the schema (SQLite is created in place;
    /// PostgreSQL must already be migrated with `limina db upgrade`).
    pub async fn connect(database_url: &str) -> Result<Self> {
        let db = connect(database_url).await?;
        db.initialize().await?;
        let service = ChallengeService::new(db.clone(), None);
        Ok(Self { db, service })
    }

    pub async fn close(&self) {
        self.db.close().await;
    }

    pub fn service(&self) -> &ChallengeService {
        &self.service
    }

    pub async fn health(&self) -> Result<Value> {
        let backend = match self.db.backend() {
            limina_store::db::Backend::Sqlite => "sqlite",
            limina_store::db::Backend::Postgres => "postgresql",
        };
        Ok(serde_json::json!({"ok": true, "mode": "embedded", "backend": backend}))
    }

    pub async fn status(&self, slug: &str) -> Result<Value> {
        to_value(self.service.status(slug).await?)
    }

    pub async fn artifacts(&self, slug: &str, kind: Option<&str>) -> Result<Value> {
        to_value(self.service.list_artifacts(slug, kind).await?)
    }

    pub async fn artifact(&self, slug: &str, artifact_id: &str) -> Result<Value> {
        to_value(self.service.get_artifact(slug, artifact_id).await?)
    }

    pub async fn create_hypothesis(
        &self,
        slug: &str,
        body: HypothesisBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        to_value(self.service.create_hypothesis(slug, body.into(), actor, command_id).await?)
    }

    pub async fn decide_hypothesis(
        &self,
        slug: &str,
        artifact_id: &str,
        body: HypothesisDecisionBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        let decision = body.into_domain()?;
        to_value(
            self.service.decide_hypothesis(slug, artifact_id, decision, actor, command_id).await?,
        )
    }

    pub async fn create_experiment(
        &self,
        slug: &str,
        body: ExperimentBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        to_value(self.service.create_experiment(slug, body.into(), actor, command_id).await?)
    }

    pub async fn claim_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: ExperimentClaimBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        to_value(
            self.service
                .claim_experiment(slug, artifact_id, body.ttl_seconds, actor, command_id)
                .await?,
        )
    }

    pub async fn observe_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: ObservationBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        to_value(
            self.service
                .append_observation(
                    slug,
                    artifact_id,
                    &body.body,
                    body.evidence_ref.as_deref(),
                    actor,
                    command_id,
                )
                .await?,
        )
    }

    pub async fn complete_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: ExperimentCompletionBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        let outcome = body.into_domain()?;
        to_value(
            self.service.complete_experiment(slug, artifact_id, outcome, actor, command_id).await?,
        )
    }

    pub async fn publish_finding(
        &self,
        slug: &str,
        body: FindingBody,
        actor: &str,
        command_id: &str,
    ) -> Result<Value> {
        to_value(self.service.publish_finding(slug, body.into(), actor, command_id).await?)
    }

    /// The Markdown export as `relative path → content`.
    pub async fn snapshot(&self, slug: &str) -> Result<BTreeMap<String, String>> {
        MarkdownExporter::new(self.service.clone()).snapshot(slug).await
    }
}

/// Connects to a database URL, keeping the driver's reason on failure.
pub async fn connect(database_url: &str) -> Result<Db> {
    Db::connect(database_url).await.map_err(|error| {
        let reason = error.to_string();
        LiminaError::from(error).with("reason", reason)
    })
}

fn to_value(value: impl Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(limina_core::internal)
}
