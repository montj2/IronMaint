//! Sanity test: the two stub descriptors carry distinct families
//! and visibly different capability sets.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use debian_stub::DebianStubAdapter;
use fedora_stub::FedoraStubAdapter;
use ironmaint_adapter_api::{AdapterCapability, DistributionAdapter};

#[test]
fn families_are_distinct() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    assert_eq!(debian.descriptor().family.as_str(), "debian");
    assert_eq!(fedora.descriptor().family.as_str(), "fedora");
    assert_ne!(debian.descriptor().family, fedora.descriptor().family);
}

#[test]
fn source_inspection_only_fedora_advertises_it() {
    // §80 visible-difference: Fedora advertises SourceInspection;
    // Debian does not. The cross-stub test reads the capability set
    // (not a static switch on family name) to discover the difference.
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    assert!(
        !debian
            .descriptor()
            .capabilities
            .contains(&AdapterCapability::SourceInspection)
    );
    assert!(
        fedora
            .descriptor()
            .capabilities
            .contains(&AdapterCapability::SourceInspection)
    );
}

#[test]
fn implementation_names_differ() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    assert_eq!(debian.descriptor().implementation_name, "debian-stub");
    assert_eq!(fedora.descriptor().implementation_name, "fedora-stub");
}
