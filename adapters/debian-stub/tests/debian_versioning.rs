//! Debian versioning conformance (§46).
//!
//! Verifies the canonical §69 fixture `1.9.0-1` vs `1.9.0-2` and the
//! Debian-specific version-split rules.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cmp::Ordering;

use debian_stub::DebianStubAdapter;
use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_core::PackageVersion;

fn pv(s: &str) -> PackageVersion {
    PackageVersion::new(s).expect("static literal must be a valid version")
}

#[test]
fn section_69_fixture_orders_less_than_2() {
    let a = DebianStubAdapter::new();
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
fn epoch_takes_precedence_over_upstream() {
    let a = DebianStubAdapter::new();
    assert_eq!(
        a.versioning().compare(&pv("2:1.0"), &pv("1:99.0")).unwrap(),
        Ordering::Greater
    );
}

#[test]
fn tilde_rejected_by_stub() {
    let a = DebianStubAdapter::new();
    let err = a.versioning().validate(&pv("1.0.0~rc1")).unwrap_err();
    assert_eq!(
        err.kind,
        ironmaint_adapter_api::AdapterErrorKind::InvalidVersion
    );
}
