//! Self-tests for the conformance runner.
//!
//! 1. [`local::LocalAdapter`] is a passing adapter; the runner must
//!    accept it.
//! 2. [`broken::BrokenStubAdapter`] deliberately violates one
//!    contract per [`broken::Brokenness`] variant; the runner must
//!    `panic!` for each.
//!
//! These tests live in `tests/` only — they are never published as
//! part of `ironmaint_testkit`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::map_err_ignore
)]

use ironmaint_adapter_api::{
    AdapterCapabilities, AdapterCapability, AdapterDescriptor, BuildCapability, BuildPlan,
    ChangedPath, DistributionAdapter, IssueCapability, PackageModelCapability, PathRole,
    PlannedCheck, PolicyCapability, PolicyPlan, PublicationPlanTemplate, QaPlan, ReleaseCapability,
    VersioningCapability,
};
use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, IssueProviderId, PackageName,
    PackageVersion,
};
use ironmaint_evidence::{ChangeDomain, EvidenceKind};
use ironmaint_policy::{
    AuthorityClassification, IssueActionKind, PolicyBaseline, PrivilegedOperationKind,
};
use ironmaint_testkit::assert_distribution_adapter_conformance;

// =============================================================================
// LocalAdapter — a passing adapter. Verifies the runner accepts a green-field
// implementation that satisfies every §68 contract.
// =============================================================================
mod local {
    use super::*;

    pub struct LocalAdapter;

    impl LocalAdapter {
        pub fn new() -> Self {
            Self
        }
    }

    fn descriptor() -> AdapterDescriptor {
        let mut caps = AdapterCapabilities::new();
        for c in [
            AdapterCapability::VersionComparison,
            AdapterCapability::PolicyDerivation,
            AdapterCapability::BuildPlanning,
            AdapterCapability::PackageQaPlanning,
            AdapterCapability::IssueRead,
            AdapterCapability::IssueWrite,
            AdapterCapability::ReleaseMetadata,
            AdapterCapability::PublicationPlanning,
        ] {
            caps.insert(c);
        }
        AdapterDescriptor {
            family: DistributionFamily::new("local").unwrap(),
            implementation_name: "local-stub".into(),
            implementation_version: "0.1.0".into(),
            capabilities: caps,
        }
    }

    fn dref() -> DistributionRef {
        DistributionRef::new(
            DistributionFamily::new("local").unwrap(),
            DistributionRelease::new("local-release").unwrap(),
        )
    }

