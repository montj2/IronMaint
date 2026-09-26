//! Executor error types.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorError {
    pub kind: ExecutorErrorKind,
    pub message: String,
}

impl ExecutorError {
    #[must_use]
    pub fn new(kind: ExecutorErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ExecutorError {}

/// Categories of executor failure. Mirrors PHASE-0B.md §33 —
/// `InfrastructureFailed` is distinct from `ToolFailed` so the
/// runtime can decide whether the cause is retriable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExecutorErrorKind {
    /// The tool returned a non-zero exit code, or its wall-clock
    /// budget elapsed (then `timed_out` is `true`).
    #[error("tool exit non-zero")]
    ToolFailed { timed_out: bool },
    /// Subprocess spawning, IO, or worker infrastructure failed.
    #[error("infrastructure failed")]
    InfrastructureFailed,
    /// Tool capability key not registered with the executor.
    #[error("unknown capability")]
    UnknownCapability,
    /// Invalid request input (e.g. malformed JSON, missing fields).
    #[error("invalid request")]
    InvalidRequest,
    /// Retry budget exhausted.
    #[error("retry exhausted")]
    RetryExhausted,
    /// Per-job artifact budget (`count` or `bytes`) exceeded
    /// (PHASE-0B.md §15). `cap` is one of `"count"` / `"bytes"`
    /// so callers can branch without parsing the message.
    #[error("artifact budget exceeded: {cap}")]
    ArtifactBudgetExceeded { cap: &'static str },
    /// Other / unspecified.
    #[error("other: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_failed_carries_timed_out_flag() {
        let k = ExecutorErrorKind::ToolFailed { timed_out: true };
        assert!(matches!(
            k,
            ExecutorErrorKind::ToolFailed { timed_out: true }
        ));
        let k2 = ExecutorErrorKind::ToolFailed { timed_out: false };
        assert!(matches!(
            k2,
            ExecutorErrorKind::ToolFailed { timed_out: false }
        ));
    }

    #[test]
    fn artifact_budget_exceeded_carries_cap_label() {
        let k = ExecutorErrorKind::ArtifactBudgetExceeded { cap: "count" };
        assert_eq!(
            k,
            ExecutorErrorKind::ArtifactBudgetExceeded { cap: "count" }
        );
        assert_ne!(
            k,
            ExecutorErrorKind::ArtifactBudgetExceeded { cap: "bytes" }
        );
    }

    #[test]
    fn error_display_contains_kind_and_message() {
        let e = ExecutorError::new(
            ExecutorErrorKind::ToolFailed { timed_out: false },
            "exit code 1",
        );
        let s = format!("{e}");
        assert!(s.contains("exit code 1"), "got: {s}");
    }
}
