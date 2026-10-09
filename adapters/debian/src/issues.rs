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
        // 1B.3 GREEN: the production adapter advertises `IssueRead`
        // and not `IssueWrite`. The conformance arm
        // `assert_issues_conforms` requires `supported_actions`
        // to be non-empty. `Comment` is the read-side observable
        // kind: the BTS snapshot a future PR (1E.x) produces
        // contains comments. The production adapter does not
        // *execute* a `Comment` action today (it only reads),
        // but it can *describe* the action kind.
        vec![IssueActionKind::Comment]
    }

    fn validate_action(&self, action: &IssueAction) -> Result<(), AdapterError> {
        // 1B.3 GREEN: `Comment` is the single read-side kind the
        // adapter supports. Anything else is rejected. The
        // production adapter does not execute actions; this
        // method is the contract for what the BTS *could* do
        // if the runtime asked, and the answer today is
        // "Comment is a valid kind, everything else is not."
        if action.kind == IssueActionKind::Comment {
            Ok(())
        } else {
            Err(AdapterError {
                kind: AdapterErrorKind::Unsupported,
                message: format!(
                    "debian production adapter: action kind {:?} is not in the \
                     1B.3 read-side set (Comment only)",
                    action.kind
                ),
            })
        }
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
    fn supported_actions_is_a_single_read_side_kind() {
        // 1B.3 GREEN: `Comment` is the single read-side kind the
        // adapter supports. Anything else (Close, Reopen,
        // SetSeverity, ...) is rejected by `validate_action`.
        let i = DebianIssues::new();
        let kinds = i.supported_actions();
        assert_eq!(kinds, vec![IssueActionKind::Comment]);
    }

    #[test]
    fn validate_action_accepts_comment_and_rejects_everything_else() {
        // 1B.3 GREEN: `Comment` is accepted; `MarkPending` and
        // `Forward` (the kinds the stub's `DebianIssues` accepts)
        // are rejected. The production adapter does not execute
        // any action; `validate_action` is the contract for
        // what the BTS *could* do.
        let i = DebianIssues::new();
        i.validate_action(&action(IssueActionKind::Comment))
            .expect("Comment is the single read-side kind");

        let err = i
            .validate_action(&action(IssueActionKind::MarkPending))
            .unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::Unsupported);

        let err = i
            .validate_action(&action(IssueActionKind::Forward))
            .unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::Unsupported);
    }
}