    pub struct LocalVersioning;
    impl VersioningCapability for LocalVersioning {
        fn validate(&self, _: &PackageVersion) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Ok(())
        }
        fn compare(
            &self,
            a: &PackageVersion,
            b: &PackageVersion,
        ) -> Result<std::cmp::Ordering, ironmaint_adapter_api::AdapterError> {
            Ok(a.as_str().cmp(b.as_str()))
        }
    }

    pub struct LocalPackageModel;
    impl PackageModelCapability for LocalPackageModel {
        fn validate_name(
            &self,
            _: &PackageName,
        ) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Ok(())
        }
        fn classify_changes(
            &self,
            changes: &[ChangedPath],
        ) -> Result<Vec<ChangeDomain>, ironmaint_adapter_api::AdapterError> {
            Ok(changes
                .iter()
                .map(|c| match c.role {
                    PathRole::PackagingMetadata => ChangeDomain::PackagingMetadata,
                    PathRole::DocumentationOnly => ChangeDomain::DocumentationOnly,
                    _ => ChangeDomain::AdapterSpecific("other".to_string()),
                })
                .collect())
        }
    }

    pub struct LocalPolicy;
    impl PolicyCapability for LocalPolicy {
        fn authority_order(&self) -> Vec<AuthorityClassification> {
            vec![AuthorityClassification::BestPractice]
        }
        fn derive_obligation_plan(
            &self,
            _ctx: &ironmaint_adapter_api::PolicyContext,
        ) -> Result<PolicyPlan, ironmaint_adapter_api::AdapterError> {
            Ok(PolicyPlan {
                baseline: PolicyBaseline::new(dref()),
                obligation_templates: vec![],
            })
        }
    }

    pub struct LocalBuild;
    impl BuildCapability for LocalBuild {
        fn build_plan(
            &self,
            _: &ironmaint_adapter_api::CandidateContext,
        ) -> Result<BuildPlan, ironmaint_adapter_api::AdapterError> {
            Ok(BuildPlan::new().with_check(PlannedCheck::new(
                ironmaint_adapter_api::ToolCapabilityKey::new("local.build.fake").unwrap(),
                EvidenceKind::Build,
                true,
            )))
        }
        fn qa_plan(
            &self,
            _: &ironmaint_adapter_api::CandidateContext,
        ) -> Result<QaPlan, ironmaint_adapter_api::AdapterError> {
            Ok(QaPlan::new().with_check(PlannedCheck::new(
                ironmaint_adapter_api::ToolCapabilityKey::new("local.qa.fake").unwrap(),
                EvidenceKind::PackageQa,
                true,
            )))
        }
    }

    pub struct LocalIssues;
    impl IssueCapability for LocalIssues {
        fn provider_id(&self) -> IssueProviderId {
            IssueProviderId::new()
        }
        fn supported_actions(&self) -> Vec<IssueActionKind> {
            vec![IssueActionKind::Comment]
        }
        fn validate_action(
            &self,
            _: &ironmaint_policy::IssueAction,
        ) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Ok(())
        }
    }

    pub struct LocalRelease;
    impl ReleaseCapability for LocalRelease {
        fn release_metadata_plan(
            &self,
            _: &ironmaint_adapter_api::ReleaseContext,
        ) -> Result<ironmaint_adapter_api::ReleaseMetadataPlan, ironmaint_adapter_api::AdapterError>
        {
            Ok(ironmaint_adapter_api::ReleaseMetadataPlan::new().with_tag("local/release"))
        }
        fn publication_plan(
            &self,
            _: &ironmaint_adapter_api::PublicationContext,
        ) -> Result<PublicationPlanTemplate, ironmaint_adapter_api::AdapterError> {
            Ok(
                PublicationPlanTemplate::new(dref(), "local publish").with_operation(
                    ironmaint_adapter_api::PlannedOperation::new(
                        PrivilegedOperationKind::CanonicalRepositoryPush,
                        "push",
                    ),
                ),
            )
        }
    }

    impl DistributionAdapter for LocalAdapter {
        fn descriptor(&self) -> AdapterDescriptor {
            descriptor()
        }
        fn versioning(&self) -> &dyn VersioningCapability {
            static V: LocalVersioning = LocalVersioning;
            &V
        }
        fn package_model(&self) -> &dyn PackageModelCapability {
            static P: LocalPackageModel = LocalPackageModel;
            &P
        }
        fn policy(&self) -> Option<&dyn PolicyCapability> {
            static P: LocalPolicy = LocalPolicy;
            Some(&P)
        }
        fn build(&self) -> Option<&dyn BuildCapability> {
            static B: LocalBuild = LocalBuild;
            Some(&B)
        }
        fn issues(&self) -> Option<&dyn IssueCapability> {
            static I: LocalIssues = LocalIssues;
            Some(&I)
        }
        fn release(&self) -> Option<&dyn ReleaseCapability> {
            static R: LocalRelease = LocalRelease;
            Some(&R)
        }
    }
}

#[test]
fn runner_accepts_a_passing_adapter() {
    assert_distribution_adapter_conformance(&local::LocalAdapter::new());
}

// =============================================================================
// BrokenStubAdapter — one variant per contract violation. Each variant
// exercises a single §68 item; the runner must panic for each.
// =============================================================================
mod broken {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Brokenness {
        // EmptyFamily omitted: `DistributionFamily::new("")` returns Err
        // and there is no public constructor that yields an empty
        // family. The runner's empty-family check still exists in
        // source; it just cannot be exercised through a fake adapter.
        EmptyImplementationName,
        AdvertisesPolicyButReturnsNone,
        VersioningRejectsValidVersion,
        VersioningCompareReversed,
        BuildPlanIsEmpty,
        PublicationUsesDistributionKind,
        ReleaseMetadataNoTags,
    }

    pub struct BrokenStubAdapter {
        pub brokenness: Brokenness,
    }

    impl BrokenStubAdapter {
        pub fn new(brokenness: Brokenness) -> Self {
            Self { brokenness }
        }
    }

