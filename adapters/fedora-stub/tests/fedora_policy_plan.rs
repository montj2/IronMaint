//! Fedora `policy_cap` conformance (§48).
//!
//! Verifies the Fedora-specific authority ordering (LocalPolicy
//! insertion) and the obligation templates produced by the stub.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fedora_stub::FedoraStubAdapter;
use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_policy::{AuthorityClassification, ObligationStrength};

#[test]
fn authority_order_inserts_local_policy() {
    let a = FedoraStubAdapter::new();
    let order = a.policy().unwrap().authority_order();
    assert_eq!(
        order,
        vec![
            AuthorityClassification::BestPractice,
            AuthorityClassification::TeamPolicy,
            AuthorityClassification::DistributionProcedure,
            AuthorityClassification::LocalPolicy,
            AuthorityClassification::FormalSpecification,
            AuthorityClassification::NormativePolicy,
        ]
    );
}

#[test]
fn policy_plan_includes_mandatory_and_local_policy_obligations() {
    let a = FedoraStubAdapter::new();
    let ctx = fedora_policy_context();
    let plan = a.policy().unwrap().derive_obligation_plan(&ctx).unwrap();
    assert_eq!(plan.obligation_templates.len(), 2);
    assert_eq!(
        plan.obligation_templates[0].strength,
        ObligationStrength::Mandatory
    );
    assert_eq!(
        plan.obligation_templates[1].strength,
        ObligationStrength::LocalPolicy
    );
}

fn fedora_policy_context() -> ironmaint_adapter_api::PolicyContext<'static> {
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
                        DistributionFamily::new("fedora").unwrap(),
                        DistributionRelease::new("rawhide").unwrap(),
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
