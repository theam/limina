//! Durable managed-turn retry policy.

use limina_core::LiminaError;

#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    pub delays_seconds: Vec<f64>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { delays_seconds: vec![30.0, 120.0, 600.0] }
    }
}

impl RetryPolicy {
    /// `LIMINA_RUNTIME_RETRY_DELAYS_SECONDS`, comma-separated (default `30,120,600`).
    pub fn from_environment() -> Result<Self, String> {
        let raw = std::env::var("LIMINA_RUNTIME_RETRY_DELAYS_SECONDS")
            .unwrap_or_else(|_| "30,120,600".to_owned());
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let delays = raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Runtime retry delays must be comma-separated seconds.".to_owned())?;
        if delays.is_empty() || delays.iter().any(|delay| *delay < 0.0 || !delay.is_finite()) {
            return Err("Runtime retry delays must contain non-negative seconds.".to_owned());
        }
        Ok(Self { delays_seconds: delays })
    }

    pub fn max_attempts(&self) -> usize {
        self.delays_seconds.len() + 1
    }

    pub fn delay_after(&self, retry_count: usize) -> Option<f64> {
        self.delays_seconds.get(retry_count).copied()
    }

    pub fn is_retryable(error: &LiminaError) -> bool {
        error.details.get("retryable").and_then(serde_json::Value::as_bool).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_parse_and_bound_retries() {
        let policy = RetryPolicy::parse("1, 2.5").unwrap();
        assert_eq!(policy.max_attempts(), 3);
        assert_eq!(policy.delay_after(1), Some(2.5));
        assert_eq!(policy.delay_after(2), None);
        assert!(RetryPolicy::parse("-1").is_err());
        assert!(RetryPolicy::parse("x").is_err());
        assert!(RetryPolicy::is_retryable(&LiminaError::transport("t").with("retryable", true)));
    }
}
