//! The H → E → F research commands of [`ChallengeService`]: a finding needs a completed
//! experiment, and an experiment needs a hypothesis that is still being tested.

use limina_core::{LiminaError, Result, now};
use limina_store::args;
use limina_store::models::{Artifact, Observation};
use serde_json::{Value, json};

use crate::command;
use crate::common::{find_challenge, record_event};
use crate::service::internal::{
    NewArtifact, acquire_lease, cas_artifact, create_artifact, find_artifact, find_lease,
    record_revision, require_active_lease, require_text, strip, validate_lease_ttl,
};
use crate::service::{
    ArtifactDetail, ArtifactView, ChallengeService, ExperimentClaim, LeaseGrant, LeaseView,
    ObservationView, active_lease,
};

/// The hypothesis statuses a decision may set.
pub const HYPOTHESIS_DECISIONS: [&str; 3] = ["CONFIRMED", "REJECTED", "TESTING"];
/// A finding's impact levels.
pub const IMPACTS: [&str; 4] = ["CRITICAL", "HIGH", "MEDIUM", "LOW"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewHypothesis {
    pub title: String,
    pub statement: String,
    pub mechanism: String,
    pub generalization: String,
    pub shortcut_risks: String,
    pub test_plan: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HypothesisDecision {
    /// `CONFIRMED`, `REJECTED` or `TESTING` (case-insensitive).
    pub status: String,
    pub conclusion: String,
    pub expected_version: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewExperiment {
    pub hypothesis_id: String,
    pub title: String,
    pub objective: String,
    pub procedure: String,
    pub success_criteria: String,
    pub guardrails: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExperimentOutcome {
    pub results: String,
    pub analysis: String,
    pub decision: String,
    pub expected_version: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewFinding {
    pub experiment_id: String,
    pub title: String,
    pub finding: String,
    pub evidence: String,
    pub improvement: String,
    pub remaining_debt: String,
    pub next_move: String,
    /// `CRITICAL`, `HIGH`, `MEDIUM` or `LOW` (case-insensitive).
    pub impact: String,
}

impl ChallengeService {
    pub async fn create_hypothesis(
        &self,
        slug: &str,
        new: NewHypothesis,
        actor: &str,
        command_id: &str,
    ) -> Result<ArtifactView> {
        require_text("title", &new.title)?;
        require_text("statement", &new.statement)?;
        command!(&self.db, command_id, "hypothesis.create", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let artifact = create_artifact(
                tx,
                NewArtifact {
                    challenge_id: &challenge.id,
                    kind: "H",
                    title: strip(&new.title),
                    status: "PROPOSED",
                    payload: json!({
                        "statement": strip(&new.statement),
                        "mechanism": strip(&new.mechanism),
                        "generalization": strip(&new.generalization),
                        "shortcut_risks": strip(&new.shortcut_risks),
                        "test_plan": strip(&new.test_plan),
                        "conclusion": "",
                    }),
                    parent_hypothesis_id: None,
                    parent_experiment_id: None,
                    created_by: actor,
                },
                command_id,
            )
            .await?;
            record_event(
                tx,
                &challenge.id,
                "hypothesis.created",
                actor,
                command_id,
                json!({"title": artifact.title}),
                Some(&artifact.artifact_id),
            )
            .await?;
            Ok(ArtifactView::from(&artifact))
        })
    }

    /// Confirms or rejects a hypothesis (or returns it to testing) with a conclusion, if it is
    /// still at `expected_version`.
    pub async fn decide_hypothesis(
        &self,
        slug: &str,
        artifact_id: &str,
        decision: HypothesisDecision,
        actor: &str,
        command_id: &str,
    ) -> Result<ArtifactView> {
        let status = decision.status.to_uppercase();
        if !HYPOTHESIS_DECISIONS.contains(&status.as_str()) {
            return Err(LiminaError::invariant(
                "Hypothesis status must be CONFIRMED, REJECTED, or TESTING.",
            ));
        }
        require_text("conclusion", &decision.conclusion)?;
        command!(&self.db, command_id, "hypothesis.decide", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let artifact = find_artifact(tx, &challenge.id, artifact_id, Some("H")).await?;
            let mut payload = artifact.payload.0.clone();
            set_fields(&mut payload, [("conclusion", Value::from(strip(&decision.conclusion)))]);
            let artifact = cas_artifact(
                tx,
                &artifact,
                decision.expected_version,
                &status,
                payload,
                actor,
                command_id,
            )
            .await?;
            record_event(
                tx,
                &challenge.id,
                "hypothesis.decided",
                actor,
                command_id,
                json!({"status": status, "version": artifact.version}),
                Some(&artifact.artifact_id),
            )
            .await?;
            Ok(ArtifactView::from(&artifact))
        })
    }

    /// Designs an experiment under a hypothesis. The first experiment moves the hypothesis from
    /// `PROPOSED` to `TESTING`; concurrent experiments share only that transition.
    pub async fn create_experiment(
        &self,
        slug: &str,
        new: NewExperiment,
        actor: &str,
        command_id: &str,
    ) -> Result<ArtifactView> {
        require_text("title", &new.title)?;
        require_text("objective", &new.objective)?;
        command!(&self.db, command_id, "experiment.create", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let hypothesis =
                find_artifact(tx, &challenge.id, &new.hypothesis_id, Some("H")).await?;
            let transitioned: Option<Artifact> = tx
                .fetch_optional(
                    "UPDATE artifacts SET status = 'TESTING', version = version + 1, \
                     updated_at = $1 WHERE uid = $2 AND status = 'PROPOSED' RETURNING *",
                    args![now(), &hypothesis.uid],
                )
                .await?;
            if let Some(transitioned) = &transitioned {
                record_revision(tx, transitioned, actor, command_id).await?;
            } else {
                // Lock the hypothesis while it accepts the experiment, so a concurrent decision
                // cannot close it underneath.
                let accepts = tx
                    .execute(
                        "UPDATE artifacts SET updated_at = updated_at WHERE uid = $1 \
                         AND status = 'TESTING'",
                        args![&hypothesis.uid],
                    )
                    .await?;
                if accepts != 1 {
                    let current: Option<Artifact> = tx
                        .fetch_optional(
                            "SELECT * FROM artifacts WHERE uid = $1",
                            args![&hypothesis.uid],
                        )
                        .await?;
                    let current = current.unwrap_or_else(|| hypothesis.clone());
                    return Err(LiminaError::invariant(format!(
                        "Cannot add an experiment to {} because it is {}.",
                        current.artifact_id, current.status
                    ))
                    .with("artifact_id", current.artifact_id.as_str())
                    .with("status", current.status.as_str()));
                }
            }
            let artifact = create_artifact(
                tx,
                NewArtifact {
                    challenge_id: &challenge.id,
                    kind: "E",
                    title: strip(&new.title),
                    status: "DESIGNED",
                    payload: json!({
                        "objective": strip(&new.objective),
                        "procedure": strip(&new.procedure),
                        "success_criteria": strip(&new.success_criteria),
                        "guardrails": strip(&new.guardrails),
                        "results": "",
                        "analysis": "",
                        "decision": "",
                        "completed_at": null,
                    }),
                    parent_hypothesis_id: Some(&hypothesis.artifact_id),
                    parent_experiment_id: None,
                    created_by: actor,
                },
                command_id,
            )
            .await?;
            record_event(
                tx,
                &challenge.id,
                "experiment.created",
                actor,
                command_id,
                json!({"hypothesis_id": hypothesis.artifact_id, "title": artifact.title}),
                Some(&artifact.artifact_id),
            )
            .await?;
            Ok(ArtifactView::from(&artifact))
        })
    }

    /// Leases an experiment to `actor` for `ttl_seconds` (30 s – 24 h); the first claim starts
    /// it. Leases are per experiment, so lanes run in parallel.
    pub async fn claim_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        ttl_seconds: i64,
        actor: &str,
        command_id: &str,
    ) -> Result<ExperimentClaim> {
        validate_lease_ttl(ttl_seconds)?;
        command!(&self.db, command_id, "experiment.claim", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let mut artifact = find_artifact(tx, &challenge.id, artifact_id, Some("E")).await?;
            if !matches!(artifact.status.as_str(), "DESIGNED" | "RUNNING") {
                return Err(LiminaError::invariant(format!(
                    "Cannot claim {} while it is {}.",
                    artifact.artifact_id, artifact.status
                ))
                .with("artifact_id", artifact.artifact_id.as_str())
                .with("status", artifact.status.as_str()));
            }
            let claimed_at = now();
            let lease =
                acquire_lease(tx, &challenge.id, &artifact.artifact_id, actor, ttl_seconds).await?;
            if artifact.status == "DESIGNED" {
                artifact = tx
                    .fetch_one(
                        "UPDATE artifacts SET status = 'RUNNING', version = $1, updated_at = $2 \
                         WHERE uid = $3 RETURNING *",
                        args![artifact.version + 1, claimed_at, &artifact.uid],
                    )
                    .await?;
                record_revision(tx, &artifact, actor, command_id).await?;
            }
            record_event(
                tx,
                &challenge.id,
                "experiment.claimed",
                actor,
                command_id,
                json!({"expires_at": limina_core::iso(&lease.expires_at)}),
                Some(&artifact.artifact_id),
            )
            .await?;
            Ok(ExperimentClaim {
                artifact: ArtifactView::from(&artifact),
                lease: LeaseGrant::from(&lease),
            })
        })
    }

    /// Appends an observation to a running experiment the actor holds the lease on. Observations
    /// are append-only and never contend on the experiment's version.
    pub async fn append_observation(
        &self,
        slug: &str,
        artifact_id: &str,
        body: &str,
        evidence_ref: Option<&str>,
        actor: &str,
        command_id: &str,
    ) -> Result<ObservationView> {
        require_text("observation", body)?;
        command!(&self.db, command_id, "experiment.observe", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let artifact = find_artifact(tx, &challenge.id, artifact_id, Some("E")).await?;
            if artifact.status != "RUNNING" {
                return Err(LiminaError::invariant(format!(
                    "Observations require a RUNNING experiment; {} is {}.",
                    artifact.artifact_id, artifact.status
                ))
                .with("artifact_id", artifact.artifact_id.as_str())
                .with("status", artifact.status.as_str()));
            }
            require_active_lease(tx, &challenge.id, &artifact.artifact_id, actor).await?;
            let evidence_ref = evidence_ref.filter(|value| !value.is_empty()).map(strip);
            let observation: Observation = tx
                .fetch_one(
                    "INSERT INTO observations (id, challenge_id, experiment_id, body, \
                     evidence_ref, actor, command_id, created_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING *",
                    args![
                        limina_core::new_uuid(),
                        &challenge.id,
                        &artifact.artifact_id,
                        strip(body),
                        evidence_ref,
                        actor,
                        command_id,
                        now()
                    ],
                )
                .await?;
            record_event(
                tx,
                &challenge.id,
                "experiment.observed",
                actor,
                command_id,
                json!({"observation_id": observation.id, "evidence_ref": observation.evidence_ref}),
                Some(&artifact.artifact_id),
            )
            .await?;
            Ok(ObservationView::from(&observation))
        })
    }

    /// Completes a running experiment the actor holds the lease on, if it is still at
    /// `expected_version`, and releases the lease.
    pub async fn complete_experiment(
        &self,
        slug: &str,
        artifact_id: &str,
        outcome: ExperimentOutcome,
        actor: &str,
        command_id: &str,
    ) -> Result<ArtifactView> {
        require_text("results", &outcome.results)?;
        require_text("analysis", &outcome.analysis)?;
        require_text("decision", &outcome.decision)?;
        command!(&self.db, command_id, "experiment.complete", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let artifact = find_artifact(tx, &challenge.id, artifact_id, Some("E")).await?;
            if artifact.status != "RUNNING" {
                return Err(LiminaError::invariant(format!(
                    "Only a RUNNING experiment can complete; {} is {}.",
                    artifact.artifact_id, artifact.status
                ))
                .with("artifact_id", artifact.artifact_id.as_str())
                .with("status", artifact.status.as_str()));
            }
            require_active_lease(tx, &challenge.id, &artifact.artifact_id, actor).await?;
            let mut payload = artifact.payload.0.clone();
            set_fields(
                &mut payload,
                [
                    ("results", Value::from(strip(&outcome.results))),
                    ("analysis", Value::from(strip(&outcome.analysis))),
                    ("decision", Value::from(strip(&outcome.decision))),
                    (
                        "completed_at",
                        Value::from(now().date_naive().format("%Y-%m-%d").to_string()),
                    ),
                ],
            );
            let artifact = cas_artifact(
                tx,
                &artifact,
                outcome.expected_version,
                "COMPLETED",
                payload,
                actor,
                command_id,
            )
            .await?;
            tx.execute(
                "DELETE FROM work_leases WHERE challenge_id = $1 AND scope = $2 AND owner = $3",
                args![&challenge.id, &artifact.artifact_id, actor],
            )
            .await?;
            record_event(
                tx,
                &challenge.id,
                "experiment.completed",
                actor,
                command_id,
                json!({"version": artifact.version}),
                Some(&artifact.artifact_id),
            )
            .await?;
            Ok(ArtifactView::from(&artifact))
        })
    }

    /// Publishes a finding from a completed experiment; it links to the experiment and its
    /// hypothesis.
    pub async fn publish_finding(
        &self,
        slug: &str,
        new: NewFinding,
        actor: &str,
        command_id: &str,
    ) -> Result<ArtifactView> {
        require_text("title", &new.title)?;
        require_text("finding", &new.finding)?;
        require_text("evidence", &new.evidence)?;
        let impact = new.impact.to_uppercase();
        if !IMPACTS.contains(&impact.as_str()) {
            return Err(LiminaError::invariant("Impact must be CRITICAL, HIGH, MEDIUM, or LOW."));
        }
        command!(&self.db, command_id, "finding.publish", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let experiment =
                find_artifact(tx, &challenge.id, &new.experiment_id, Some("E")).await?;
            if experiment.status != "COMPLETED" {
                return Err(LiminaError::invariant(format!(
                    "Cannot publish a finding before {} is COMPLETED.",
                    experiment.artifact_id
                ))
                .with("artifact_id", experiment.artifact_id.as_str())
                .with("status", experiment.status.as_str()));
            }
            let hypothesis = find_artifact(
                tx,
                &challenge.id,
                experiment.parent_hypothesis_id.as_deref().unwrap_or_default(),
                Some("H"),
            )
            .await?;
            let artifact = create_artifact(
                tx,
                NewArtifact {
                    challenge_id: &challenge.id,
                    kind: "F",
                    title: strip(&new.title),
                    status: "PUBLISHED",
                    payload: json!({
                        "finding": strip(&new.finding),
                        "evidence": strip(&new.evidence),
                        "improvement": strip(&new.improvement),
                        "remaining_debt": strip(&new.remaining_debt),
                        "next_move": strip(&new.next_move),
                        "impact": impact,
                    }),
                    parent_hypothesis_id: Some(&hypothesis.artifact_id),
                    parent_experiment_id: Some(&experiment.artifact_id),
                    created_by: actor,
                },
                command_id,
            )
            .await?;
            record_event(
                tx,
                &challenge.id,
                "finding.published",
                actor,
                command_id,
                json!({
                    "hypothesis_id": hypothesis.artifact_id,
                    "experiment_id": experiment.artifact_id,
                    "impact": impact,
                }),
                Some(&artifact.artifact_id),
            )
            .await?;
            Ok(ArtifactView::from(&artifact))
        })
    }

    /// One artifact by display ID; experiments include their observations and active lease.
    pub async fn get_artifact(&self, slug: &str, artifact_id: &str) -> Result<ArtifactDetail> {
        let normalized = artifact_id.to_uppercase();
        if !is_artifact_id(&normalized) {
            return Err(LiminaError::not_found(format!(
                "Artifact '{artifact_id}' does not exist."
            ))
            .with("artifact_id", artifact_id));
        }
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let artifact = find_artifact(&mut tx, &challenge.id, &normalized, None).await?;
        let mut detail = ArtifactDetail {
            artifact: ArtifactView::from(&artifact),
            observations: None,
            lease: None,
        };
        if artifact.kind == "E" {
            let observations: Vec<Observation> = tx
                .fetch_all(
                    "SELECT * FROM observations WHERE challenge_id = $1 AND experiment_id = $2 \
                     ORDER BY created_at, id",
                    args![&challenge.id, &artifact.artifact_id],
                )
                .await?;
            detail.observations = Some(observations.iter().map(ObservationView::from).collect());
            let lease = find_lease(&mut tx, &challenge.id, &artifact.artifact_id).await?;
            detail.lease = active_lease(lease).as_ref().map(LeaseView::from);
        }
        tx.commit().await?;
        Ok(detail)
    }

    /// A project's artifacts by display ID, optionally of one kind.
    pub async fn list_artifacts(
        &self,
        slug: &str,
        kind: Option<&str>,
    ) -> Result<Vec<ArtifactView>> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let artifacts: Vec<Artifact> = match kind.filter(|kind| !kind.is_empty()) {
            Some(kind) => {
                tx.fetch_all(
                    "SELECT * FROM artifacts WHERE challenge_id = $1 AND kind = $2 \
                     ORDER BY artifact_id",
                    args![&challenge.id, kind.to_uppercase()],
                )
                .await?
            }
            None => {
                tx.fetch_all(
                    "SELECT * FROM artifacts WHERE challenge_id = $1 ORDER BY artifact_id",
                    args![&challenge.id],
                )
                .await?
            }
        };
        tx.commit().await?;
        Ok(artifacts.iter().map(ArtifactView::from).collect())
    }
}

