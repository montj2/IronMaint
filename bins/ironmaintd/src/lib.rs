//! `ironmaintd` — the IronMaint daemon.
//!
//! PHASE-0B.md §66-§68. The binary in `main.rs` is a thin
//! startup sequence; everything worth testing lives here.
//!
//! The crate is a library *and* a binary for one reason: the
//! daemon's configuration is the code most likely to be
//! silently wrong (a state directory in the wrong place is a
//! daemon that appears to work and loses the operator's data),
//! and a parser that cannot be called from a test is a parser
//! that will be.

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

pub mod config;

pub use config::{ConfigError, ConfigOutcome, RuntimeConfig};
