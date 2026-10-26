//! Provider token usage and cost for one turn, as `limina_domain::usage::RuntimeUsage`.

use limina_domain::usage::{RuntimeUsage, TokenPricing};
use serde_json::{Map, Value};

use crate::text::{py_int, truthy};

/// Normalizes a usage mapping with the snake_case keys both providers' Python SDKs exposed
/// (`input_tokens`, `output_tokens`, `cache_read_input_tokens` or `cached_input_tokens`,
/// `reasoning_output_tokens`, `total_tokens`) plus an optional `total_cost_usd`, then fills a
/// missing cost from operator prices. Returns `None` when nothing was reported.
pub fn usage_from_parts(
    raw_usage: &Map<String, Value>,
    total_cost_usd: Option<&Value>,
    pricing: Option<&TokenPricing>,
) -> Option<RuntimeUsage> {
    let int = |key: &str| raw_usage.get(key).and_then(py_int);
    // `cache_read_input_tokens or cached_input_tokens`: a zero or missing first value falls back.
    let cached = match raw_usage.get("cache_read_input_tokens") {
        Some(value) if truthy(value) => py_int(value),
        _ => int("cached_input_tokens"),
    };
    let cost = total_cost_usd.filter(|value| !value.is_null()).and_then(cost_microusd);
    let usage = RuntimeUsage {
        input_tokens: int("input_tokens"),
        output_tokens: int("output_tokens"),
        cached_input_tokens: cached,
        reasoning_output_tokens: int("reasoning_output_tokens"),
        total_tokens: int("total_tokens"),
        cost_microusd: cost,
        usage_source: (!raw_usage.is_empty()).then(|| "provider".to_owned()),
        cost_source: cost.map(|_| "provider".to_owned()),
    };
    usage.finalize(pricing)
}

/// `round(float(cost) * 1_000_000)`: USD to micro-USD with Python's round-half-to-even.
fn cost_microusd(cost: &Value) -> Option<i64> {
    let usd = match cost {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        _ => return None,
    };
    let micro = (usd * 1_000_000.0).round_ties_even();
    // `as` saturates; the range check keeps non-finite or huge values out.
    (micro.is_finite() && micro.abs() < 9.0e18).then_some(micro as i64)
}

/// Codex usage for one turn from a `thread/tokenUsage/updated` `tokenUsage` object.
///
/// `ThreadTokenUsage.total` is cumulative across resumed turns; `last` is the current turn's
/// delta and the only safe value for a per-run row.
pub fn codex_turn_usage(
    token_usage: Option<&Value>,
    pricing: Option<&TokenPricing>,
) -> Option<RuntimeUsage> {
    let mut raw = Map::new();
    if let Some(last) = token_usage.and_then(|usage| usage.get("last")).and_then(Value::as_object) {
        for (key, wire) in [
            ("input_tokens", Some("inputTokens")),
            ("output_tokens", Some("outputTokens")),
            ("cache_read_input_tokens", None),
            ("cached_input_tokens", Some("cachedInputTokens")),
            ("reasoning_output_tokens", Some("reasoningOutputTokens")),
            ("total_tokens", Some("totalTokens")),
        ] {
            let value = wire.and_then(|wire| last.get(wire)).cloned().unwrap_or(Value::Null);
            raw.insert(key.to_owned(), value);
        }
    }
    usage_from_parts(&raw, None, pricing)
}

/// Claude usage for one turn from its `result` message: `usage` plus `total_cost_usd`.
pub fn claude_result_usage(result: &Value) -> Option<RuntimeUsage> {
    let empty = Map::new();
    let raw = result.get("usage").and_then(Value::as_object).unwrap_or(&empty);
    usage_from_parts(raw, result.get("total_cost_usd"), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_usage_reports_tokens_and_provider_cost() {
        let usage = claude_result_usage(&json!({
            "usage": {"input_tokens": 1200, "cache_read_input_tokens": 300,
                      "cache_creation_input_tokens": 50, "output_tokens": 80},
            "total_cost_usd": 0.0123455,
        }))
        .unwrap();
        assert_eq!(usage.input_tokens, Some(1200));
        assert_eq!(usage.cached_input_tokens, Some(300));
        assert_eq!(usage.output_tokens, Some(80));
        assert_eq!(usage.total_tokens, None);
        assert_eq!(usage.cost_microusd, Some(12_346));
        assert_eq!(usage.usage_source.as_deref(), Some("provider"));
        assert_eq!(usage.cost_source.as_deref(), Some("provider"));
        assert!(claude_result_usage(&json!({"usage": {}})).is_none());
    }

    #[test]
    fn costs_round_half_to_even_like_python() {
        assert_eq!(cost_microusd(&json!(0.0000025)), Some(2));
        assert_eq!(cost_microusd(&json!(0.0000035)), Some(4));
        assert_eq!(cost_microusd(&json!("0.5")), Some(500_000));
        assert_eq!(cost_microusd(&json!(null)), None);
    }

    #[test]
    fn codex_usage_without_a_notification_is_empty() {
        assert!(codex_turn_usage(None, None).is_none());
        let zero = json!({"last": {"inputTokens": 0, "outputTokens": 0, "cachedInputTokens": 0,
                                   "reasoningOutputTokens": 0, "totalTokens": 0}});
        let usage = codex_turn_usage(Some(&zero), None).unwrap();
        assert_eq!(usage.input_tokens, Some(0));
        assert_eq!(usage.usage_source.as_deref(), Some("provider"));
    }
}
