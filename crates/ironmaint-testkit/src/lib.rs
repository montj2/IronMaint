//! `ironmaint-testkit` — shared fakes, fixtures, and the adapter
//! conformance suite.
//!
//! Implements PHASE-0A.md §68: [`assert_distribution_adapter_conformance`]
//! is the single public entry point that proves any
//! [`ironmaint_adapter_api::DistributionAdapter`] satisfies the Phase 0A
//! contract. Both `debian-stub` and `fedora-stub` are expected to pass it
//! (PHASE-0A.md §80 acceptance).
//!
//! ## Module map
//!
//! - [`fixtures`] — family-parameterized canonical fixtures used by
//!   both stubs and the conformance runner.
//! - [`conformance`] — [`assert_distribution_adapter_conformance`]
//!   orchestrator plus private per-capability helpers.
//!
//! ## Phase 0A.5 deferred to 0A.6
//!
//! - `cargo xtask verify-conformance` subcommand
//! - Public `fakes` API for downstream consumers
//!
//! Landed in 0A.6:
//! - JSON schema generation (`schemars`) and `insta` snapshot tests

#![forbid(unsafe_code)]

pub mod conformance;
pub mod fixtures;

pub use conformance::assert_distribution_adapter_conformance;
