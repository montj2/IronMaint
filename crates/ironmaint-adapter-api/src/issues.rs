//! Issue capability (§51) and issue plans.
//!
//! The issue capability describes what actions a distribution's
//! issue tracker supports; the executor (Phase 0B) materializes
//! concrete [`IssuePlan`]s into [`ironmaint_policy::IssueAction`]
//! executions.

use ironmaint_core::IssueProviderId;
use ironmaint_policy::{IssueAction, IssueActionKind};

use crate::error::AdapterError;

/// Bundle of [`IssueAction`]s an adapter proposes for one candidate
/// cycle. (§4.5 lists this in the adapter-api ownership surface.)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssuePlan {
    pub actions: Vec<IssueAction>,
}

impl IssuePlan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_action(mut self, action: IssueAction) -> Self {
        self.actions.push(action);
        self
    }
}

/// Optional capability: describe an issue tracker (§51).
pub trait IssueCapability: Send + Sync + 'static {
    /// Stable identifier for the provider (BTS, Bugzilla, etc.).
    fn provider_id(&self) -> IssueProviderId;

    /// Subset of [`IssueActionKind`] the adapter supports.
    fn supported_actions(&self) -> Vec<IssueActionKind>;

    /// Validate that `action` is one the provider can perform.
    ///
    /// # Errors
    /// Returns `AdapterError { kind: Unsupported, .. }` if the
    /// action kind is outside [`Self::supported_actions`].
    fn validate_action(&self, action: &IssueAction) -> Result<(), AdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_plan_starts_empty() {
        let plan = IssuePlan::new();
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn trait_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static + ?Sized>() {}
        assert_send_sync_static::<dyn IssueCapability>();
    }
}
