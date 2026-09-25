//! Debian `policy_cap` conformance (§48).
//!
//! Verifies the Debian-specific authority ordering and the obligation
//! templates produced by the stub.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use debian_stub::DebianStubAdapter;
use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_policy::{Applicability, AuthorityClassification, ObligationStrength};

#[test]
fn authority_order_matches_spec_48() {
    let a = DebianStubAdapter::new();
    let order = a.policy().unwrap().authority_order();
    assert_eq!(
        order,
        vec![
            AuthorityClassification::BestPractice,
            AuthorityClassification::TeamPolicy,
            AuthorityClassification::DistributionProcedure,
            AuthorityClassification::FormalSpecification,
            AuthorityClassification::NormativePolicy,
        ]
    );
}

#[test]
fn policy_plan_includes_mandatory_and_recommended_obligations() {
    let a = DebianStubAdapter::new();
    let ctx = crate::policy_context();
    let plan = a.policy().unwrap().derive_obligation_plan(&ctx).unwrap();
    assert_eq!(plan.obligation_templates.len(), 2);
    assert_eq!(
        plan.obligation_templates[0].strength,
        ObligationStrength::Mandatory
    );
    assert_eq!(
        plan.obligation_templates[0].applicability,
        Applicability::Applicable
    );
    assert_eq!(
        plan.obligation_templates[1].strength,
        ObligationStrength::Recommended
    );
}

fn policy_context() -> ironmaint_adapter_api::PolicyContext<'static> {
    use ironmaint_core::{
        DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId,
        JobId, PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef,
        SourceCandidate, VcsKind,
    };
    use ironmaint_policy::PolicyBaseline;
    use time::macros::datetime;
    use url::Url;

    let candidate: &'static SourceCandidate = Box::leak(Box::new({
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
            GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap(),
            GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap(),
            datetime!(2026-01-01 00:00:00 UTC),
        )
    }));
    ironmaint_adapter_api::PolicyContext {
        package: candidate.package(),
        candidate,
        requested_baseline: None::<&PolicyBaseline>,
    }
}
