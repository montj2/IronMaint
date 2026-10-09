//! 1B.1 RED — `SourceInspection` advertised ⇒ `inspection()` `Some`.
//!
//! This test is the teeth of PR 1B.1's broken-then-fixed pattern.
//! It walks every `DistributionAdapter` impl registered with the
//! shared conformance harness and asserts the contract that
//! `assert_descriptor_is_valid` enforces (`SourceInspection`
//! advertised in the descriptor must be backed by an
//! `inspection()` returning `Some`).
//!
//! RED (the state of the test at the RED commit of 1B.1):
//!   the Fedora stub advertises `SourceInspection` but does not
//!   yet implement it, so this test fails on the Fedora stub.
//! GREEN (after the GREEN commit of 1B.1 lands the
//! `FedoraInspection` impl): both stubs pass.
//!
//! The test runs both stubs end-to-end and surfaces which one
//! failed, so the broken-then-fixed transition is visible in the
//! test output.

use ironmaint_adapter_api::{AdapterCapability, DistributionAdapter};

#[test]
fn fedora_stub_advertised_source_inspection_is_backed_by_inspection() {
    use ironmaint_adapter_api::AdapterDescriptor;
    let fedora = fedora_stub::FedoraStubAdapter::new();
    let descriptor: AdapterDescriptor = fedora.descriptor();
    assert!(
        descriptor.capabilities.contains(&AdapterCapability::SourceInspection),
        "precondition: the Fedora stub advertises SourceInspection"
    );
    assert!(
        fedora.inspection().is_some(),
        "Fedora stub advertises SourceInspection in descriptor.capabilities \
         but adapter.inspection() returned None; the conformance arm in \
         ironmaint_testkit::conformance::assert_descriptor_is_valid is the \
         same assertion and will fail for the same reason — see 1B.1 RED."
    );
}

#[test]
fn debian_stub_does_not_advertise_source_inspection() {
    use ironmaint_adapter_api::AdapterDescriptor;
    let debian = debian_stub::DebianStubAdapter::new();
    let descriptor: AdapterDescriptor = debian.descriptor();
    assert!(
        !descriptor.capabilities.contains(&AdapterCapability::SourceInspection),
        "precondition: the Debian stub does NOT advertise SourceInspection \
         (descriptor_families_differ::source_inspection_only_fedora_advertises_it \
         pins this asymmetry; 1B.1 leaves it intact)"
    );
}
