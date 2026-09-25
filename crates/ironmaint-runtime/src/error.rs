//! Runtime error types.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    pub message: String,
}

impl RuntimeError {
    #[must_use]
    pub fn new(kind: RuntimeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeErrorKind {
    /// Transition was blocked by the state machine.
    #[error("transition blocked")]
    TransitionBlocked,
    /// Store error (see ironmaint-store).
    #[error("store error")]
    Store,
    /// Workspace error (see ironmaint-workspace).
    #[error("workspace error")]
    Workspace,
    /// Executor error.
    #[error("executor error")]
    Executor,
    /// Adapter produced no plan (or unknown family).
    #[error("adapter error")]
    Adapter,
    /// Invalid command input.
    #[error("invalid input")]
    InvalidInput,
    /// Other / unspecified.
    #[error("other: {0}")]
    Other(String),
}
