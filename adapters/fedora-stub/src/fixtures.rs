//! Shared test fixtures for the Fedora stub.
//!
//! Thin wrappers over [`ironmaint_testkit::fixtures`] so the canonical
//! fixture code lives in one place (PHASE-0A.md §4.6). Every `pub fn`
//! name is preserved verbatim from the original hand-rolled fixture
//! module so the existing stub unit tests continue to call these
//! sites unchanged.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use ironmaint_adapter_api::{
    AdapterDescriptor, CandidateContext, PolicyContext, PublicationContext, ReleaseContext,
};
use ironmaint_core::{DistributionFamily, DistributionRef, DistributionRelease, SourceCandidate};
use ironmaint_policy::{IssueAction, IssueActionKind, ReleaseCandidate};

pub fn fedora_family() -> DistributionFamily {
    ironmaint_testkit::fixtures::family("fedora")
}

pub fn rawhide_release() -> DistributionRelease {
    ironmaint_testkit::fixtures::release("rawhide")
}

pub fn fedora_ref() -> DistributionRef {
    DistributionRef::new(fedora_family(), rawhide_release())
}

pub fn hex(c: char, n: usize) -> String {
    ironmaint_testkit::fixtures::hex(c, n)
}

pub fn source_candidate() -> SourceCandidate {
    ironmaint_testkit::fixtures::source_candidate(fedora_family(), rawhide_release())
}

pub fn release_candidate() -> ReleaseCandidate {
    ironmaint_testkit::fixtures::release_candidate(fedora_family(), rawhide_release())
}

pub fn candidate_context() -> CandidateContext<'static> {
    let c: &'static SourceCandidate = Box::leak(Box::new(source_candidate()));
    ironmaint_testkit::fixtures::candidate_context(c)
}

pub fn policy_context() -> PolicyContext<'static> {
    let c: &'static SourceCandidate = Box::leak(Box::new(source_candidate()));
    ironmaint_testkit::fixtures::policy_context(c)
}

pub fn release_context() -> ReleaseContext<'static> {
    let rc: &'static ReleaseCandidate = Box::leak(Box::new(release_candidate()));
    let desc: &'static AdapterDescriptor =
        Box::leak(Box::new(crate::descriptor::fedora_descriptor()));
    ironmaint_testkit::fixtures::release_context(rc, desc)
}

pub fn publication_context() -> PublicationContext<'static> {
    let rc: &'static ReleaseCandidate = Box::leak(Box::new(release_candidate()));
    let desc: &'static AdapterDescriptor =
        Box::leak(Box::new(crate::descriptor::fedora_descriptor()));
    ironmaint_testkit::fixtures::publication_context(rc, desc)
}

pub fn sample_issue_action() -> IssueAction {
    issue_action_with_kind(IssueActionKind::MarkPending, "awaiting reproducer info")
}

pub fn issue_action_with_kind(kind: IssueActionKind, rationale: &str) -> IssueAction {
    ironmaint_testkit::fixtures::sample_issue_action(crate::issues::provider_id(), kind, rationale)
}
