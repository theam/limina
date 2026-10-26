//! Failures inside a provider protocol, before they become a `LiminaError`.
//!
//! Sessions turn these into the Python runtime's `TransportError`s ("The managed … turn failed
//! before Limina accepted its checkpoint.") with the provider's text as the `reason` detail.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    /// The provider process could not be started.
    #[error("{0}")]
    Spawn(String),
    /// A JSON-RPC error response from `codex app-server`.
    #[error("JSON-RPC error {code}: {message}")]
    Rpc { code: i64, message: String, data: Option<Value> },
    /// The provider closed its output, exited, or stopped answering.
    #[error("{0}")]
    Closed(String),
    /// The provider broke the protocol or reported a failure.
    #[error("{0}")]
    Protocol(String),
}

impl ProviderError {
    /// Transient overload, worth retrying the whole turn later. Mirrors the Codex SDK's
    /// `is_retryable_error`: a JSON-RPC error whose data says `server_overloaded`.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Rpc { data: Some(data), .. } if is_server_overloaded(data))
    }
}

fn is_overloaded_text(value: &Value) -> bool {
    value.as_str().is_some_and(|text| text.eq_ignore_ascii_case("server_overloaded"))
}

/// `_is_server_overloaded` from the Codex SDK, including its search through nested values.
pub(crate) fn is_server_overloaded(data: &Value) -> bool {
    match data {
        Value::String(_) => is_overloaded_text(data),
        Value::Object(map) => {
            let direct = ["codex_error_info", "codexErrorInfo", "errorInfo"]
                .iter()
                .filter_map(|key| map.get(*key))
                .find(|value| crate::text::truthy(value));
            if let Some(direct) = direct {
                if is_overloaded_text(direct) {
                    return true;
                }
                if let Value::Object(info) = direct
                    && info.values().any(is_overloaded_text)
                {
                    return true;
                }
            }
            map.values().any(is_server_overloaded)
        }
        Value::Array(items) => items.iter().any(is_server_overloaded),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overload_detection_matches_the_codex_sdk() {
        let rpc = |data: Value| ProviderError::Rpc {
            code: -32001,
            message: "busy".into(),
            data: Some(data),
        };
        assert!(rpc(json!("SERVER_OVERLOADED")).is_retryable());
        assert!(rpc(json!({"codex_error_info": "server_overloaded"})).is_retryable());
        assert!(rpc(json!({"codexErrorInfo": {"kind": "server_overloaded"}})).is_retryable());
        assert!(rpc(json!({"nested": [{"errorInfo": "server_overloaded"}]})).is_retryable());
        assert!(!rpc(json!({"codex_error_info": "unauthorized"})).is_retryable());
        assert!(!ProviderError::Closed("gone".into()).is_retryable());
        assert_eq!(rpc(json!(null)).to_string(), "JSON-RPC error -32001: busy");
    }
}
