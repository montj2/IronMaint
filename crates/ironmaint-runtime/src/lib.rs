//! `ironmaint-runtime` — application service layer.
//!
//! PHASE-0B.md §38. The runtime is the only thing the daemon
//! (0B.14) and MCP server (0B.15) talk to. It exposes a
//! command/query split:
//!
//! - Commands mutate state through the store and the state
//!   engine. They go through `ironmaint-state::TransitionEngine`,
//!   never through ad-hoc writes.
//! - Queries are read-only projections and never block a
//!   concurrent command.
//!
//! The runtime never touches distribution-specific state. It
//! speaks only in terms of `DistributionFamily`-tagged strings
//! (`DistributionRef` from core), opaque package versions, and
//! adapter capabilities. The two distribution adapters (debian-
//! stub, fedora-stub) are looked up by the daemon and passed to
//! the runtime on construction.

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

pub mod clock;
pub mod command;
pub mod config;
pub mod error;
pub mod next_actions;
pub mod orchestrator;
pub mod query;
pub mod service;

pub use clock::{Clock, FixedClock, SystemClock};
pub use command::RuntimeCommand;
pub use config::RuntimeConfig;
pub use error::{RuntimeError, RuntimeErrorKind};
pub use next_actions::{ActionBlocker, AllowedAction, JobNextActions};
pub use orchestrator::{OrchestratorKind, OrchestratorRef};
pub use query::RuntimeQuery;
pub use service::{CommandResult, QueryResult, RuntimeService};