    fn base_descriptor() -> AdapterDescriptor {
        let mut caps = AdapterCapabilities::new();
        for c in [
            AdapterCapability::VersionComparison,
            AdapterCapability::PolicyDerivation,
            AdapterCapability::BuildPlanning,
            AdapterCapability::PackageQaPlanning,
            AdapterCapability::IssueRead,
            AdapterCapability::IssueWrite,
            AdapterCapability::ReleaseMetadata,
            AdapterCapability::PublicationPlanning,
        ] {
            caps.insert(c);
        }
        AdapterDescriptor {
            family: DistributionFamily::new("local").unwrap(),
            implementation_name: "broken-stub".into(),
            implementation_version: "0.1.0".into(),
            capabilities: caps,
        }
    }

    fn brokenness_descriptor(b: Brokenness) -> AdapterDescriptor {
        let mut d = base_descriptor();
        if b == Brokenness::EmptyImplementationName {
            d.implementation_name = String::new();
        }
        d
    }

    pub struct PassVersioning;
    impl VersioningCapability for PassVersioning {
        fn validate(&self, _: &PackageVersion) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Ok(())
        }
        fn compare(
            &self,
            a: &PackageVersion,
            b: &PackageVersion,
        ) -> Result<std::cmp::Ordering, ironmaint_adapter_api::AdapterError> {
            Ok(a.as_str().cmp(b.as_str()))
        }
    }

    pub struct RejectVersioning;
    impl VersioningCapability for RejectVersioning {
        fn validate(&self, _: &PackageVersion) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Err(ironmaint_adapter_api::AdapterError::new(
                ironmaint_adapter_api::AdapterErrorKind::InvalidVersion,
                "rejecting all versions",
            ))
        }
        fn compare(
            &self,
            a: &PackageVersion,
            b: &PackageVersion,
        ) -> Result<std::cmp::Ordering, ironmaint_adapter_api::AdapterError> {
            Ok(a.as_str().cmp(b.as_str()))
        }
    }

    pub struct ReversedVersioning;
    impl VersioningCapability for ReversedVersioning {
        fn validate(&self, _: &PackageVersion) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Ok(())
        }
        fn compare(
            &self,
            a: &PackageVersion,
            b: &PackageVersion,
        ) -> Result<std::cmp::Ordering, ironmaint_adapter_api::AdapterError> {
            // Deliberately wrong: returns the reverse ordering.
            Ok(b.as_str().cmp(a.as_str()))
        }
    }

    pub struct PassPackageModel;
    impl PackageModelCapability for PassPackageModel {
        fn validate_name(
            &self,
            _: &PackageName,
        ) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Ok(())
        }
        fn classify_changes(
            &self,
            changes: &[ChangedPath],
        ) -> Result<Vec<ChangeDomain>, ironmaint_adapter_api::AdapterError> {
            Ok(changes
                .iter()
                .map(|_| ChangeDomain::PackagingMetadata)
                .collect())
        }
    }

    pub struct PassPolicy;
    impl PolicyCapability for PassPolicy {
        fn authority_order(&self) -> Vec<AuthorityClassification> {
            vec![AuthorityClassification::BestPractice]
        }
        fn derive_obligation_plan(
            &self,
            _: &ironmaint_adapter_api::PolicyContext,
        ) -> Result<PolicyPlan, ironmaint_adapter_api::AdapterError> {
            Ok(PolicyPlan {
                baseline: PolicyBaseline::new(DistributionRef::new(
                    DistributionFamily::new("local").unwrap(),
                    DistributionRelease::new("local-release").unwrap(),
                )),
                obligation_templates: vec![],
            })
        }
    }

    pub struct PassBuild;
    impl BuildCapability for PassBuild {
        fn build_plan(
            &self,
            _: &ironmaint_adapter_api::CandidateContext,
        ) -> Result<BuildPlan, ironmaint_adapter_api::AdapterError> {
            Ok(BuildPlan::new().with_check(PlannedCheck::new(
                ironmaint_adapter_api::ToolCapabilityKey::new("local.build.fake").unwrap(),
                EvidenceKind::Build,
                true,
            )))
        }
        fn qa_plan(
            &self,
            _: &ironmaint_adapter_api::CandidateContext,
        ) -> Result<QaPlan, ironmaint_adapter_api::AdapterError> {
            Ok(QaPlan::new().with_check(PlannedCheck::new(
                ironmaint_adapter_api::ToolCapabilityKey::new("local.qa.fake").unwrap(),
                EvidenceKind::PackageQa,
                true,
            )))
        }
    }

    pub struct EmptyBuild;
    impl BuildCapability for EmptyBuild {
        fn build_plan(
            &self,
            _: &ironmaint_adapter_api::CandidateContext,
        ) -> Result<BuildPlan, ironmaint_adapter_api::AdapterError> {
            Ok(BuildPlan::new())
        }
        fn qa_plan(
            &self,
            _: &ironmaint_adapter_api::CandidateContext,
        ) -> Result<QaPlan, ironmaint_adapter_api::AdapterError> {
            Ok(QaPlan::new())
        }
    }

    pub struct PassIssues;
    impl IssueCapability for PassIssues {
        fn provider_id(&self) -> IssueProviderId {
            IssueProviderId::new()
        }
        fn supported_actions(&self) -> Vec<IssueActionKind> {
            vec![IssueActionKind::Comment]
        }
        fn validate_action(
            &self,
            _: &ironmaint_policy::IssueAction,
        ) -> Result<(), ironmaint_adapter_api::AdapterError> {
            Ok(())
        }
    }

    pub struct PassRelease;
    impl ReleaseCapability for PassRelease {
        fn release_metadata_plan(
            &self,
            _: &ironmaint_adapter_api::ReleaseContext,
        ) -> Result<ironmaint_adapter_api::ReleaseMetadataPlan, ironmaint_adapter_api::AdapterError>
        {
            Ok(ironmaint_adapter_api::ReleaseMetadataPlan::new().with_tag("local/release"))
        }
        fn publication_plan(
            &self,
            _: &ironmaint_adapter_api::PublicationContext,
        ) -> Result<PublicationPlanTemplate, ironmaint_adapter_api::AdapterError> {
            Ok(PublicationPlanTemplate::new(
                DistributionRef::new(
                    DistributionFamily::new("local").unwrap(),
                    DistributionRelease::new("local-release").unwrap(),
                ),
                "publish",
            )
            .with_operation(ironmaint_adapter_api::PlannedOperation::new(
                PrivilegedOperationKind::CanonicalRepositoryPush,
                "push",
            )))
        }
    }

    pub struct NoTagsRelease;
    impl ReleaseCapability for NoTagsRelease {
        fn release_metadata_plan(
            &self,
            _: &ironmaint_adapter_api::ReleaseContext,
        ) -> Result<ironmaint_adapter_api::ReleaseMetadataPlan, ironmaint_adapter_api::AdapterError>
        {
            Ok(ironmaint_adapter_api::ReleaseMetadataPlan::new())
        }
        fn publication_plan(
            &self,
            _: &ironmaint_adapter_api::PublicationContext,
        ) -> Result<PublicationPlanTemplate, ironmaint_adapter_api::AdapterError> {
            Ok(PublicationPlanTemplate::new(
                DistributionRef::new(
                    DistributionFamily::new("local").unwrap(),
                    DistributionRelease::new("local-release").unwrap(),
                ),
                "publish",
            )
            .with_operation(ironmaint_adapter_api::PlannedOperation::new(
                PrivilegedOperationKind::CanonicalRepositoryPush,
                "push",
            )))
        }
    }

    pub struct DistributionSpecificRelease;
    impl ReleaseCapability for DistributionSpecificRelease {
        fn release_metadata_plan(
            &self,
            _: &ironmaint_adapter_api::ReleaseContext,
        ) -> Result<ironmaint_adapter_api::ReleaseMetadataPlan, ironmaint_adapter_api::AdapterError>
        {
            Ok(ironmaint_adapter_api::ReleaseMetadataPlan::new().with_tag("local/release"))
        }
        fn publication_plan(
            &self,
            _: &ironmaint_adapter_api::PublicationContext,
        ) -> Result<PublicationPlanTemplate, ironmaint_adapter_api::AdapterError> {
            // Deliberately use the escape hatch to break §68 item 9.
            Ok(PublicationPlanTemplate::new(
                DistributionRef::new(
                    DistributionFamily::new("local").unwrap(),
                    DistributionRelease::new("local-release").unwrap(),
                ),
                "publish",
            )
            .with_operation(ironmaint_adapter_api::PlannedOperation::new(
                PrivilegedOperationKind::Other("custom.distribution.op".to_string()),
                "escape hatch",
            )))
        }
    }

    impl DistributionAdapter for BrokenStubAdapter {
        fn descriptor(&self) -> AdapterDescriptor {
            brokenness_descriptor(self.brokenness)
        }
        fn versioning(&self) -> &dyn VersioningCapability {
            match self.brokenness {
                Brokenness::VersioningRejectsValidVersion => {
                    static V: RejectVersioning = RejectVersioning;
                    &V
                }
                Brokenness::VersioningCompareReversed => {
                    static V: ReversedVersioning = ReversedVersioning;
                    &V
                }
                _ => {
                    static V: PassVersioning = PassVersioning;
                    &V
                }
            }
        }
        fn package_model(&self) -> &dyn PackageModelCapability {
            static P: PassPackageModel = PassPackageModel;
            &P
        }
        fn policy(&self) -> Option<&dyn PolicyCapability> {
            match self.brokenness {
                Brokenness::AdvertisesPolicyButReturnsNone => None,
                _ => {
                    static P: PassPolicy = PassPolicy;
                    Some(&P)
                }
            }
        }
        fn build(&self) -> Option<&dyn BuildCapability> {
            match self.brokenness {
                Brokenness::BuildPlanIsEmpty => {
                    static B: EmptyBuild = EmptyBuild;
                    Some(&B)
                }
                _ => {
                    static B: PassBuild = PassBuild;
                    Some(&B)
                }
            }
        }
        fn issues(&self) -> Option<&dyn IssueCapability> {
            static I: PassIssues = PassIssues;
            Some(&I)
        }
        fn release(&self) -> Option<&dyn ReleaseCapability> {
            match self.brokenness {
                Brokenness::PublicationUsesDistributionKind => {
                    static R: DistributionSpecificRelease = DistributionSpecificRelease;
                    Some(&R)
                }
                Brokenness::ReleaseMetadataNoTags => {
                    static R: NoTagsRelease = NoTagsRelease;
                    Some(&R)
                }
                _ => {
                    static R: PassRelease = PassRelease;
                    Some(&R)
                }
            }
        }
    }
}

