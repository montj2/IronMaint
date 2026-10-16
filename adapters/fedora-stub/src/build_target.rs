//! `BuildTargetProvider` impl for the Fedora stub.
//!
//! PHASE-2 PR 2.0-C (lock the build execution model,
//! `doc/PHASE-2-C.md`). The Fedora stub returns
//! `rawhide` (the Fedora moving-target suite) + host
//! arch. The test
//! `crates/ironmaint-adapter-api/tests/build_target_provider_required.rs::fedora_stub_provides_rawhide_plus_host_arch`
//! pins the divergence from the Debian default.

use ironmaint_adapter_api::{BuildTarget, BuildTargetProvider, CandidateContext};

use crate::FedoraStubAdapter;

impl BuildTargetProvider for FedoraStubAdapter {
    fn build_target(&self, ctx: &CandidateContext<'_>) -> BuildTarget {
        // The Fedora default is `rawhide` + host arch.
        // The Debian default is `sid` + host arch. The
        // two are pinned separately so a future "unify
        // the default" refactor has to update both
        // tests and the doc.
        let mut t = BuildTarget::default_for_candidate(ctx);
        t.suite = "rawhide".to_string();
        t
    }
}
