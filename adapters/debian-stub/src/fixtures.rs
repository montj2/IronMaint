//! Shared test fixtures for the Debian stub.
//!
//! Centralized so all per-module tests build identical
//! [`PackageRevision`]/[`SourceCandidate`] inputs and so the §46
//! acceptance test (cross-stub) can compare the same shapes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_adapter_api::{
    AdapterDescriptor, CandidateContext, PolicyContext, PublicationContext, ReleaseContext,
};
use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate,
    VcsKind,
};
use ironmaint_policy::{IssueAction, IssueActionKind, PolicyBaseline, ReleaseCandidate};
use time::macros::datetime;
use url::Url;

pub fn debian_family() -> DistributionFamily {
    DistributionFamily::new("debian").expect("static literal")
}

pub fn sid_release() -> DistributionRelease {
    DistributionRelease::new("sid").expect("static literal")
}

pub fn debian_ref() -> DistributionRef {
    DistributionRef::new(debian_family(), sid_release())
}

pub fn hex(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

pub fn source_candidate() -> SourceCandidate {
    let repo = RepositoryRef::new(
        VcsKind::Git,
        Url::parse("https://example.invalid/foo.git").unwrap(),
    )
    .unwrap();
    SourceCandidate::new(
        JobId::new(),
        PackageRevision::new(
            PackageIdentity::new(debian_ref(), PackageName::new("foo").unwrap()),
            PackageVersion::new("1.0.0").unwrap(),
        ),
        repo,
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('a', 40)).unwrap(),
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('b', 40)).unwrap(),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

pub fn candidate_context() -> CandidateContext<'static> {
    let candidate: &'static SourceCandidate = Box::leak(Box::new(source_candidate()));
    CandidateContext {
        package: candidate.package(),
        candidate,
    }
}

pub fn policy_context() -> PolicyContext<'static> {
    let candidate: &'static SourceCandidate = Box::leak(Box::new(source_candidate()));
    PolicyContext {
        package: candidate.package(),
        candidate,
        requested_baseline: None,
    }
}

pub fn release_context() -> ReleaseContext<'static> {
    let descriptor: &'static AdapterDescriptor =
        Box::leak(Box::new(crate::descriptor::debian_descriptor()));
    let candidate = Box::leak(Box::new(release_candidate()));
    ReleaseContext {
        release_candidate: candidate,
        adapter: descriptor,
    }
}

pub fn publication_context() -> PublicationContext<'static> {
    let descriptor: &'static AdapterDescriptor =
        Box::leak(Box::new(crate::descriptor::debian_descriptor()));
    let candidate = Box::leak(Box::new(release_candidate()));
    PublicationContext {
        release_candidate: candidate,
        adapter: descriptor,
    }
}

pub fn release_candidate() -> ReleaseCandidate {
    ReleaseCandidate::new(
        JobId::new(),
        ironmaint_core::CandidateFingerprint::from_hex(hex('c', 64)).unwrap(),
        PolicyBaseline::new(debian_ref()),
        datetime!(2026-01-02 00:00:00 UTC),
    )
}

#[allow(dead_code)]
pub fn sample_issue_action() -> IssueAction {
    issue_action_with_kind(IssueActionKind::MarkPending, "awaiting reproducer info")
}

#[allow(dead_code)]
pub fn issue_action_with_kind(kind: IssueActionKind, rationale: &str) -> IssueAction {
    IssueAction::new(
        ironmaint_core::IssueRef::new(crate::issues::provider_id(), "1234567".to_string()).unwrap(),
        None,
        kind,
        rationale.to_string(),
    )
    .unwrap()
}
