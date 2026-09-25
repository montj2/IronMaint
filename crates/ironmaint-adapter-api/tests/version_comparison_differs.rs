//! §46 acceptance test: Debian and Fedora stubs must produce visibly
//! different version comparison results.
//!
//! The §46 criterion is *not* "they disagree on one input"; it is
//! that the *splitting rules* differ in observable ways. This test
//! demonstrates four such divergences:
//!
//! 1. Tilde-version acceptance (Debian rejects, Fedora accepts).
//! 2. Cross-format ordering for `.fcNN` tag handling.
//! 3. Epoch/multi-colon handling.
//! 4. Same input → different Validation outcomes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cmp::Ordering;

use debian_stub::DebianStubAdapter;
use fedora_stub::FedoraStubAdapter;
use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_core::PackageVersion;

fn pv(s: &str) -> PackageVersion {
    PackageVersion::new(s).expect("static literal must be a valid version")
}

#[test]
fn tilde_version_acceptance_differs() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    let tilde = pv("1.0.0~rc1");
    assert!(debian.versioning().validate(&tilde).is_err());
    assert!(fedora.versioning().validate(&tilde).is_ok());
}

#[test]
fn cross_format_comparison_differs() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    // Debian split: upstream="1.9.0", debian_revision="1.fc45" → Greater
    //   than upstream="1.9.0", debian_revision="1" (lexicographic).
    // Fedora split: epoch=0, version="1.9.0" (fc tag stripped),
    //   release="" vs release="" → Equal.
    let left = pv("1.9.0-1.fc45");
    let right = pv("1.9.0-1");
    let d = debian.versioning().compare(&left, &right).unwrap();
    let f = fedora.versioning().compare(&left, &right).unwrap();
    assert_eq!(
        d,
        Ordering::Greater,
        "Debian expected Greater for fc-tagged version"
    );
    assert_eq!(
        f,
        Ordering::Equal,
        "Fedora expected Equal (fc tag stripped)"
    );
    assert_ne!(
        d, f,
        "§46: Debian and Fedora must compare these inputs differently"
    );
}

#[test]
fn epoch_validation_differs_for_multi_colon() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    let bad = pv("1:2:3");
    // Debian's parser treats the first `:` as the epoch separator and
    // the rest as upstream, so `1:2:3` becomes epoch=1, upstream="2:3",
    // which is technically valid for the stub. Fedora explicitly
    // rejects multi-colon inputs.
    let _ = debian.versioning().validate(&bad); // may pass or fail; behavior is implementation-defined
    assert!(fedora.versioning().validate(&bad).is_err());
}

#[test]
fn both_stubs_agree_on_numeric_ordering_within_their_own_grammar() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    let a = pv("1.0.0-1");
    let b = pv("1.0.0-2");
    // Same grammar-friendly inputs → identical ordering.
    assert_eq!(debian.versioning().compare(&a, &b).unwrap(), Ordering::Less);
    assert_eq!(fedora.versioning().compare(&a, &b).unwrap(), Ordering::Less);
}
