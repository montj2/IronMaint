//! Privileged operations and their authorization state (§§39, §41).
//!
//! A [`PrivilegedOperation`] is "an external side effect that
//! actually does something we can't undo" — opening a BTS ticket,
//! submitting to Koji, signing a package, pushing to a canonical
//! repository. These are first-class objects, not hidden inside
//! adapter methods, so the audit trail can carry them.
//!
//! Agents create operations in [`AuthorizationState::Proposed`].
//! Only the privileged service may transition the state forward
//! to `Authorized` → `Executing` → `Succeeded` / `Failed` /
//! `Cancelled`. The state machine does NOT evaluate authorization
//! state (§2.7 keeps privileged operations external to workflow
//! state).

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ironmaint_core::{ApprovalId, CandidateFingerprint, OperationId};

/// Lifecycle state of a [`PrivilegedOperation`] (§39).
///
/// `Proposed` is the only state an agent may create. `Authorized`
/// is set by the privileged service after collecting the required
/// approvals. `Executing` is set when the external action begins;
/// terminal states are `Succeeded`, `Failed`, `Cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationState {
    /// Initial state: agent-created, awaiting required approvals.
    Proposed,
    /// Privileged service: required approvals collected and verified.
    Authorized,
    /// Privileged service: external action is in flight.
    Executing,
    /// Terminal: external action completed successfully.
    Succeeded,
    /// Terminal: external action attempted and failed.
    Failed,
    /// Terminal: operator or policy cancelled before authorization.
    Cancelled,
}

impl AuthorizationState {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Authorized => "authorized",
            Self::Executing => "executing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether this state is terminal (no further transitions).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

impl fmt::Display for AuthorizationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Kind of privileged operation (§41).
///
/// `Other(String)` is the explicit escape hatch for adapter- or
/// org-specific actions the core vocabulary doesn't enumerate
/// (e.g. `debian.dak.acceptance`, `fedora.packit.create-update`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedOperationKind {
    IssueTrackerMutation,
    CanonicalRepositoryPush,
    RemoteBuildSubmission,
    DistributionUpdateCreation,
    Signing,
    /// Adapter- or org-specific kinds.
    Other(String),
}

impl PrivilegedOperationKind {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::IssueTrackerMutation => "issue_tracker_mutation",
            Self::CanonicalRepositoryPush => "canonical_repository_push",
            Self::RemoteBuildSubmission => "remote_build_submission",
            Self::DistributionUpdateCreation => "distribution_update_creation",
            Self::Signing => "signing",
            Self::Other(_) => "other",
        }
    }
}

impl fmt::Display for PrivilegedOperationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(s) => write!(f, "other({s})"),
            other => f.write_str(other.name()),
        }
    }
}

/// A privileged side-effect bound to a candidate (§41).
///
/// `required_approvals` is the list of [`ApprovalId`]s that must be
/// in `Approved` before [`AuthorizationState`] may advance past
/// `Proposed`. Agents create operations here; only the privileged
/// service may mutate the [`AuthorizationState`].
///
/// - **Represents**: one privileged side-effect (BTS comment, Koji
///   submission, Bodhi update, signing key use, canonical push)
///   bound to a candidate, plus the approvals required to authorize
///   it and the current authorization state.
/// - **Mutation ownership**: agents and adapters create operations
///   in `Proposed`; only the privileged service may mutate
///   `AuthorizationState` past `Proposed` (§41). The set of
///   `required_approvals` is fixed at construction; changing
///   authorization requirements means creating a new operation.
/// - **Invariants**: `required_approvals` must all be `Approved`
///   before `AuthorizationState` may advance past `Proposed`. The
///   state machine is
///   `Proposed → Authorized → Executing → Succeeded | Failed | Cancelled`;
///   transitions are owned by the privileged service and the
///   executor (Phase 0B).
/// - **Does NOT represent**: NOT an executed side-effect, NOT an
///   audit log entry, NOT a retry token. The operation is a *plan
///   to perform* a privileged action; performing it is the executor's
///   job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct PrivilegedOperation {
    pub id: OperationId,
    pub kind: PrivilegedOperationKind,
    pub candidate: CandidateFingerprint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_approvals: Vec<ApprovalId>,
    pub authorization: AuthorizationState,
}

impl PrivilegedOperation {
    /// Construct a [`PrivilegedOperation`] in the [`AuthorizationState::Proposed`]
    /// state — the only state agents and adapters may create.
    #[must_use]
    pub fn proposed(kind: PrivilegedOperationKind, candidate: CandidateFingerprint) -> Self {
        Self {
            id: OperationId::new(),
            kind,
            candidate,
            required_approvals: Vec::new(),
            authorization: AuthorizationState::Proposed,
        }
    }

    #[must_use]
    pub fn with_required_approval(mut self, id: ApprovalId) -> Self {
        self.required_approvals.push(id);
        self
    }

    /// Authorization for a forward transition. Returns `None` if
    /// the transition is illegal (e.g. from a terminal state, or
    /// skipping `Authorized`).
    ///
    /// Only the privileged service should call this; it lives on
    /// the type for testability.
    #[must_use]
    pub fn authorized(self) -> Option<Self> {
        if self.authorization == AuthorizationState::Proposed {
            Some(Self {
                authorization: AuthorizationState::Authorized,
                ..self
            })
        } else {
            None
        }
    }

