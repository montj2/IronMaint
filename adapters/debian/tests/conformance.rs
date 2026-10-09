//! Shared conformance suite walks the production Debian adapter
//! end-to-end (PHASE-1 §10, §11, §12, §51; PHASE-0A.md §68).
//!
//! 1B.3 RED: the production adapter's descriptor advertises
//! `BuildPlanning` (a Phase 2 capability that the §11 "Do not
//! advertise yet" list forbids) and `build()` returns `None`.
//! The conformance arm at
//! `crates/ironmaint-testkit/src/conformance.rs:104-111` rejects
//! the inconsistency: "advertised `BuildPlanning` but `build()`
//! returns `None`." This test fails on the RED commit.
//!
//! 1B.3 GREEN: the descriptor no longer advertises
//! `BuildPlanning`; the conformance arm is no longer reached;
//! this test passes. The `verify-seams` S6 check is the
//! structural enforcement that prevents the regression.

use debian::DebianAdapter;
use ironmaint_testkit::conformance::assert_distribution_adapter_conformance;

#[test]
fn production_debian_passes_conformance() {
    assert_distribution_adapter_conformance(&DebianAdapter::new());
}