#[test]
#[should_panic(expected = "descriptor implementation_name is empty")]
fn runner_panics_on_empty_implementation_name() {
    assert_distribution_adapter_conformance(&broken::BrokenStubAdapter::new(
        broken::Brokenness::EmptyImplementationName,
    ));
}

#[test]
#[should_panic(expected = "advertised PolicyDerivation")]
fn runner_panics_when_advertised_policy_returns_none() {
    assert_distribution_adapter_conformance(&broken::BrokenStubAdapter::new(
        broken::Brokenness::AdvertisesPolicyButReturnsNone,
    ));
}

#[test]
#[should_panic(expected = "versioning.validate")]
fn runner_panics_when_versioning_rejects_valid_version() {
    assert_distribution_adapter_conformance(&broken::BrokenStubAdapter::new(
        broken::Brokenness::VersioningRejectsValidVersion,
    ));
}

#[test]
#[should_panic(expected = "must be Greater")]
fn runner_panics_when_versioning_compare_is_reversed() {
    assert_distribution_adapter_conformance(&broken::BrokenStubAdapter::new(
        broken::Brokenness::VersioningCompareReversed,
    ));
}

#[test]
#[should_panic(expected = "build.build_plan produced no PlannedCheck")]
fn runner_panics_when_build_plan_is_empty() {
    assert_distribution_adapter_conformance(&broken::BrokenStubAdapter::new(
        broken::Brokenness::BuildPlanIsEmpty,
    ));
}

#[test]
#[should_panic(expected = "distribution-specific PrivilegedOperationKind::Other")]
fn runner_panics_when_publication_uses_distribution_specific_kind() {
    assert_distribution_adapter_conformance(&broken::BrokenStubAdapter::new(
        broken::Brokenness::PublicationUsesDistributionKind,
    ));
}

#[test]
#[should_panic(expected = "release.release_metadata_plan produced no tags")]
fn runner_panics_when_release_metadata_has_no_tags() {
    assert_distribution_adapter_conformance(&broken::BrokenStubAdapter::new(
        broken::Brokenness::ReleaseMetadataNoTags,
    ));
}
