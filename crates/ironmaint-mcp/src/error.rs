//! MCP error type.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum McpError {
    #[error("auth error: {0}")]
    Auth(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("internal: {0}")]
    Internal(String),
}
