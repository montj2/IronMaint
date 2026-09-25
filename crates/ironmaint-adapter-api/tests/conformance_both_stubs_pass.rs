//! §68 + §80 acceptance: both stubs pass the same conformance suite.
//!
//! Run `assert_distribution_adapter_conformance` against both
//! `DebianStubAdapter` and `FedoraStubAdapter`. This is the cross-
//! distribution fit test — proves a Debian-shaped and a Fedora-shaped
//! workflow both fit the Phase 0A contracts without any per-
//! distribution branching inside core crates.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use debian_stub::DebianStubAdapter;
use fedora_stub::FedoraStubAdapter;
use ironmaint_testkit::assert_distribution_adapter_conformance;

#[test]
fn debian_stub_passes_conformance() {
    assert_distribution_adapter_conformance(&DebianStubAdapter::new());
}

#[test]
fn fedora_stub_passes_conformance() {
    assert_distribution_adapter_conformance(&FedoraStubAdapter::new());
}
