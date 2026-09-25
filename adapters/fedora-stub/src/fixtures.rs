//! Shared test fixtures for the Fedora stub.

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

pub fn fedora_family() -> DistributionFamily {
    DistributionFamily::new("fedora").expect("static literal")
}

pub fn rawhide_release() -> DistributionRelease {
    DistributionRelease::new("rawhide").expect("static literal")
}

pub fn fedora_ref() -> DistributionRef {
    DistributionRef::new(fedora_family(), rawhide_release())
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
            PackageIdentity::new(fedora_ref(), PackageName::new("foo").unwrap()),
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
        Box::leak(Box::new(crate::descriptor::fedora_descriptor()));
    let candidate = Box::leak(Box::new(release_candidate()));
    ReleaseContext {
        release_candidate: candidate,
        adapter: descriptor,
    }
}

pub fn publication_context() -> PublicationContext<'static> {
    let descriptor: &'static AdapterDescriptor =
        Box::leak(Box::new(crate::descriptor::fedora_descriptor()));
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
        PolicyBaseline::new(fedora_ref()),
        datetime!(2026-01-02 00:00:00 UTC),
    )
}

#[allow(dead_code)]
pub fn sample_issue_action() -> IssueAction {
    issue_action_with_kind(IssueActionKind::SetSeverity, "blocking release")
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
