//! Debian issue capability (§51).
//!
//! Provider identity is a stable, lazily-initialised
//! [`IssueProviderId`]. The Phase 0A.5 conformance suite reads this
//! through the trait to confirm two adapters return different ids.
//!
//! Validation: Debian BTS numbers are decimal, ≥1, no leading zeros
//! outside of "0" itself.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterError, AdapterErrorKind, IssueCapability};
use ironmaint_core::IssueProviderId;
use ironmaint_policy::{IssueAction, IssueActionKind};

static PROVIDER_ID: LazyLock<IssueProviderId> = LazyLock::new(IssueProviderId::new);
static SUPPORTED: LazyLock<Vec<IssueActionKind>> = LazyLock::new(|| {
    vec![
        IssueActionKind::MarkPending,
        IssueActionKind::Forward,
        IssueActionKind::SetSeverity,
        IssueActionKind::Comment,
        IssueActionKind::Link,
    ]
});

pub struct DebianIssues;

impl DebianIssues {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for DebianIssues {
    fn default() -> Self {
        Self::new()
    }
}

impl IssueCapability for DebianIssues {
    fn provider_id(&self) -> IssueProviderId {
        *PROVIDER_ID
    }

    fn supported_actions(&self) -> Vec<IssueActionKind> {
        SUPPORTED.clone()
    }

    fn validate_action(&self, action: &IssueAction) -> Result<(), AdapterError> {
        let supported = self.supported_actions();
        if !supported.contains(&action.kind) {
            return Err(AdapterError {
                kind: AdapterErrorKind::Unsupported,
                message: format!("debian BTS does not support action: {}", action.kind),
            });
        }
        // Debian BTS external_id must be a positive integer.
        let raw = action.issue.external_id.as_str();
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidConfiguration,
                message: "debian BTS issue id must be a positive integer".to_string(),
            });
        }
        if raw.len() > 1 && raw.starts_with('0') {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidConfiguration,
                message: "debian BTS issue id must not have leading zeros".to_string(),
            });
        }
        Ok(())
    }
}

#[must_use]
pub fn debian_issues() -> &'static DebianIssues {
    // The trait object requires a 'static reference; DebianIssues is
    // zero-sized, so a single leaked instance is fine for tests and
    // for the orchestrator's owned reference.
    static SINGLETON: LazyLock<DebianIssues> = LazyLock::new(DebianIssues::new);
    &SINGLETON
}

/// Returns the singleton [`IssueProviderId`] for Debian BTS.
///
/// Tests reach this through the trait, but the fixtures module calls
/// it directly to construct a valid `IssueRef`.
#[must_use]
pub fn provider_id() -> IssueProviderId {
    *PROVIDER_ID
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_policy::{AuthorizationState, IssueAction, IssueActionKind as PKind};

    fn action_with_id(id: &str, kind: IssueActionKind) -> IssueAction {
        IssueAction::new(
            ironmaint_core::IssueRef::new(*PROVIDER_ID, id.to_string()).unwrap(),
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
    fn supported_actions_include_markpending_and_forward() {
        let i = DebianIssues::new();
        let kinds = i.supported_actions();
        assert!(kinds.contains(&PKind::MarkPending));
        assert!(kinds.contains(&PKind::Forward));
    }

    #[test]
    fn unsupported_action_rejected() {
        let i = DebianIssues::new();
        let action = action_with_id("123456", IssueActionKind::RemoveLabels);
        let err = i.validate_action(&action).unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::Unsupported);
    }

    #[test]
    fn non_numeric_id_rejected() {
        let i = DebianIssues::new();
        let action = action_with_id("not-a-number", IssueActionKind::Comment);
        let err = i.validate_action(&action).unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn leading_zero_id_rejected() {
        let i = DebianIssues::new();
        let action = action_with_id("012345", IssueActionKind::Comment);
        let err = i.validate_action(&action).unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn valid_id_accepted() {
        let i = DebianIssues::new();
        let action = action_with_id("823451", IssueActionKind::MarkPending);
        assert!(i.validate_action(&action).is_ok());
    }

    #[test]
    fn authorization_state_is_proposed_after_construction() {
        let action = action_with_id("1", IssueActionKind::Comment);
        assert_eq!(action.authorization, AuthorizationState::Proposed);
    }
}