/// `^(H|E|F|L|CR|SR)\d{3,}$`
fn is_artifact_id(value: &str) -> bool {
    let digits =
        ["CR", "SR", "H", "E", "F", "L"].iter().find_map(|prefix| value.strip_prefix(prefix));
    digits.is_some_and(|digits| digits.len() >= 3 && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Python's `dict.update`: existing keys keep their position, new keys are appended.
fn set_fields<const N: usize>(payload: &mut Value, fields: [(&str, Value); N]) {
    if !payload.is_object() {
        *payload = Value::Object(serde_json::Map::new());
    }
    if let Some(object) = payload.as_object_mut() {
        for (key, value) in fields {
            object.insert(key.to_owned(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_ids_follow_the_display_pattern() {
        for valid in ["H001", "E1234", "CR001", "SR010", "L999"] {
            assert!(is_artifact_id(valid), "{valid}");
        }
        for invalid in ["H01", "X001", "H001A", "", "CR", "HH001"] {
            assert!(!is_artifact_id(invalid), "{invalid}");
        }
    }

    #[test]
    fn payload_updates_keep_key_order() {
        let mut payload = json!({"a": 1, "conclusion": "", "z": 2});
        set_fields(&mut payload, [("conclusion", json!("done")), ("new", json!(true))]);
        assert_eq!(payload.to_string(), r#"{"a":1,"conclusion":"done","z":2,"new":true}"#);
    }
}
