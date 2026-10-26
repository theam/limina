//! What a managed turn returns: the checkpoint decision, continuation and usage.

use limina_core::{LiminaError, Result};
use limina_domain::usage::RuntimeUsage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(test)]
use crate::events::CHECKPOINT_FIELDS;
use crate::text::{py_str, strip};

/// Project statuses a checkpoint may report.
pub const CHECKPOINT_STATUSES: [&str; 3] = ["RUNNING", "WAITING", "COMPLETE"];

/// The JSON schema every turn's final answer must satisfy (Codex `outputSchema`, Claude
/// `--json-schema`).
pub fn turn_output_schema() -> Value {
    let decision = json!({
        "type": ["object", "null"], "additionalProperties": false,
        "properties": {
            "question": {"type": "string"},
            "recommendation": {"type": "string"},
            "trade_off": {"type": "string"}
        }, "required": ["question", "recommendation", "trade_off"]
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "summary": {"type": "string"},
            "status": {"type": "string", "enum": CHECKPOINT_STATUSES},
            "current_objective": {"type": "string"},
            "next_step": {"type": "string"},
            "blocker": {"type": "string"},
            "mission_update": {
                "type": ["object", "null"], "additionalProperties": false,
                "properties": {
                    "headline": {"type": "string"},
                    "established": {"type": "string"},
                    "ruled_out": {"type": "string"},
                    "uncertain": {"type": "string"},
                    "evidence": {"type": "array", "items": {"type": "string"}},
                    "decision": decision
                },
                "required": ["headline", "established", "ruled_out", "uncertain", "evidence", "decision"]
            },
        },
        "required": ["summary", "status", "current_objective", "next_step", "blocker", "mission_update"],
    })
}

