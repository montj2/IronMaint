//! `BuildTargetProvider` impl for the production Debian adapter.
//!
//! PHASE-2 PR 2.0-C (lock the build execution model,
//! `doc/PHASE-2-C.md`). For the first slice, the
//! production adapter returns the same default as
//! `BuildTarget::default_for_candidate` — `sid` +
//! host arch. Per-candidate overrides (e.g. from
//! the candidate's `debian/control`-equivalent
//! metadata) are 2A.2.

use ironmaint_adapter_api::{BuildTarget, BuildTargetProvider, CandidateContext};

use crate::DebianAdapter;

impl BuildTargetProvider for DebianAdapter {
    fn build_target(&self, ctx: &CandidateContext<'_>) -> BuildTarget {
        BuildTarget::default_for_candidate(ctx)
    }
}
