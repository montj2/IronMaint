//! Canonical fixtures shared by both stubs and the conformance suite
//! (PHASE-0A.md §4.6).
//!
//! Every helper is parameterized on the family + release (and descriptor
//! for context builders) so `debian-stub` and `fedora-stub` share the
//! same code paths. The `*_for(d: &AdapterDescriptor)` builders are
//! used by the conformance runner; the bare builders are used by
//! stub tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_adapter_api::{
    AdapterDescriptor, CandidateContext, PolicyContext, PublicationContext, ReleaseContext,
};
use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, IssueProviderId, IssueRef, JobId, PackageIdentity, PackageName,
    PackageRevision, PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_policy::{IssueAction, IssueActionKind, PolicyBaseline, ReleaseCandidate};
use time::macros::datetime;
use url::Url;

// -----------------------------------------------------------------------------
// Identity helpers.
// -----------------------------------------------------------------------------

/// Build a `DistributionFamily` from a static literal. Used by stub
/// fixture wrappers and the conformance runner's per-family heuristic.
pub fn family(label: &str) -> DistributionFamily {
    DistributionFamily::new(label).expect("static literal must satisfy DistributionFamily::new")
}

/// Build a `DistributionRelease` from a static literal.
pub fn release(label: &str) -> DistributionRelease {
    DistributionRelease::new(label).expect("static literal must satisfy DistributionRelease::new")
}

/// Build a hex string by repeating `c` `n` times. Used to fabricate
/// deterministic commit / tree object IDs and fingerprints.
pub fn hex(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

// -----------------------------------------------------------------------------
// Core domain objects.
// -----------------------------------------------------------------------------

/// Canonical `SourceCandidate` for the given family + release.
pub fn source_candidate(
    family: DistributionFamily,
    release: DistributionRelease,
) -> SourceCandidate {
    let repo = RepositoryRef::new(
        VcsKind::Git,
        Url::parse("https://example.invalid/foo.git").expect("static URL parses"),
    )
    .expect("VcsKind::Git + valid URL yields a RepositoryRef");
    SourceCandidate::new(
        JobId::new(),
        PackageRevision::new(
            PackageIdentity::new(
                DistributionRef::new(family, release),
                PackageName::new("foo").expect("static literal must satisfy PackageName::new"),
            ),
            PackageVersion::new("1.0.0").expect("static literal must satisfy PackageVersion::new"),
        ),
        repo,
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('a', 40))
            .expect("40-char hex must satisfy GitObjectId::new"),
        GitObjectId::new(GitHashAlgorithm::Sha1, hex('b', 40))
            .expect("40-char hex must satisfy GitObjectId::new"),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

/// Canonical `ReleaseCandidate` for the given family + release.
pub fn release_candidate(
    family: DistributionFamily,
    release: DistributionRelease,
) -> ReleaseCandidate {
    ReleaseCandidate::new(
        JobId::new(),
        CandidateFingerprint::from_hex(hex('c', 64))
            .expect("64-char hex must satisfy CandidateFingerprint::from_hex"),
        PolicyBaseline::new(DistributionRef::new(family, release)),
        datetime!(2026-01-02 00:00:00 UTC),
    )
}

// -----------------------------------------------------------------------------
// Bare context builders (used by stub wrappers which own the leak).
// -----------------------------------------------------------------------------

pub fn candidate_context<'a>(c: &'a SourceCandidate) -> CandidateContext<'a> {
    CandidateContext {
        package: c.package(),
        candidate: c,
    }
}

pub fn policy_context<'a>(c: &'a SourceCandidate) -> PolicyContext<'a> {
    PolicyContext {
        package: c.package(),
        candidate: c,
        requested_baseline: None,
    }
}

pub fn release_context<'a>(
    rc: &'a ReleaseCandidate,
    d: &'a AdapterDescriptor,
) -> ReleaseContext<'a> {
    ReleaseContext {
        release_candidate: rc,
        adapter: d,
    }
}

pub fn publication_context<'a>(
    rc: &'a ReleaseCandidate,
    d: &'a AdapterDescriptor,
) -> PublicationContext<'a> {
    PublicationContext {
        release_candidate: rc,
        adapter: d,
    }
}

// -----------------------------------------------------------------------------
// Self-leaking context builders used by the conformance runner.
// -----------------------------------------------------------------------------

/// `'static` `PolicyContext` whose `SourceCandidate` carries the
/// family's release. The leak is intentional — the runner runs in a
/// single test process.
pub(crate) fn policy_context_for(d: &AdapterDescriptor) -> PolicyContext<'static> {
    let release = distribution_release_for(d);
    let candidate: &'static SourceCandidate =
        Box::leak(Box::new(source_candidate(d.family.clone(), release)));
    policy_context(candidate)
}

/// `'static` `CandidateContext` whose `SourceCandidate` carries the
/// family's release.
pub(crate) fn candidate_context_for(d: &AdapterDescriptor) -> CandidateContext<'static> {
    let release = distribution_release_for(d);
    let candidate: &'static SourceCandidate =
        Box::leak(Box::new(source_candidate(d.family.clone(), release)));
    candidate_context(candidate)
}

/// `'static` `ReleaseContext` whose `ReleaseCandidate` and
/// `AdapterDescriptor` are both leaked so the runner can hold them.
pub(crate) fn release_context_for(d: &AdapterDescriptor) -> ReleaseContext<'static> {
    let release = distribution_release_for(d);
    let rc: &'static ReleaseCandidate =
        Box::leak(Box::new(release_candidate(d.family.clone(), release)));
    let desc: &'static AdapterDescriptor = Box::leak(Box::new(d.clone()));
    release_context(rc, desc)
}

/// `'static` `PublicationContext`, mirroring [`release_context_for`].
pub(crate) fn publication_context_for(d: &AdapterDescriptor) -> PublicationContext<'static> {
    let release = distribution_release_for(d);
    let rc: &'static ReleaseCandidate =
        Box::leak(Box::new(release_candidate(d.family.clone(), release)));
    let desc: &'static AdapterDescriptor = Box::leak(Box::new(d.clone()));
    publication_context(rc, desc)
}

/// Pick a canonical release for the descriptor's family. This is a
/// fixture-only heuristic; the conformance runner body itself does
/// not branch on family identity (it inspects `descriptor.capabilities`).
fn distribution_release_for(d: &AdapterDescriptor) -> DistributionRelease {
    match d.family.as_str() {
        "debian" => release("sid"),
        "fedora" => release("rawhide"),
        other => release(other),
    }
}

// -----------------------------------------------------------------------------
// Issue-action helpers (used by stub fixtures for sample actions).
// -----------------------------------------------------------------------------

/// Canonical `IssueAction` for a given provider + kind + rationale.
/// Returns a fully-formed action in the `Proposed` authorization state.
#[allow(clippy::expect_used)]
pub fn sample_issue_action(
    provider: IssueProviderId,
    kind: IssueActionKind,
    rationale: &str,
) -> IssueAction {
    let issue = IssueRef::new(provider, "1234567".to_string())
        .expect("static literal must satisfy IssueRef::new");
    IssueAction::new(issue, None, kind, rationale.to_string())
        .expect("rationale within max length must satisfy IssueAction::new")
}
