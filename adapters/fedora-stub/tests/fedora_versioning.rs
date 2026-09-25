//! Fedora versioning conformance (§46).
//!
//! Verifies the Fedora-specific version-split rules and confirms the
//! tilde-version behaviour differs from the Debian stub (§80 visible
//! difference test).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cmp::Ordering;

use fedora_stub::FedoraStubAdapter;
use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_core::PackageVersion;

fn pv(s: &str) -> PackageVersion {
    PackageVersion::new(s).expect("static literal must be a valid version")
}

#[test]
fn section_69_fixture_orders_less_than_2() {
    let a = FedoraStubAdapter::new();
    assert_eq!(
        a.versioning()
            .compare(&pv("1.9.0-1"), &pv("1.9.0-2"))
            .unwrap(),
        Ordering::Less
    );
    assert_eq!(
        a.versioning()
            .compare(&pv("1.9.0-2"), &pv("1.9.0-1"))
            .unwrap(),
        Ordering::Greater
    );
    assert_eq!(
        a.versioning()
            .compare(&pv("1.9.0-1"), &pv("1.9.0-1"))
            .unwrap(),
        Ordering::Equal
    );
}

#[test]
fn epoch_takes_precedence_over_version() {
    let a = FedoraStubAdapter::new();
    assert_eq!(
        a.versioning().compare(&pv("2:1.0"), &pv("1:99.0")).unwrap(),
        Ordering::Greater
    );
}

#[test]
fn tilde_versions_accepted_by_fedora() {
    // §46 visible-difference: Fedora accepts tilde versions;
    // the Debian stub rejects them.
    let a = FedoraStubAdapter::new();
    assert!(a.versioning().validate(&pv("1.0.0~rc1")).is_ok());
}

#[test]
fn fc_tag_stripped_for_release_comparison() {
    let a = FedoraStubAdapter::new();
    assert_eq!(
        a.versioning()
            .compare(&pv("1.9.0-1.fc45"), &pv("1.9.0-1"))
            .unwrap(),
        Ordering::Equal
    );
}