    #[must_use]
    pub fn executing(self) -> Option<Self> {
        if self.authorization == AuthorizationState::Authorized {
            Some(Self {
                authorization: AuthorizationState::Executing,
                ..self
            })
        } else {
            None
        }
    }

    #[must_use]
    pub fn succeeded(self) -> Option<Self> {
        if self.authorization == AuthorizationState::Executing {
            Some(Self {
                authorization: AuthorizationState::Succeeded,
                ..self
            })
        } else {
            None
        }
    }

    #[must_use]
    pub fn failed(self) -> Option<Self> {
        if self.authorization == AuthorizationState::Executing {
            Some(Self {
                authorization: AuthorizationState::Failed,
                ..self
            })
        } else {
            None
        }
    }

    /// Cancellation is permitted from any non-terminal state.
    #[must_use]
    pub fn cancelled(self) -> Option<Self> {
        if self.authorization.is_terminal() {
            None
        } else {
            Some(Self {
                authorization: AuthorizationState::Cancelled,
                ..self
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    #[test]
    fn authorization_state_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&AuthorizationState::Authorized).unwrap(),
            r#""authorized""#
        );
    }

    #[test]
    fn authorization_state_is_terminal_predicate() {
        assert!(!AuthorizationState::Proposed.is_terminal());
        assert!(!AuthorizationState::Authorized.is_terminal());
        assert!(!AuthorizationState::Executing.is_terminal());
        assert!(AuthorizationState::Succeeded.is_terminal());
        assert!(AuthorizationState::Failed.is_terminal());
        assert!(AuthorizationState::Cancelled.is_terminal());
    }

    #[test]
    fn operation_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&PrivilegedOperationKind::RemoteBuildSubmission).unwrap(),
            r#""remote_build_submission""#
        );
    }

    #[test]
    fn operation_kind_other_carries_string() {
        let k = PrivilegedOperationKind::Other("dak.acceptance".to_string());
        let json = serde_json::to_string(&k).unwrap();
        let parsed: PrivilegedOperationKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, k);
    }

    #[test]
    fn operation_kind_display() {
        assert_eq!(PrivilegedOperationKind::Signing.to_string(), "signing");
        assert_eq!(
            PrivilegedOperationKind::Other("foo".into()).to_string(),
            "other(foo)"
        );
    }

    #[test]
    fn proposed_starts_in_proposed_state() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::IssueTrackerMutation, fp());
        assert_eq!(op.authorization, AuthorizationState::Proposed);
        assert!(op.required_approvals.is_empty());
    }

    #[test]
    fn with_required_approval_pushes_id() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp())
            .with_required_approval(ApprovalId::new())
            .with_required_approval(ApprovalId::new());
        assert_eq!(op.required_approvals.len(), 2);
    }

    #[test]
    fn lifecycle_progression() {
        let op =
            PrivilegedOperation::proposed(PrivilegedOperationKind::CanonicalRepositoryPush, fp());
        let op = op.authorized().unwrap();
        let op = op.executing().unwrap();
        let op = op.succeeded().unwrap();
        assert_eq!(op.authorization, AuthorizationState::Succeeded);
    }

    #[test]
    fn authorized_requires_proposed() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp())
            .authorized()
            .unwrap()
            .executing()
            .unwrap();
        assert!(op.authorized().is_none());
    }

    #[test]
    fn executing_requires_authorized() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp());
        assert!(op.executing().is_none());
    }

    #[test]
    fn succeeded_requires_executing() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp());
        assert!(op.clone().succeeded().is_none());
        let op = op.authorized().unwrap();
        assert!(op.succeeded().is_none());
    }

    #[test]
    fn cancelled_from_proposed() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp());
        let cancelled = op.cancelled().unwrap();
        assert_eq!(cancelled.authorization, AuthorizationState::Cancelled);
    }

    #[test]
    fn cancelled_from_authorized() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp())
            .authorized()
            .unwrap()
            .cancelled()
            .unwrap();
        assert_eq!(op.authorization, AuthorizationState::Cancelled);
    }

    #[test]
    fn cancelled_from_executing() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp())
            .authorized()
            .unwrap()
            .executing()
            .unwrap()
            .cancelled()
            .unwrap();
        assert_eq!(op.authorization, AuthorizationState::Cancelled);
    }

    #[test]
    fn cancelled_from_terminal_returns_none() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp())
            .authorized()
            .unwrap()
            .executing()
            .unwrap()
            .succeeded()
            .unwrap();
        assert!(op.cancelled().is_none());
    }

    #[test]
    fn failed_requires_executing() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp());
        assert!(op.failed().is_none());
    }

    #[test]
    fn failed_from_executing() {
        let op = PrivilegedOperation::proposed(PrivilegedOperationKind::Signing, fp())
            .authorized()
            .unwrap()
            .executing()
            .unwrap()
            .failed()
            .unwrap();
        assert_eq!(op.authorization, AuthorizationState::Failed);
    }

    #[test]
    fn operation_round_trips() {
        let op = PrivilegedOperation::proposed(
            PrivilegedOperationKind::DistributionUpdateCreation,
            fp(),
        )
        .with_required_approval(ApprovalId::new());
        let json = serde_json::to_string(&op).unwrap();
        let parsed: PrivilegedOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, op);
    }
}
