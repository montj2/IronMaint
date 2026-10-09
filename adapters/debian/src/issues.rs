//! Production Debian issue capability (PHASE-1 §51).
//!
//! 1B.3 ships a `IssueRead`-only adapter. The provider id is a
//! stable singleton; the `supported_actions` set is empty in
//! 1B.3 RED (the production adapter does not execute any
//! actions in the read-only-intake phase) and the GREEN commit
//! adds a single read-side observable kind (`Comment`) to
//! satisfy the conformance arm
//! `crates/ironmaint-testkit/src/conformance.rs:401-404`.
//!
//! 1E.x is the real BTS transport and BTS snapshot
//! normalization; 1B.3 is just the trait shape so the
//! conformance arm at `assert_descriptor_is_valid` for
//! `AdapterCapability::IssueRead` passes.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterError, AdapterErrorKind, IssueCapability};
use ironmaint_core::IssueProviderId;
use ironmaint_policy::{IssueAction, IssueActionKind};

static PROVIDER_ID: LazyLock<IssueProviderId> = LazyLock::new(IssueProviderId::new);
static DEBIAN_ISSUES: LazyLock<DebianIssues> = LazyLock::new(DebianIssues::new);

#[derive(Debug, Default, Clone, Copy)]
pub struct DebianIssues;

impl DebianIssues {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Static accessor. Used by the `DistributionAdapter::issues`
/// implementation in `lib.rs`.
#[must_use]
pub fn debian_issues() -> &'static DebianIssues {
    &DEBIAN_ISSUES
}

impl IssueCapability for DebianIssues {
    fn provider_id(&self) -> IssueProviderId {
        // Stable singleton: every call returns the same id, so
        // observers that key on `provider_id` see the same value
        // across all candidate captures.
        *PROVIDER_ID
    }

    fn supported_actions(&self) -> Vec<IssueActionKind> {
        // 1B.3 RED: the production adapter advertises `IssueRead`
        // and not `IssueWrite`. The conformance arm
        // `assert_issues_conforms` requires `supported_actions`
        // to be non-empty; the GREEN commit adds a single
        // read-side observable kind (`Comment`) so the arm
        // passes without changing the conformance suite.
        Vec::new()
    }

    fn validate_action(&self, action: &IssueAction) -> Result<(), AdapterError> {
        // 1B.3 RED: every action is rejected because
        // `supported_actions` is empty. The GREEN commit accepts
        // `Comment` (the single read-side kind) and rejects
        // everything else.
        let _ = action;
        Err(AdapterError {
            kind: AdapterErrorKind::Unsupported,
            message: "debian production adapter: no actions supported in 1B.3 (read-only-intake)"
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::IssueRef;
    use ironmaint_policy::IssueAction;

    fn action(kind: IssueActionKind) -> IssueAction {
        IssueAction::new(
            IssueRef::new(IssueProviderId::new(), "123456".to_string()).unwrap(),
            None,
            kind,
            "r",
        )
        .unwrap()
    }

    #[test]
    fn provider_id_is_stable() {
        let i = DebianIssues::new();
        assert_eq!(i.provider_id(), i.provider_id());
    }

    #[test]
    fn supported_actions_is_empty_in_red() {
        // 1B.3 RED. The GREEN commit changes this to one read-side
        // kind (`Comment`).
        let i = DebianIssues::new();
        assert!(i.supported_actions().is_empty());
    }

    #[test]
    fn validate_action_rejects_everything_in_red() {
        // 1B.3 RED. The GREEN commit accepts `Comment`.
        let i = DebianIssues::new();
        let err = i
            .validate_action(&action(IssueActionKind::Comment))
            .unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::Unsupported);
    }
}
