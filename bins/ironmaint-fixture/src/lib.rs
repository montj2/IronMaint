//! Lib target for the `ironmaint-fixture` crate.
//!
//! The crate is primarily a binary (`src/main.rs`); this lib
//! exists so that downstream crates can add it as a dev-dep and
//! pick up `CARGO_BIN_EXE_ironmaint-fixture` at runtime. The
//! binary itself does not import from this lib.

#![allow(clippy::expect_used, clippy::unwrap_used)]

/// Stable name of the fixture binary, for assertions and
/// `CARGO_BIN_EXE_<name>` lookups.
pub const FIXTURE_BINARY_NAME: &str = "ironmaint-fixture";