/// A concise, cumulative assessment authored by the agent, never inferred from activity counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionUpdate {
    pub headline: String,
    pub established: String,
    pub ruled_out: String,
    pub uncertain: String,
    pub evidence: Vec<String>,
    pub decision: Option<MissionDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionDecision {
    pub question: String,
    pub recommendation: String,
    pub trade_off: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDecision {
    pub summary: String,
    pub status: String,
    pub current_objective: String,
    pub next_step: String,
    pub blocker: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_update: Option<Box<MissionUpdate>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTurn {
    /// The provider thread (Codex) or session (Claude) to resume next turn. Fakes may omit it.
    pub continuation_id: Option<String>,
    pub turn_id: String,
    pub decision: RuntimeDecision,
    pub usage: Option<RuntimeUsage>,
}

/// Parses a provider's structured checkpoint. A JSON string is decoded first; any other value
/// is read as the checkpoint object itself.
pub fn parse_runtime_decision(response: &Value, provider: &str) -> Result<RuntimeDecision> {
    let invalid =
        || LiminaError::transport(format!("{provider} returned an invalid project checkpoint."));
    let decoded;
    let value = match response {
        Value::String(text) => {
            decoded = serde_json::from_str::<Value>(text).map_err(|_| invalid())?;
            &decoded
        }
        other => other,
    };
    let object = value.as_object().ok_or_else(invalid)?;
    let field = |name: &str| object.get(name).map(py_str).ok_or_else(invalid);
    let blocker = strip(&field("blocker")?).to_owned();
    let decision = RuntimeDecision {
        summary: strip(&field("summary")?).to_owned(),
        status: field("status")?.to_uppercase(),
        current_objective: strip(&field("current_objective")?).to_owned(),
        next_step: strip(&field("next_step")?).to_owned(),
        blocker: if blocker.is_empty() { "None".to_owned() } else { blocker },
        mission_update: object
            .get("mission_update")
            .filter(|value| !value.is_null())
            .map(|value| {
                serde_json::from_value::<MissionUpdate>(value.clone())
                    .map(Box::new)
                    .map_err(|_| invalid())
            })
            .transpose()?,
    };
    if let Some(update) = &decision.mission_update {
        let texts = [&update.headline, &update.established, &update.ruled_out, &update.uncertain];
        if texts.iter().any(|text| text.trim().is_empty() || text.chars().count() > 600)
            || update.headline.chars().count() > 160
            || update.evidence.len() > 8
            || update.evidence.iter().collect::<std::collections::BTreeSet<_>>().len()
                != update.evidence.len()
            || (decision.status == "COMPLETE" && update.decision.is_some())
            || update.evidence.iter().any(|id| !valid_evidence_id(id))
            || update.decision.as_ref().is_some_and(|d| {
                [&d.question, &d.recommendation, &d.trade_off]
                    .iter()
                    .any(|text| text.trim().is_empty() || text.chars().count() > 600)
            })
        {
            return Err(invalid());
        }
    }
    if !CHECKPOINT_STATUSES.contains(&decision.status.as_str()) {
        return Err(LiminaError::transport(format!(
            "{provider} returned unsupported project status '{}'.",
            decision.status
        )));
    }
    if decision.summary.is_empty()
        || decision.current_objective.is_empty()
        || decision.next_step.is_empty()
    {
        return Err(LiminaError::transport(format!(
            "{provider} returned an incomplete project checkpoint."
        )));
    }
    Ok(decision)
}

fn valid_evidence_id(id: &str) -> bool {
    let digits = id
        .strip_prefix("CR")
        .or_else(|| id.strip_prefix("SR"))
        .or_else(|| id.strip_prefix(['H', 'E', 'F', 'L']));
    digits.is_some_and(|value| value.len() >= 3 && value.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint(status: &str) -> Value {
        json!({
            "summary": " Done ",
            "status": status,
            "current_objective": "Ship",
            "next_step": "Review",
            "blocker": "  ",
        })
    }

    #[test]
    fn checkpoints_parse_from_objects_and_json_strings() {
        let decision = parse_runtime_decision(&checkpoint("complete"), "Claude").unwrap();
        assert_eq!(decision.summary, "Done");
        assert_eq!(decision.status, "COMPLETE");
        assert_eq!(decision.blocker, "None");
        let text = Value::String(checkpoint("WAITING").to_string());
        assert_eq!(parse_runtime_decision(&text, "Codex").unwrap().status, "WAITING");
    }

    #[test]
    fn invalid_checkpoints_keep_the_python_messages() {
        let error = parse_runtime_decision(&json!("not json"), "Codex").unwrap_err();
        assert_eq!(error.message, "Codex returned an invalid project checkpoint.");
        assert_eq!(error.code(), "transport_error");
        let error = parse_runtime_decision(&json!({"summary": "x"}), "Claude").unwrap_err();
        assert_eq!(error.message, "Claude returned an invalid project checkpoint.");
        let error = parse_runtime_decision(&checkpoint("paused"), "Codex").unwrap_err();
        assert_eq!(error.message, "Codex returned unsupported project status 'PAUSED'.");
        let mut incomplete = checkpoint("RUNNING");
        incomplete["next_step"] = json!(" ");
        let error = parse_runtime_decision(&incomplete, "Codex").unwrap_err();
        assert_eq!(error.message, "Codex returned an incomplete project checkpoint.");
    }

    #[test]
    fn the_output_schema_requires_every_checkpoint_field() {
        let schema = turn_output_schema();
        for field in CHECKPOINT_FIELDS {
            assert!(schema["required"].as_array().unwrap().contains(&json!(field)));
        }
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["properties"]["status"]["enum"], json!(CHECKPOINT_STATUSES));
    }

    #[test]
    fn mission_assessments_reject_unbounded_prose_and_fake_reference_shapes() {
        let mut value = checkpoint("WAITING");
        value["mission_update"] = json!({
            "headline": "A guarded improvement", "established": "Supported on dev only.",
            "ruled_out": "No alternative has been ruled out yet.", "uncertain": "Test confirmation remains.",
            "evidence": ["F001", "E002"], "decision": {
                "question": "Keep the guardrail?", "recommendation": "Keep it.", "trade_off": "Less coverage otherwise."
            }
        });
        let parsed = parse_runtime_decision(&value, "Codex").unwrap();
        assert_eq!(parsed.mission_update.as_ref().unwrap().evidence, ["F001", "E002"]);
        let redacted = crate::redaction::redact_turn(
            &RuntimeTurn {
                continuation_id: None,
                turn_id: "turn".into(),
                decision: parsed,
                usage: None,
            },
            &["guardrail"],
        );
        assert!(
            redacted
                .decision
                .mission_update
                .unwrap()
                .decision
                .unwrap()
                .question
                .contains("[REDACTED]")
        );
        value["mission_update"]["evidence"] =
            json!(["https://invented.invalid", "../other-project/F001"]);
        assert!(parse_runtime_decision(&value, "Claude").is_err());
        value["mission_update"]["evidence"] = json!([]);
        value["mission_update"]["headline"] = json!("x".repeat(161));
        assert!(parse_runtime_decision(&value, "Codex").is_err());
    }
}
