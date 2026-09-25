//! Debian release capability (§52).
//!
//! Release metadata uses `debian/release` tags; the publication plan
//! proposes a canonical repository push and an issue-tracker mutation
//! (BTS tag update), the two privileged operations Debian's upload
//! workflow actually needs.

use std::sync::LazyLock;

use ironmaint_adapter_api::{
    AdapterError, PlannedOperation, PublicationContext, PublicationPlanTemplate, ReleaseCapability,
    ReleaseContext, ReleaseMetadataPlan,
};
use ironmaint_policy::PrivilegedOperationKind;

static DEBIAN_RELEASE: LazyLock<DebianRelease> = LazyLock::new(DebianRelease::new);

pub struct DebianRelease;

impl DebianRelease {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianRelease {
    fn default() -> Self {
        Self::new()
    }
}

impl ReleaseCapability for DebianRelease {
    fn release_metadata_plan(
        &self,
        _ctx: &ReleaseContext,
    ) -> Result<ReleaseMetadataPlan, AdapterError> {
        Ok(ReleaseMetadataPlan::new().with_tag("debian/release"))
    }

    fn publication_plan(
        &self,
        ctx: &PublicationContext,
    ) -> Result<PublicationPlanTemplate, AdapterError> {
        Ok(PublicationPlanTemplate::new(
            ctx.adapter
                .family
                .clone()
                .into_ref_distribution(ctx.release_candidate),
            "debian: push canonical and tag BTS entries",
        )
        .with_operation(PlannedOperation::new(
            PrivilegedOperationKind::CanonicalRepositoryPush,
            "git push origin main",
        ))
        .with_operation(PlannedOperation::new(
            PrivilegedOperationKind::IssueTrackerMutation,
            "tag BTS entries as fixed-by-release",
        )))
    }
}

#[must_use]
pub fn debian_release() -> &'static DebianRelease {
    &DEBIAN_RELEASE
}

/// Trait extension so the `AdapterDescriptor::family` (a
/// `DistributionFamily`) can be turned into a `DistributionRef` using
/// the release candidate's distribution context. Stubs pick whatever
/// `DistributionRef` they like — the cross-stub test only asserts
/// the *family* differs.
trait IntoRefDistribution {
    fn into_ref_distribution(
        self,
        candidate: &ironmaint_policy::ReleaseCandidate,
    ) -> ironmaint_core::DistributionRef;
}

impl IntoRefDistribution for ironmaint_core::DistributionFamily {
    fn into_ref_distribution(
        self,
        candidate: &ironmaint_policy::ReleaseCandidate,
    ) -> ironmaint_core::DistributionRef {
        // We assume the release candidate's policy baseline carries
        // the authoritative distribution ref; fall back to a known
        // release ("sid") if not.
        if candidate.policy_baseline.distribution.family == self {
            candidate.policy_baseline.distribution.clone()
        } else {
            ironmaint_core::DistributionRef::new(self, fallback_release())
        }
    }
}

/// `sid` is a static literal that satisfies the grammar — there is
/// no runtime fallback path. The narrow allow localises the panic.
#[allow(clippy::expect_used, clippy::unwrap_used)]
fn fallback_release() -> ironmaint_core::DistributionRelease {
    ironmaint_core::DistributionRelease::new("sid")
        .expect("static literal \"sid\" must satisfy DistributionRelease::new")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_metadata_has_debian_release_tag() {
        let r = DebianRelease::new();
        let plan = r
            .release_metadata_plan(&crate::fixtures::release_context())
            .unwrap();
        assert_eq!(plan.tags, vec!["debian/release"]);
    }

    #[test]
    fn publication_plan_proposes_push_and_issue_tracker_mutation() {
        let r = DebianRelease::new();
        let plan = r
            .publication_plan(&crate::fixtures::publication_context())
            .unwrap();
        assert_eq!(plan.operations.len(), 2);
        assert_eq!(
            plan.operations[0].kind,
            PrivilegedOperationKind::CanonicalRepositoryPush
        );
        assert_eq!(
            plan.operations[1].kind,
            PrivilegedOperationKind::IssueTrackerMutation
        );
    }

    #[test]
    fn publication_plan_uses_debian_family() {
        let r = DebianRelease::new();
        let plan = r
            .publication_plan(&crate::fixtures::publication_context())
            .unwrap();
        assert_eq!(plan.distribution.family.as_str(), "debian");
    }
}
