//! §51 acceptance test: Debian and Fedora stubs advertise different
//! sets of supported [`IssueActionKind`] values.
//!
//! The cross-stub difference is the §80 "no conditional logic on
//! `debian` vs `fedora` may appear inside core crates" criterion at
//! the adapter level — core never sees these kinds, only opaque
//! strings.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use debian_stub::DebianStubAdapter;
use fedora_stub::FedoraStubAdapter;
use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_policy::IssueActionKind;

#[test]
fn provider_ids_are_distinct() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    assert_ne!(
        debian.issues().unwrap().provider_id(),
        fedora.issues().unwrap().provider_id(),
        "§51: provider ids must be distinct per adapter"
    );
}

#[test]
fn supported_action_sets_differ() {
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    let d: Vec<IssueActionKind> = debian.issues().unwrap().supported_actions();
    let f: Vec<IssueActionKind> = fedora.issues().unwrap().supported_actions();

    // Debian-specific: MarkPending / Forward (BTS-specific).
    assert!(d.contains(&IssueActionKind::MarkPending));
    assert!(d.contains(&IssueActionKind::Forward));
    assert!(!f.contains(&IssueActionKind::MarkPending));
    assert!(!f.contains(&IssueActionKind::Forward));

    // Fedora-specific: AddLabels / RemoveLabels / Reopen
    // (Bugzilla-specific).
    assert!(f.contains(&IssueActionKind::AddLabels));
    assert!(f.contains(&IssueActionKind::RemoveLabels));
    assert!(f.contains(&IssueActionKind::Reopen));
    assert!(!d.contains(&IssueActionKind::AddLabels));
    assert!(!d.contains(&IssueActionKind::RemoveLabels));
    assert!(!d.contains(&IssueActionKind::Reopen));
}

#[test]
fn common_actions_are_common() {
    // Comment + SetSeverity appear in both adapters' supported sets.
    let debian = DebianStubAdapter::new();
    let fedora = FedoraStubAdapter::new();
    let d = debian.issues().unwrap().supported_actions();
    let f = fedora.issues().unwrap().supported_actions();
    assert!(d.contains(&IssueActionKind::Comment));
    assert!(d.contains(&IssueActionKind::SetSeverity));
    assert!(f.contains(&IssueActionKind::Comment));
    assert!(f.contains(&IssueActionKind::SetSeverity));
}
