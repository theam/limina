//! Request bodies of the internal agent protocol (`/internal/v1/projects/{slug}/…`). Field names
//! and order are the JSON the Python CLI sent; embedded mode converts them into domain commands.

use limina_core::{LiminaError, Result};
use limina_domain::research::{
    ExperimentOutcome, HypothesisDecision, NewExperiment, NewFinding, NewHypothesis,
};
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HypothesisBody {
    pub title: String,
    pub statement: String,
    pub mechanism: String,
    pub generalization: String,
    pub shortcut_risks: String,
    pub test_plan: String,
}

impl From<HypothesisBody> for NewHypothesis {
    fn from(body: HypothesisBody) -> Self {
        Self {
            title: body.title,
            statement: body.statement,
            mechanism: body.mechanism,
            generalization: body.generalization,
            shortcut_risks: body.shortcut_risks,
            test_plan: body.test_plan,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HypothesisDecisionBody {
    pub status: String,
    pub conclusion: String,
    pub expected_version: i64,
}

impl HypothesisDecisionBody {
    pub fn into_domain(self) -> Result<HypothesisDecision> {
        Ok(HypothesisDecision {
            status: self.status,
            conclusion: self.conclusion,
            expected_version: expected_version(self.expected_version)?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExperimentBody {
    pub hypothesis_id: String,
    pub title: String,
    pub objective: String,
    pub procedure: String,
    pub success_criteria: String,
    pub guardrails: String,
}

impl From<ExperimentBody> for NewExperiment {
    fn from(body: ExperimentBody) -> Self {
        Self {
            hypothesis_id: body.hypothesis_id,
            title: body.title,
            objective: body.objective,
            procedure: body.procedure,
            success_criteria: body.success_criteria,
            guardrails: body.guardrails,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ExperimentClaimBody {
    pub ttl_seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ObservationBody {
    pub body: String,
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExperimentCompletionBody {
    pub results: String,
    pub analysis: String,
    pub decision: String,
    pub expected_version: i64,
}

impl ExperimentCompletionBody {
    pub fn into_domain(self) -> Result<ExperimentOutcome> {
        Ok(ExperimentOutcome {
            results: self.results,
            analysis: self.analysis,
            decision: self.decision,
            expected_version: expected_version(self.expected_version)?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FindingBody {
    pub experiment_id: String,
    pub title: String,
    pub finding: String,
    pub evidence: String,
    pub improvement: String,
    pub remaining_debt: String,
    pub next_move: String,
    pub impact: String,
}

impl From<FindingBody> for NewFinding {
    fn from(body: FindingBody) -> Self {
        Self {
            experiment_id: body.experiment_id,
            title: body.title,
            finding: body.finding,
            evidence: body.evidence,
            improvement: body.improvement,
            remaining_debt: body.remaining_debt,
            next_move: body.next_move,
            impact: body.impact,
        }
    }
}

/// Artifact versions are stored as `INTEGER`; a value outside its range can never match.
fn expected_version(value: i64) -> Result<i32> {
    i32::try_from(value).map_err(|_| {
        LiminaError::invariant("The expected version is out of range.")
            .with("expected_version", value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_serialize_with_the_python_field_order() {
        let body = ObservationBody { body: "seen".into(), evidence_ref: None };
        assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"body":"seen","evidence_ref":null}"#);
        let body = HypothesisDecisionBody {
            status: "CONFIRMED".into(),
            conclusion: "Yes.".into(),
            expected_version: 2,
        };
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"status":"CONFIRMED","conclusion":"Yes.","expected_version":2}"#
        );
        let too_big = HypothesisDecisionBody { expected_version: i64::MAX, ..body };
        assert_eq!(too_big.into_domain().unwrap_err().exit_code(), 5);
    }
}
