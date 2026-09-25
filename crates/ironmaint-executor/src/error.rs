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
    /// The tool returned a non-zero exit code.
    #[error("tool exit non-zero")]
    ToolFailed,
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
    /// Other / unspecified.
    #[error("other: {0}")]
    Other(String),
}
