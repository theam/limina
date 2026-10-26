//! Provider usage per turn, and explicit operator-rate cost estimation.

use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cost_microusd: Option<i64>,
    /// `provider` when the provider reported the token counts.
    pub usage_source: Option<String>,
    /// `provider` or `operator_rate` (estimated from configured prices).
    pub cost_source: Option<String>,
}

impl RuntimeUsage {
    /// True when no count or cost was reported.
    pub fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cached_input_tokens.is_none()
            && self.reasoning_output_tokens.is_none()
            && self.total_tokens.is_none()
            && self.cost_microusd.is_none()
    }

    /// Fills a missing cost from operator prices and drops usage that reports nothing.
    pub fn finalize(mut self, pricing: Option<&TokenPricing>) -> Option<Self> {
        if let Some(pricing) = pricing
            && self.cost_microusd.is_none()
            && let Some(estimate) = pricing.estimate_microusd(&self)
        {
            self.cost_microusd = Some(estimate);
            self.cost_source = Some("operator_rate".to_owned());
        }
        (!self.is_empty()).then_some(self)
    }
}

/// Operator-configured USD prices per million tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenPricing {
    pub input_usd_per_million: Decimal,
    pub cached_input_usd_per_million: Decimal,
    pub output_usd_per_million: Decimal,
}

impl TokenPricing {
    /// Reads `<PREFIX>_INPUT_USD_PER_MILLION_TOKENS`, `<PREFIX>_CACHED_INPUT_…` and
    /// `<PREFIX>_OUTPUT_…`. All three or none must be set.
    pub fn from_environment(prefix: &str) -> Result<Option<Self>, String> {
        let names = [
            format!("{prefix}_INPUT_USD_PER_MILLION_TOKENS"),
            format!("{prefix}_CACHED_INPUT_USD_PER_MILLION_TOKENS"),
            format!("{prefix}_OUTPUT_USD_PER_MILLION_TOKENS"),
        ];
        let raw: Vec<String> = names
            .iter()
            .map(|name| std::env::var(name).unwrap_or_default().trim().to_owned())
            .collect();
        Self::from_values(&names, &raw)
    }

    /// Parses the three rates from already-read values (`raw` in the order of `names`, which
    /// only label error messages).
    pub fn from_values(names: &[String; 3], raw: &[String]) -> Result<Option<Self>, String> {
        if raw.iter().all(String::is_empty) {
            return Ok(None);
        }
        if raw.iter().any(String::is_empty) {
            let missing: Vec<&str> = names
                .iter()
                .zip(raw)
                .filter(|(_, value)| value.is_empty())
                .map(|(name, _)| name.as_str())
                .collect();
            return Err(format!(
                "Configure all token-price rates together; missing: {}.",
                missing.join(", ")
            ));
        }
        let invalid = || "Token-price rates must be non-negative decimal numbers.".to_owned();
        let parse = |value: &str| {
            Decimal::from_str(value).ok().filter(|d| !d.is_sign_negative()).ok_or_else(invalid)
        };
        Ok(Some(Self {
            input_usd_per_million: parse(&raw[0])?,
            cached_input_usd_per_million: parse(&raw[1])?,
            output_usd_per_million: parse(&raw[2])?,
        }))
    }

    pub fn estimate_microusd(&self, usage: &RuntimeUsage) -> Option<i64> {
        let input = usage.input_tokens?;
        let output = usage.output_tokens?;
        let cached = usage.cached_input_tokens.unwrap_or(0).clamp(0, input);
        // tokens × USD-per-million is numerically micro-USD because both scales are 1e6.
        let estimate = Decimal::from(input - cached) * self.input_usd_per_million
            + Decimal::from(cached) * self.cached_input_usd_per_million
            + Decimal::from(output) * self.output_usd_per_million;
        estimate.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero).try_into().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> [String; 3] {
        ["I".to_owned(), "C".to_owned(), "O".to_owned()]
    }

    #[test]
    fn operator_rates_estimate_missing_costs() {
        let pricing =
            TokenPricing::from_values(&names(), &["1.25".into(), "0.125".into(), "10".into()])
                .unwrap()
                .unwrap();
        let usage = RuntimeUsage {
            input_tokens: Some(1000),
            output_tokens: Some(100),
            cached_input_tokens: Some(400),
            usage_source: Some("provider".into()),
            ..RuntimeUsage::default()
        };
        // 600 × 1.25 + 400 × 0.125 + 100 × 10 = 750 + 50 + 1000
        let usage = usage.finalize(Some(&pricing)).unwrap();
        assert_eq!(usage.cost_microusd, Some(1800));
        assert_eq!(usage.cost_source.as_deref(), Some("operator_rate"));
        assert!(RuntimeUsage::default().finalize(Some(&pricing)).is_none());
    }

    #[test]
    fn partial_or_negative_rates_are_rejected() {
        assert!(
            TokenPricing::from_values(&names(), &["1".into(), "".into(), "1".into()])
                .unwrap_err()
                .contains("missing: C")
        );
        assert!(
            TokenPricing::from_values(&names(), &["-1".into(), "1".into(), "1".into()]).is_err()
        );
        assert_eq!(
            TokenPricing::from_values(&names(), &["".into(), "".into(), "".into()]).unwrap(),
            None
        );
    }
}
