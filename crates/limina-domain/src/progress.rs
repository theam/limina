//! Evidence-backed project KPIs. Outcome measurements stay separate from activity counts.

use limina_core::time::{serde_iso, serde_iso_opt};
use limina_core::{LiminaError, Result, Timestamp, internal, now};
use limina_store::models::ProjectMetric;
use limina_store::{args, db::Arg};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::collaboration::{CollaborationService, knowledge_artifact, lock_project, project};
use crate::command;
use crate::common::record_event;

pub const MAX_PROJECT_METRICS: i64 = 50;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricInput {
    pub name: String,
    /// The success criterion this KPI measures.
    #[serde(default)]
    pub criterion: String,
    #[serde(default)]
    pub unit: String,
    /// `at_least` or `at_most`.
    pub direction: String,
    pub target: f64,
    #[serde(default)]
    pub baseline: Option<f64>,
    #[serde(default)]
    pub current: Option<f64>,
    #[serde(default)]
    pub evidence_id: Option<String>,
    #[serde(default, with = "serde_iso_opt")]
    pub observed_at: Option<Timestamp>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub id: String,
    #[serde(flatten)]
    pub input: MetricInput,
    pub version: i32,
    pub updated_by: String,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl TryFrom<ProjectMetric> for Metric {
    type Error = LiminaError;
    fn try_from(row: ProjectMetric) -> Result<Self> {
        Ok(Self {
            id: row.metric_id,
            input: serde_json::from_value(row.payload.0).map_err(internal)?,
            version: row.version,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        })
    }
}

impl MetricInput {
    pub fn parse(value: Value) -> Result<Self> {
        serde_json::from_value(value)
            .map_err(|error| LiminaError::invariant(format!("Invalid KPI fields: {error}")))
    }

    fn validate(&mut self) -> Result<()> {
        self.name = self.name.trim().to_owned();
        self.criterion = self.criterion.trim().to_owned();
        self.unit = self.unit.trim().to_owned();
        self.notes = self.notes.trim().to_owned();
        for (text, label, limit) in [
            (&self.name, "KPI name", 120),
            (&self.criterion, "Success criterion", 2000),
            (&self.unit, "Unit", 40),
            (&self.notes, "Measurement notes", 4000),
        ] {
            if text.chars().count() > limit {
                return Err(LiminaError::invariant(format!(
                    "{label} must be at most {limit} characters."
                )));
            }
        }
        if self.name.is_empty() {
            return Err(LiminaError::invariant("Give the KPI a name."));
        }
        if !matches!(self.direction.as_str(), "at_least" | "at_most") {
            return Err(LiminaError::invariant("KPI direction must be at_least or at_most."));
        }
        if ![Some(self.target), self.baseline, self.current]
            .into_iter()
            .flatten()
            .all(f64::is_finite)
        {
            return Err(LiminaError::invariant("KPI values must be finite numbers."));
        }
        self.evidence_id =
            self.evidence_id.take().map(|id| id.trim().to_uppercase()).filter(|id| !id.is_empty());
        if self.current.is_some() && self.evidence_id.is_none() {
            return Err(LiminaError::invariant(
                "A measured value needs an experiment or finding as evidence.",
            ));
        }
        if self.current.is_none() {
            self.evidence_id = None;
            self.observed_at = None;
        }
        Ok(())
    }
}

impl CollaborationService {
    pub async fn metrics(&self, slug: &str) -> Result<Vec<Metric>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let rows: Vec<ProjectMetric> = tx.fetch_all(
            "SELECT * FROM project_metrics WHERE challenge_id = $1 ORDER BY created_at, metric_id LIMIT $2",
            args![&challenge.id, MAX_PROJECT_METRICS],
        ).await?;
        tx.commit().await?;
        rows.into_iter().map(Metric::try_from).collect()
    }

    /// A full replacement, with optimistic concurrency and a durable idempotency receipt.
    pub async fn set_metric(
        &self,
        slug: &str,
        id: &str,
        mut input: MetricInput,
        expected_version: i32,
        actor: &str,
        command_id: &str,
    ) -> Result<Metric> {
        input.validate()?;
        if id.is_empty()
            || id.len() > 120
            || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(LiminaError::invariant(
                "KPI ID must use 1–120 letters, digits, dashes or underscores.",
            ));
        }
        command!(&self.db, command_id, "project.metric.updated", actor, |tx| {
            let challenge = lock_project(tx, slug).await?;
            if challenge.status == "ARCHIVED" {
                return Err(LiminaError::conflict("Archived project KPIs cannot be changed."));
            }
            let old: Option<ProjectMetric> = tx
                .fetch_optional(
                    "SELECT * FROM project_metrics WHERE challenge_id = $1 AND metric_id = $2",
                    args![&challenge.id, id],
                )
                .await?;
            let version = old.as_ref().map_or(0, |row| row.version);
            if expected_version != version {
                return Err(LiminaError::conflict(
                    "This KPI changed. Reload it before saving again.",
                ));
            }
            if old.is_none() {
                let count: i64 = tx.scalar(
                    "SELECT CAST(COUNT(*) AS BIGINT) FROM project_metrics WHERE challenge_id = $1",
                    args![&challenge.id],
                ).await?;
                if count >= MAX_PROJECT_METRICS {
                    return Err(LiminaError::invariant("A project can track at most 50 KPIs."));
                }
            }
            if let Some(id) = &input.evidence_id {
                let evidence = knowledge_artifact(tx, &challenge.id, id).await?;
                if !(evidence.kind == "F"
                    || (evidence.kind == "E" && evidence.status == "COMPLETED"))
                {
                    return Err(LiminaError::invariant(
                        "Use a finding or completed experiment as measurement evidence.",
                    ));
                }
            }
            let at = now();
            if input.current.is_some() && input.observed_at.is_none() {
                let previous = old
                    .as_ref()
                    .map(|row| MetricInput::parse(row.payload.0.clone()))
                    .transpose()?;
                input.observed_at = previous
                    .filter(|old| {
                        old.current == input.current && old.evidence_id == input.evidence_id
                    })
                    .and_then(|old| old.observed_at)
                    .or(Some(at));
            }
            let metric = Metric {
                id: id.to_owned(),
                input: input.clone(),
                version: version
                    .checked_add(1)
                    .ok_or_else(|| LiminaError::conflict("KPI version limit reached."))?,
                updated_by: actor.to_owned(),
                updated_at: at,
            };
            let payload = serde_json::to_value(&metric.input).map_err(internal)?;
            tx.execute(
                "INSERT INTO project_metrics (challenge_id, metric_id, payload, version, updated_by, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT (challenge_id, metric_id) \
                 DO UPDATE SET payload = EXCLUDED.payload, version = EXCLUDED.version, updated_by = EXCLUDED.updated_by, updated_at = EXCLUDED.updated_at",
                vec![Arg::from(&challenge.id), Arg::from(id), Arg::from(payload), Arg::from(metric.version), Arg::from(actor), Arg::from(at)],
            ).await?;
            record_event(
                tx,
                &challenge.id,
                "project.metric.updated",
                actor,
                command_id,
                json!({"metric":metric}),
                None,
            )
            .await?;
            Ok(metric)
        })
    }
}
