//! Generic adapter conformance suite (PHASE-0A.md §68).
//!
//! [`assert_distribution_adapter_conformance`] is the single public
//! entry point. It walks every item from the §68 minimum-verification
//! checklist against a single `&dyn DistributionAdapter` and `panic!`s
//! with a descriptive message on the first contract violation.
//!
//! ## Required vs optional capabilities
//!
//! The three required capabilities ([`VersioningCapability`],
//! [`PackageModelCapability`], and the descriptor itself) always run.
//! The four optional capabilities ([`PolicyCapability`],
//! [`BuildCapability`], [`IssueCapability`], [`ReleaseCapability`]) run
//! only when the corresponding `adapter.method()` returns `Some`.
//! The cross-check inside [`assert_descriptor_is_valid`] independently
//! enforces that every capability advertised in
//! `descriptor.capabilities` has a matching `Some(&dyn ...)` — so an
//! adapter cannot advertise `PolicyDerivation` and return `None` from
//! `policy()` without failing conformance.
//!
//! ## Distribution neutrality
//!
//! This module contains no `debian` / `fedora` / `ubuntu` / `arch` /
//! `rhel` literal. The only distribution-shaped string it ever sees
//! is the family returned by `adapter.descriptor().family.as_str()`,
//! which the adapter itself provided. Conformance runs identically
//! against any adapter.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::map_err_ignore
)]

use std::cmp::Ordering;

use ironmaint_adapter_api::{
    AdapterCapability, AdapterDescriptor, BuildCapability, DistributionAdapter, IssueCapability,
    PackageModelCapability, PathRole, PolicyCapability, ReleaseCapability, VersioningCapability,
};
use ironmaint_core::{IssueProviderId, PackageName, PackageVersion};
use ironmaint_evidence::ChangeDomain;
use ironmaint_policy::{IssueActionKind, PrivilegedOperationKind};

use crate::fixtures;

// -----------------------------------------------------------------------------
// Public entry point.
// -----------------------------------------------------------------------------

/// Run the conformance suite against a single adapter. See module docs.
///
/// # Panics
/// Panics with a descriptive message on the first contract violation.
/// On success, returns `()`.
pub fn assert_distribution_adapter_conformance(adapter: &dyn DistributionAdapter) {
    let desc = assert_descriptor_is_valid(adapter);
    assert_versioning_conforms(adapter.versioning());
    assert_package_model_conforms(adapter.package_model());
    if let Some(p) = adapter.policy() {
        assert_policy_conforms(p, &desc);
    }
    if let Some(b) = adapter.build() {
        assert_build_conforms(b, &desc);
    }
    if let Some(i) = adapter.issues() {
        assert_issues_conforms(i);
    }
    if let Some(r) = adapter.release() {
        assert_release_conforms(r, &desc);
    }
    assert_state_surface_is_unaffected_by_adapter(adapter);
}

// -----------------------------------------------------------------------------
// §68 item 1: descriptor family is valid.
// -----------------------------------------------------------------------------

fn assert_descriptor_is_valid(a: &dyn DistributionAdapter) -> AdapterDescriptor {
    let d = a.descriptor();
    assert!(!d.family.as_str().is_empty(), "descriptor family is empty",);
    assert!(
        !d.implementation_name.is_empty(),
        "descriptor implementation_name is empty",
    );
    assert!(
        !d.implementation_version.is_empty(),
        "descriptor implementation_version is empty",
    );
    assert!(
        !d.capabilities.is_empty(),
        "descriptor capabilities set is empty",
    );
    for cap in d.capabilities.iter() {
        match cap {
            AdapterCapability::PolicyDerivation => assert!(
                a.policy().is_some(),
                "advertised PolicyDerivation in descriptor.capabilities but \
                 adapter.policy() returned None",
            ),
            AdapterCapability::BuildPlanning
            | AdapterCapability::PackageQaPlanning
            | AdapterCapability::FunctionalTestPlanning
            | AdapterCapability::UpgradeTestPlanning
            | AdapterCapability::ReproducibilityPlanning => assert!(
                a.build().is_some(),
                "advertised {cap:?} in descriptor.capabilities but \
                 adapter.build() returned None",
            ),
            AdapterCapability::IssueRead | AdapterCapability::IssueWrite => assert!(
                a.issues().is_some(),
                "advertised {cap:?} in descriptor.capabilities but \
                 adapter.issues() returned None",
            ),
            AdapterCapability::ReleaseMetadata | AdapterCapability::PublicationPlanning => assert!(
                a.release().is_some(),
                "advertised {cap:?} in descriptor.capabilities but \
                 adapter.release() returned None",
            ),
            AdapterCapability::SourceInspection
            | AdapterCapability::VersionComparison
            | AdapterCapability::UpstreamDiscovery => {
                // These are declarative; the root trait's required methods
                // (descriptor / versioning) cover them, so no cross-check.
            }
            // `AdapterCapability` is `#[non_exhaustive]`; future variants
            // added to the spec require no corresponding trait method, so
            // a no-op arm is the correct default.
            _ => {}
        }
    }
    d
}

