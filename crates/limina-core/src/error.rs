//! Stable domain errors shared by the CLI, HTTP API and MCP server.
//!
//! Every error carries a machine-readable `code`, a human message and structured `details`. The
//! code, HTTP status and CLI exit code are part of the public contract.

use serde_json::{Map, Value};
use std::fmt;

pub type Result<T, E = LiminaError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// A generic, caller-correctable failure.
    Generic,
    NotFound,
    Conflict,
    Invariant,
    LeaseConflict,
    Authentication,
    Authorization,
    RateLimited,
    Transport,
}

impl ErrorKind {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Generic => "limina_error",
            Self::NotFound => "not_found",
            Self::Conflict => "version_conflict",
            Self::Invariant => "invariant_violation",
            Self::LeaseConflict => "lease_conflict",
            Self::Authentication => "authentication_required",
            Self::Authorization => "permission_denied",
            Self::RateLimited => "rate_limited",
            Self::Transport => "transport_error",
        }
    }

    pub const fn http_status(self) -> u16 {
        match self {
            Self::Generic => 400,
            Self::NotFound => 404,
            Self::Conflict | Self::LeaseConflict => 409,
            Self::Invariant => 422,
            Self::Authentication => 401,
            Self::Authorization => 403,
            Self::RateLimited => 429,
            Self::Transport => 503,
        }
    }

    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Generic => 1,
            Self::NotFound => 3,
            Self::Conflict => 4,
            Self::Invariant => 5,
            Self::LeaseConflict => 6,
            Self::Authentication | Self::Authorization => 7,
            Self::Transport => 8,
            Self::RateLimited => 9,
        }
    }
}

/// Why an error happened, when callers need to react to it. Never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A unique or foreign-key constraint rejected a write (usually a concurrent writer).
    Constraint,
    /// The database could not be reached or failed.
    Database,
}

#[derive(Debug, Clone, thiserror::Error)]
pub struct LiminaError {
    pub kind: ErrorKind,
    pub message: String,
    pub details: Map<String, Value>,
    /// Overrides the kind's code for errors that need a more specific one.
    pub code_override: Option<&'static str>,
    pub cause: Option<Cause>,
}

impl fmt::Display for LiminaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl LiminaError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            details: Map::new(),
            code_override: None,
            cause: None,
        }
    }

    pub fn generic(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Generic, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, message)
    }
    pub fn invariant(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invariant, message)
    }
    pub fn lease_conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::LeaseConflict, message)
    }
    pub fn authentication() -> Self {
        Self::new(ErrorKind::Authentication, "Authentication required.")
    }
    pub fn authentication_with(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Authentication, message)
    }
    pub fn authorization() -> Self {
        Self::new(ErrorKind::Authorization, "You do not have access to this project.")
    }
    pub fn authorization_with(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Authorization, message)
    }
    pub fn rate_limited(retry_after_seconds: u64) -> Self {
        Self::new(ErrorKind::RateLimited, "Too many failed authentication attempts.")
            .with("retry_after_seconds", retry_after_seconds)
    }
    pub fn transport(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Transport, message)
    }

    /// Adds one structured detail. `None` values are recorded as JSON `null`, like the Python
    /// runtime did.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.details.insert(key.to_owned(), value.into());
        self
    }

    #[must_use]
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code_override = Some(code);
        self
    }

    #[must_use]
    pub fn caused_by(mut self, cause: Cause) -> Self {
        self.cause = Some(cause);
        self
    }

    pub fn is_constraint_violation(&self) -> bool {
        self.cause == Some(Cause::Constraint)
    }

    pub fn code(&self) -> &'static str {
        self.code_override.unwrap_or_else(|| self.kind.code())
    }

    pub fn http_status(&self) -> u16 {
        self.kind.http_status()
    }

    pub fn exit_code(&self) -> i32 {
        self.kind.exit_code()
    }

    /// The public JSON body: `{"error": {"code", "message", "details"}}`.
    pub fn to_body(&self) -> Value {
        serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.message,
                "details": Value::Object(self.details.clone()),
            }
        })
    }
}

/// Internal failures (I/O, database, protocol) that are not the caller's fault.
pub fn internal(error: impl fmt::Display) -> LiminaError {
    LiminaError::transport(format!("Internal runtime error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_keep_the_public_contract() {
        let error = LiminaError::not_found("Challenge 'x' does not exist.").with("challenge", "x");
        assert_eq!(error.code(), "not_found");
        assert_eq!(error.http_status(), 404);
        assert_eq!(error.exit_code(), 3);
        assert_eq!(error.to_body()["error"]["details"]["challenge"], "x");
        assert_eq!(LiminaError::rate_limited(5).details["retry_after_seconds"], 5);
    }
}
