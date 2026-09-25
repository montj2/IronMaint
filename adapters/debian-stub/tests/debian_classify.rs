//! Debian `package_model` conformance (§47).
//!
//! Verifies the Debian-specific path-classification rules: `debian/*`
//! paths map to packaging metadata / build configuration / tests /
//! patch-set buckets, and Debian package-name grammar is enforced.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use debian_stub::DebianStubAdapter;
use ironmaint_adapter_api::{ChangedPath, DistributionAdapter, PathRole};
use ironmaint_core::PackageName;
use ironmaint_evidence::ChangeDomain;

fn paths(rs: &[(&str, PathRole)]) -> Vec<ChangedPath> {
    rs.iter()
        .map(|(p, r)| ChangedPath {
            path: (*p).to_string(),
            role: r.clone(),
        })
        .collect()
}

#[test]
fn debian_control_classifies_as_packaging_metadata() {
    let a = DebianStubAdapter::new();
    let inputs = paths(&[("debian/control", PathRole::PackagingMetadata)]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert_eq!(out, vec![ChangeDomain::PackagingMetadata]);
}

#[test]
fn debian_rules_classifies_as_build_configuration() {
    let a = DebianStubAdapter::new();
    let inputs = paths(&[("debian/rules", PathRole::PackagingBuildConfig)]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert_eq!(out, vec![ChangeDomain::BuildConfiguration]);
}

#[test]
fn debian_tests_directory_classifies_as_tests() {
    let a = DebianStubAdapter::new();
    let inputs = paths(&[
        ("debian/tests/control", PathRole::PackagingTests),
        ("debian/tests/test-foo", PathRole::PackagingTests),
    ]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert!(out.iter().all(|d| *d == ChangeDomain::Tests));
}

#[test]
fn debian_patches_classifies_as_patch_set() {
    let a = DebianStubAdapter::new();
    let inputs = paths(&[("debian/patches/01-fix.patch", PathRole::PackagingPatch)]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert_eq!(out, vec![ChangeDomain::PatchSet]);
}

#[test]
fn package_name_validation_accepts_legal_names() {
    let a = DebianStubAdapter::new();
    for n in ["foo", "libfoo1", "libfoo-dev", "libfoo+meta1"] {
        assert!(
            a.package_model()
                .validate_name(&PackageName::new(n).unwrap())
                .is_ok(),
            "expected {n} to validate"
        );
    }
}

#[test]
fn package_name_validation_rejects_uppercase() {
    let a = DebianStubAdapter::new();
    let err = a
        .package_model()
        .validate_name(&PackageName::new("Foo").unwrap())
        .unwrap_err();
    assert_eq!(
        err.kind,
        ironmaint_adapter_api::AdapterErrorKind::InvalidPackage
    );
}

#[test]
fn package_name_validation_rejects_too_short() {
    let a = DebianStubAdapter::new();
    let err = a
        .package_model()
        .validate_name(&PackageName::new("a").unwrap())
        .unwrap_err();
    assert_eq!(
        err.kind,
        ironmaint_adapter_api::AdapterErrorKind::InvalidPackage
    );
}
