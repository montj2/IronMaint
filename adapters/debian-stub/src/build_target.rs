//! `BuildTargetProvider` impl for the Debian stub.
//!
//! PHASE-2 PR 2.0-C (lock the build execution model,
//! `doc/PHASE-2-C.md`). The stub returns the same
//! default as the real adapter so the conformance
//! suite can pin the divergence in 2A.2.

use ironmaint_adapter_api::{BuildTarget, BuildTargetProvider, CandidateContext};

use crate::DebianStubAdapter;

impl BuildTargetProvider for DebianStubAdapter {
    fn build_target(&self, ctx: &CandidateContext<'_>) -> BuildTarget {
        BuildTarget::default_for_candidate(ctx)
    }
}