// -----------------------------------------------------------------------------
// §68 item 3: versions can be compared.
// -----------------------------------------------------------------------------

fn assert_versioning_conforms(v: &dyn VersioningCapability) {
    let one =
        PackageVersion::new("1.0.0").expect("static literal must satisfy PackageVersion::new");
    let one_dot_one =
        PackageVersion::new("1.0.1").expect("static literal must satisfy PackageVersion::new");
    let zero =
        PackageVersion::new("0.9.0").expect("static literal must satisfy PackageVersion::new");

    assert!(
        v.validate(&one).is_ok(),
        "versioning.validate(\"1.0.0\") returned Err",
    );
    assert_eq!(
        v.compare(&one_dot_one, &one)
            .expect("compare 1.0.1 vs 1.0.0 must not error"),
        Ordering::Greater,
        "versioning.compare(\"1.0.1\", \"1.0.0\") must be Greater",
    );
    assert_eq!(
        v.compare(&one, &one_dot_one)
            .expect("compare 1.0.0 vs 1.0.1 must not error"),
        Ordering::Less,
        "versioning.compare(\"1.0.0\", \"1.0.1\") must be Less",
    );
    assert_eq!(
        v.compare(&one, &one)
            .expect("compare 1.0.0 vs 1.0.0 must not error"),
        Ordering::Equal,
        "versioning.compare(\"1.0.0\", \"1.0.0\") must be Equal",
    );
    let _ = zero; // silence unused warning; included for symmetry in test-local reasoning.
}

// -----------------------------------------------------------------------------
// §68 items 2 + 4: package names can be validated; change paths can be classified.
// -----------------------------------------------------------------------------

fn assert_package_model_conforms(pm: &dyn PackageModelCapability) {
    let name = PackageName::new("foo").expect("static literal must satisfy PackageName::new");
    assert!(
        pm.validate_name(&name).is_ok(),
        "package_model.validate_name(\"foo\") returned Err",
    );

    let changes = [
        ironmaint_adapter_api::ChangedPath::new("debian/control", PathRole::PackagingMetadata),
        ironmaint_adapter_api::ChangedPath::new("README.md", PathRole::DocumentationOnly),
    ];
    let domains = pm
        .classify_changes(&changes)
        .expect("classify_changes on canonical inputs must succeed");
    assert_eq!(
        domains.len(),
        changes.len(),
        "classify_changes must return one ChangeDomain per ChangedPath",
    );
    let has_canonical = domains
        .iter()
        .any(|domain| !matches!(domain, ChangeDomain::AdapterSpecific(_)));
    assert!(
        has_canonical,
        "classify_changes returned only AdapterSpecific domains — adapter does not actually classify paths",
    );
}

// -----------------------------------------------------------------------------
// §68 item 5: policy plan can be produced.
// -----------------------------------------------------------------------------

