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
    /// The runtime does not implement this command, and will not
    /// until the surrounding system can support it honestly.
    ///
    /// This is *not* "not yet wired" — it means the boundary is
    /// real and the refusal is the design. `RequestApproval` is the
    /// motivating case: an approval needs a human principal, and
    /// the agent that would issue the request is exactly the party
    /// that must not be able to grant it. Returning `InvalidInput`
    /// or `Other` would both misdescribe that: the input is
    /// well-formed, and the failure is structural, not a bug to be
    /// fixed by a later commit.
    ///
    /// Mirrors [`ironmaint_adapter_api::AdapterErrorKind::Unsupported`]
    /// so both layers report "the capability does not exist" the
    /// same way on the wire.
    #[error("unsupported")]
    Unsupported,
    /// Optimistic-concurrency contention: another writer
    /// advanced the projection between read and write.
    /// Distinct from `Store` because callers (notably MCP
    /// and IronClaw) treat `ConcurrentModification` as a
    /// retryable surface, not an internal failure.
    #[error("concurrent modification")]
    ConcurrentModification,
    /// Other / unspecified.
    #[error("other: {0}")]
    Other(String),
}
