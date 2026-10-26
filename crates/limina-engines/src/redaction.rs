//! Replaces project secret values in events, decisions and error reasons with `[REDACTED]`.

use serde_json::Value;

use crate::events::RuntimeEvent;
use crate::turn::{RuntimeDecision, RuntimeTurn};

pub const REDACTED: &str = "[REDACTED]";

/// Replaces every non-empty secret, longest first, so a secret that contains another is
/// redacted whole.
pub fn redact_text<S: AsRef<str>>(value: &str, secrets: &[S]) -> String {
    let mut ordered: Vec<&str> =
        secrets.iter().map(AsRef::as_ref).filter(|secret| !secret.is_empty()).collect();
    ordered.sort_by_key(|secret| std::cmp::Reverse(secret.chars().count()));
    let mut redacted = value.to_owned();
    for secret in ordered {
        redacted = redacted.replace(secret, REDACTED);
    }
    redacted
}

/// Redacts strings anywhere inside a JSON value. Object keys are kept.
pub fn redact_value<S: AsRef<str>>(value: &Value, secrets: &[S]) -> Value {
    match value {
        Value::String(text) => Value::String(redact_text(text, secrets)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| redact_value(item, secrets)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter().map(|(key, item)| (key.clone(), redact_value(item, secrets))).collect(),
        ),
        other => other.clone(),
    }
}

pub fn redact_event<S: AsRef<str>>(event: &RuntimeEvent, secrets: &[S]) -> RuntimeEvent {
    RuntimeEvent {
        event_type: event.event_type.clone(),
        summary: redact_text(&event.summary, secrets),
        detail: event
            .detail
            .iter()
            .map(|(key, item)| (key.clone(), redact_value(item, secrets)))
            .collect(),
    }
}

/// Redacts the decision's prose. The status, ids and usage are kept.
pub fn redact_turn<S: AsRef<str>>(turn: &RuntimeTurn, secrets: &[S]) -> RuntimeTurn {
    let decision = &turn.decision;
    RuntimeTurn {
        continuation_id: turn.continuation_id.clone(),
        turn_id: turn.turn_id.clone(),
        decision: RuntimeDecision {
            summary: redact_text(&decision.summary, secrets),
            mission_update: decision.mission_update.as_ref().map(|update| {
                Box::new(crate::turn::MissionUpdate {
                    headline: redact_text(&update.headline, secrets),
                    established: redact_text(&update.established, secrets),
                    ruled_out: redact_text(&update.ruled_out, secrets),
                    uncertain: redact_text(&update.uncertain, secrets),
                    evidence: update.evidence.clone(),
                    decision: update.decision.as_ref().map(|d| crate::turn::MissionDecision {
                        question: redact_text(&d.question, secrets),
                        recommendation: redact_text(&d.recommendation, secrets),
                        trade_off: redact_text(&d.trade_off, secrets),
                    }),
                })
            }),
            status: decision.status.clone(),
            current_objective: redact_text(&decision.current_objective, secrets),
            next_step: redact_text(&decision.next_step, secrets),
            blocker: redact_text(&decision.blocker, secrets),
        },
        usage: turn.usage.clone(),
    }
}
