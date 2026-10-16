//! `BuildTargetProvider` impls are part of the adapter
//! contract.
//!
//! PHASE-2 PR 2.0-C (lock the build execution model,
//! `doc/PHASE-2-C.md`). The Debian adapter and the
//! Debian stub both implement
//! [`ironmaint_adapter_api::BuildTargetProvider`];
//! the Fedora stub does too. The runtime integration
//! that asks the adapter for the target per candidate
//! is 2A.x's job; this test pins that the trait is
//! implementable on the adapter surface in 2.0-C, so
//! the type and the default are stable contracts.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_adapter_api::{
    BuildTarget, BuildTargetProvider, CandidateContext, host_arch,
};
use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId,
    JobId, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
    SourceCandidate, VcsKind,
};
use time::macros::datetime;
use url::Url;

fn hex(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

fn candidate() -> SourceCandidate {
    let repo = RepositoryRef::new(
        VcsKind::Git,
        Url::parse("https://example.invalid/foo.git").unwrap(),
    )
    .unwrap();
    SourceCandidate::new(
        JobId::new(),
        PackageRevision::new(
            PackageIdentity::new(
                DistributionRef::new(
                    DistributionFamily::new("debian").unwrap(),
                    DistributionRelease::new("sid").unwrap(),
                ),
                PackageName::new("foo").unwrap(),
            ),
            PackageVersion::new("1.0.0").unwrap(),
        ),
        repo,
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('a', 40)).unwrap(),
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('b', 40)).unwrap(),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

fn context<'a>(c: &'a SourceCandidate) -> CandidateContext<'a> {
    CandidateContext {
        package: c.package(),
        candidate: c,
    }
}

#[test]
fn debian_stub_provides_sid_plus_host_arch() {
    // The Debian stub's `BuildTargetProvider` impl
    // is a deliberate parity with the real
    // adapter's: both return the same default for
    // the first slice. 2A.2 will refactor the
    // stub to return a per-fixture override so
    // the conformance suite can test the
    // override path.
    let adapter = debian_stub::DebianStubAdapter::new();
    let c = candidate();
    let ctx = context(&c);
    let t: BuildTarget =
        <debian_stub::DebianStubAdapter as BuildTargetProvider>::build_target(&adapter, &ctx);
    assert_eq!(t.suite, "sid");
    assert_eq!(t.arch, host_arch());
}

#[test]
fn fedora_stub_provides_rawhide_plus_host_arch() {
    // The Fedora stub returns `rawhide` (the Fedora
    // moving-target suite), not `sid`. This is
    // the per-distribution choice the trait
    // exists to express; pinning the divergence
    // here means a future "let's unify the default"
    // refactor has to update this test.
    let adapter = fedora_stub::FedoraStubAdapter::new();
    let c = candidate();
    let ctx = context(&c);
    let t: BuildTarget =
        <fedora_stub::FedoraStubAdapter as BuildTargetProvider>::build_target(&adapter, &ctx);
    assert_eq!(t.suite, "rawhide");
    assert_eq!(t.arch, host_arch());
}
