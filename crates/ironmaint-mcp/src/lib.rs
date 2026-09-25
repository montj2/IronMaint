//! `ironmaint-mcp` — MCP tool surface for the IronMaint runtime.
//!
//! PHASE-0B.md §50-§53. Each tool wraps a single `RuntimeCommand`
//! or `RuntimeQuery` and presents a JSON-schema-shaped input/
//! output pair. The transport (rmcp + Streamable HTTP on
//! 127.0.0.1) is wired in commit 15's bin target; the schema
//! generation is what `verify-mcp-schemas` snapshots.
//!
//! Auth: bearer-token from a config file. Every tool call is
//! validated before dispatch (PHASE-0B.md §51).

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
    )
)]

pub mod auth;
pub mod error;
pub mod schema;
pub mod tools;

pub use auth::{AuthError, AuthToken, TokenValidator};
pub use error::McpError;
pub use schema::{McpToolName, generate_all_schemas};