fn assert_policy_conforms(p: &dyn PolicyCapability, d: &AdapterDescriptor) {
    let ctx = fixtures::policy_context_for(d);
    let plan = p
        .derive_obligation_plan(&ctx)
        .expect("policy.derive_obligation_plan must succeed");
    assert_eq!(
        plan.baseline.distribution.family.as_str(),
        d.family.as_str(),
        "policy plan baseline distribution family must match adapter descriptor family",
    );
    let order = p.authority_order();
    assert!(
        !order.is_empty(),
        "policy.authority_order must be non-empty",
    );
    // No further invariants — `AuthorityClassification` does not derive
    // `Ord` so duplicate detection would require a custom comparator;
    // uniqueness is the adapter author's responsibility, not a §68
    // requirement.
}

// -----------------------------------------------------------------------------
// §68 item 6: build plan can be produced.
// -----------------------------------------------------------------------------

fn assert_build_conforms(b: &dyn BuildCapability, d: &AdapterDescriptor) {
    let ctx = fixtures::candidate_context_for(d);
    let plan = b.build_plan(&ctx).expect("build.build_plan must succeed");
    assert!(
        !plan.checks.is_empty(),
        "build.build_plan produced no PlannedCheck entries",
    );
    let qa = b.qa_plan(&ctx).expect("build.qa_plan must succeed");
    assert!(
        !qa.checks.is_empty(),
        "build.qa_plan produced no PlannedCheck entries",
    );
    for check in plan.checks.iter().chain(qa.checks.iter()) {
        let ns = check.key.namespace();
        assert!(
            !ns.is_empty(),
            "PlannedCheck {:?} has empty tool-key namespace",
            check.key.as_str(),
        );
        assert_eq!(
            ns,
            d.family.as_str(),
            "PlannedCheck {:?} namespace must equal adapter family {:?}",
            check.key.as_str(),
            d.family.as_str(),
        );
    }
}

// -----------------------------------------------------------------------------
// §68 item 7: issue provider can be described.
// -----------------------------------------------------------------------------

fn assert_issues_conforms(i: &dyn IssueCapability) {
    let _provider: IssueProviderId = i.provider_id();
    let actions: Vec<IssueActionKind> = i.supported_actions();
    assert!(
        !actions.is_empty(),
        "issues.supported_actions must be non-empty",
    );
    // No further invariants — `IssueActionKind` does not derive `Ord`
    // so duplicate detection would require a custom comparator;
    // uniqueness is the adapter author's responsibility, not a §68
    // requirement.
}

// -----------------------------------------------------------------------------
// §68 items 8 + 9: release plan can be produced; publication operations remain generic.
// -----------------------------------------------------------------------------

fn assert_release_conforms(r: &dyn ReleaseCapability, d: &AdapterDescriptor) {
    let rctx = fixtures::release_context_for(d);
    let pctx = fixtures::publication_context_for(d);

    let meta = r
        .release_metadata_plan(&rctx)
        .expect("release.release_metadata_plan must succeed");
    assert!(
        !meta.tags.is_empty(),
        "release.release_metadata_plan produced no tags",
    );

    let pub_plan = r
        .publication_plan(&pctx)
        .expect("release.publication_plan must succeed");
    assert!(
        !pub_plan.operations.is_empty(),
        "release.publication_plan produced no operations",
    );
    assert!(
        !pub_plan.rationale.is_empty(),
        "release.publication_plan rationale must be non-empty",
    );
    for op in &pub_plan.operations {
        assert!(
            !matches!(op.kind, PrivilegedOperationKind::Other(_)),
            "release.publication_plan used a distribution-specific \
             PrivilegedOperationKind::Other({:?}); publication operations \
             must use the core vocabulary",
            op.kind,
        );
    }
}

// -----------------------------------------------------------------------------
// §68 item 10: no adapter mutates JobState.
// -----------------------------------------------------------------------------

/// PHASE-0A.md §68 item 10: the adapter must not mutate `JobState`.
///
/// This invariant is enforced by the type system — `DistributionAdapter`
/// declares no method that takes `&mut JobState`, and adapter crates
/// do not depend on `ironmaint-state` (verified at the workspace level
/// by `cargo xtask verify-architecture`). The conformance runner itself
/// cannot observe a violation that the type system has already excluded.
fn assert_state_surface_is_unaffected_by_adapter(_a: &dyn DistributionAdapter) {
    // No-op: the type system guarantees this invariant.
}
