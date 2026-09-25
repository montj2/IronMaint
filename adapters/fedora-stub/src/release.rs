//! Fedora release capability (§52).
//!
//! Release metadata uses `fedora/release` tags; the publication plan
//! proposes a canonical repository push, a remote build submission
//! (Koji, in production), and a distribution-update creation (Bodhi, in
//! production). Distinct from Debian's two-operation plan (§80).

use std::sync::LazyLock;

use ironmaint_adapter_api::{
    AdapterError, PlannedOperation, PublicationContext, PublicationPlanTemplate, ReleaseCapability,
    ReleaseContext, ReleaseMetadataPlan,
};
use ironmaint_policy::PrivilegedOperationKind;

static FEDORA_RELEASE: LazyLock<FedoraRelease> = LazyLock::new(FedoraRelease::new);

pub struct FedoraRelease;

impl FedoraRelease {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FedoraRelease {
    fn default() -> Self {
        Self::new()
    }
}

impl ReleaseCapability for FedoraRelease {
    fn release_metadata_plan(
        &self,
        _ctx: &ReleaseContext,
    ) -> Result<ReleaseMetadataPlan, AdapterError> {
        Ok(ReleaseMetadataPlan::new().with_tag("fedora/release"))
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
            "fedora: push canonical, submit to koji, open bodhi update",
        )
        .with_operation(PlannedOperation::new(
            PrivilegedOperationKind::CanonicalRepositoryPush,
            "git push origin main",
        ))
        .with_operation(PlannedOperation::new(
            PrivilegedOperationKind::RemoteBuildSubmission,
            "submit koji build",
        ))
        .with_operation(PlannedOperation::new(
            PrivilegedOperationKind::DistributionUpdateCreation,
            "open bodhi update",
        )))
    }
}

#[must_use]
pub fn fedora_release() -> &'static FedoraRelease {
    &FEDORA_RELEASE
}

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
        if candidate.policy_baseline.distribution.family == self {
            candidate.policy_baseline.distribution.clone()
        } else {
            ironmaint_core::DistributionRef::new(self, fallback_release())
        }
    }
}

#[allow(clippy::expect_used, clippy::unwrap_used)]
fn fallback_release() -> ironmaint_core::DistributionRelease {
    ironmaint_core::DistributionRelease::new("rawhide")
        .expect("static literal \"rawhide\" must satisfy DistributionRelease::new")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_metadata_has_fedora_release_tag() {
        let r = FedoraRelease::new();
        let plan = r
            .release_metadata_plan(&crate::fixtures::release_context())
            .unwrap();
        assert_eq!(plan.tags, vec!["fedora/release"]);
    }

    #[test]
    fn publication_plan_proposes_three_operations() {
        let r = FedoraRelease::new();
        let plan = r
            .publication_plan(&crate::fixtures::publication_context())
            .unwrap();
        assert_eq!(plan.operations.len(), 3);
        assert_eq!(
            plan.operations[0].kind,
            PrivilegedOperationKind::CanonicalRepositoryPush
        );
        assert_eq!(
            plan.operations[1].kind,
            PrivilegedOperationKind::RemoteBuildSubmission
        );
        assert_eq!(
            plan.operations[2].kind,
            PrivilegedOperationKind::DistributionUpdateCreation
        );
    }

    #[test]
    fn publication_plan_uses_fedora_family() {
        let r = FedoraRelease::new();
        let plan = r
            .publication_plan(&crate::fixtures::publication_context())
            .unwrap();
        assert_eq!(plan.distribution.family.as_str(), "fedora");
    }
}
