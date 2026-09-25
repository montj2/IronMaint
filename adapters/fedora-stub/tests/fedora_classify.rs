//! Fedora `package_model` conformance (§47).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fedora_stub::FedoraStubAdapter;
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
fn spec_file_classifies_as_packaging_metadata() {
    let a = FedoraStubAdapter::new();
    let inputs = paths(&[("foo.spec", PathRole::PackagingMetadata)]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert_eq!(out, vec![ChangeDomain::PackagingMetadata]);
}

#[test]
fn macro_marker_classifies_as_build_configuration() {
    let a = FedoraStubAdapter::new();
    let inputs = paths(&[("%build", PathRole::PackagingBuildConfig)]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert_eq!(out, vec![ChangeDomain::BuildConfiguration]);
}

#[test]
fn tests_directory_classifies_as_tests() {
    let a = FedoraStubAdapter::new();
    let inputs = paths(&[("tests/run.sh", PathRole::PackagingTests)]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert_eq!(out, vec![ChangeDomain::Tests]);
}

#[test]
fn upstream_source_classifies_as_upstream() {
    let a = FedoraStubAdapter::new();
    let inputs = paths(&[("src/main.c", PathRole::UpstreamSource)]);
    let out = a.package_model().classify_changes(&inputs).unwrap();
    assert_eq!(out, vec![ChangeDomain::UpstreamSource]);
}

#[test]
fn package_name_validation_accepts_legal_names() {
    let a = FedoraStubAdapter::new();
    for n in ["foo", "libfoo1", "python3-foo", "foo_bar"] {
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
    let a = FedoraStubAdapter::new();
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
    let a = FedoraStubAdapter::new();
    let err = a
        .package_model()
        .validate_name(&PackageName::new("a").unwrap())
        .unwrap_err();
    assert_eq!(
        err.kind,
        ironmaint_adapter_api::AdapterErrorKind::InvalidPackage
    );
}
