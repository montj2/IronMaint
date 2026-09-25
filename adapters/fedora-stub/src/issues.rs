//! Fedora issue capability (§51).
//!
//! Provider identity is a stable, lazily-initialised
//! [`IssueProviderId`] distinct from the Debian BTS one.
//! Validation: Bugzilla numbers are decimal, ≥1, no leading zeros
//! outside of "0" itself.

use std::sync::LazyLock;

use ironmaint_adapter_api::{AdapterError, AdapterErrorKind, IssueCapability};
use ironmaint_core::IssueProviderId;
use ironmaint_policy::{IssueAction, IssueActionKind};

static PROVIDER_ID: LazyLock<IssueProviderId> = LazyLock::new(IssueProviderId::new);
static SUPPORTED: LazyLock<Vec<IssueActionKind>> = LazyLock::new(|| {
    vec![
        IssueActionKind::SetSeverity,
        IssueActionKind::Comment,
        IssueActionKind::Reopen,
        IssueActionKind::AddLabels,
        IssueActionKind::RemoveLabels,
    ]
});

pub struct FedoraIssues;

impl FedoraIssues {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FedoraIssues {
    fn default() -> Self {
        Self::new()
    }
}

impl IssueCapability for FedoraIssues {
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
                message: format!("fedora bugzilla does not support action: {}", action.kind),
            });
        }
        let raw = action.issue.external_id.as_str();
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidConfiguration,
                message: "fedora bugzilla id must be a positive integer".to_string(),
            });
        }
        if raw.len() > 1 && raw.starts_with('0') {
            return Err(AdapterError {
                kind: AdapterErrorKind::InvalidConfiguration,
                message: "fedora bugzilla id must not have leading zeros".to_string(),
            });
        }
        Ok(())
    }
}

#[must_use]
pub fn fedora_issues() -> &'static FedoraIssues {
    static SINGLETON: LazyLock<FedoraIssues> = LazyLock::new(FedoraIssues::new);
    &SINGLETON
}

#[must_use]
pub fn provider_id() -> IssueProviderId {
    *PROVIDER_ID
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_policy::IssueActionKind as PKind;

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
        let i = FedoraIssues::new();
        assert_eq!(i.provider_id(), i.provider_id());
    }

    #[test]
    fn supported_actions_include_setseverity_and_reopen() {
        let i = FedoraIssues::new();
        let kinds = i.supported_actions();
        assert!(kinds.contains(&PKind::SetSeverity));
        assert!(kinds.contains(&PKind::Reopen));
    }

    #[test]
    fn unsupported_action_rejected() {
        let i = FedoraIssues::new();
        let action = action_with_id("123", IssueActionKind::MarkPending);
        let err = i.validate_action(&action).unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::Unsupported);
    }

    #[test]
    fn non_numeric_id_rejected() {
        let i = FedoraIssues::new();
        let action = action_with_id("FEDORA-1", IssueActionKind::Comment);
        let err = i.validate_action(&action).unwrap_err();
        assert_eq!(err.kind, AdapterErrorKind::InvalidConfiguration);
    }

    #[test]
    fn valid_id_accepted() {
        let i = FedoraIssues::new();
        let action = action_with_id("123456", IssueActionKind::Reopen);
        assert!(i.validate_action(&action).is_ok());
    }
}
