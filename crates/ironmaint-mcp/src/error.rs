//! MCP error type.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum McpError {
    #[error("auth error: {0}")]
    Auth(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// An optimistic-concurrency conflict: the caller's
    /// `expected_revision` is stale and the request is retryable
    /// after re-reading state. Kept distinct from
    /// [`McpError::Runtime`] so a client can tell "re-read and try
    /// again" from "this will never work" — the distinction is what
    /// makes `workspace.apply_patch`'s documented 409-shaped
    /// contract real rather than advisory.
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("internal: {0}")]
    Internal(String),
    #[error("{0}")]
    Other(String),
}
