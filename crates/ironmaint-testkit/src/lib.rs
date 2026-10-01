//! `ironmaint-testkit` — shared fakes, fixtures, and the conformance
//! suites.
//!
//! Implements PHASE-0A.md §68: [`assert_distribution_adapter_conformance`]
//! is the single public entry point that proves any
//! [`ironmaint_adapter_api::DistributionAdapter`] satisfies the Phase 0A
//! contract. Both `debian-stub` and `fedora-stub` are expected to pass it
//! (PHASE-0A.md §80 acceptance).
//!
//! Implements PHASE-0B.md §92: [`assert_executor_conformance`] is the
//! exit-checkpoint suite for any
//! [`ironmaint_executor::Executor`] backed by a registry of
//! `ironmaint-fixture` tool keys.
//!
//! ## Module map
//!
//! - [`fixtures`] — family-parameterized canonical fixtures used by
//!   both stubs and the conformance runner.
//! - [`conformance`] — [`assert_distribution_adapter_conformance`]
//!   orchestrator plus private per-capability helpers.
//! - [`executor_conformance`] — [`assert_executor_conformance`] plus
//!   [`register_fixture_tools`] / [`FixtureKeys`] for the §92 suite.
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
pub mod executor_conformance;
pub mod fixtures;
pub mod scenario;

pub use conformance::assert_distribution_adapter_conformance;
pub use executor_conformance::{
    FixtureKeys, assert_executor_conformance, fixture_binary_path, register_fixture_tools,
};
pub use scenario::{
    SCENARIO_OBLIGATION_REQUIREMENT, SCENARIO_STAGES, ScenarioAdapter, ScenarioScript,
    ScenarioTools, register_scenario_tools,
};
