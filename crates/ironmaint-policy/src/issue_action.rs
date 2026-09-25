//! Issue actions — proposed external issue mutations (§38).
//!
//! An [`IssueAction`] is what an adapter proposes to do to a bug
//! tracker entry. It always begins life in
//! [`crate::AuthorizationState::Proposed`]; only the privileged
//! service may transition it forward. Agents and adapters can create
//! actions; only the privileged service may mutate the authorization
//! state.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ironmaint_core::{CandidateFingerprint, EvidenceId, IssueActionId, IssueRef};

use crate::AuthorizationState;

const RATIONALE_MAX: usize = 2048;

/// Generic kind of an issue mutation (§38).
///
/// Adapters translate this vocabulary into provider-specific
/// semantics. The `non_exhaustive` attribute signals the enum grows
/// over time — adapters should treat unknown variants as `Other(_)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IssueActionKind {
    Comment,
    Close,
    Reopen,
    SetSeverity,
    AddLabels,
    RemoveLabels,
    Reassign,
    Link,
    MarkFixed,
    MarkPending,
    Forward,
    CreateRelated,
    /// Adapter- or provider-specific kinds.
    Other(String),
}

impl IssueActionKind {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Comment => "comment",
            Self::Close => "close",
            Self::Reopen => "reopen",
            Self::SetSeverity => "set_severity",
            Self::AddLabels => "add_labels",
            Self::RemoveLabels => "remove_labels",
            Self::Reassign => "reassign",
            Self::Link => "link",
            Self::MarkFixed => "mark_fixed",
            Self::MarkPending => "mark_pending",
            Self::Forward => "forward",
            Self::CreateRelated => "create_related",
            Self::Other(s) => s,
        }
    }
}

impl fmt::Display for IssueActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A proposed external issue mutation (§38).
///
/// `candidate` is `Option` because an issue action may apply to a
/// tracker entry that is not yet bound to a candidate (e.g. an initial
/// `Comment` on an upstream issue that has no in-progress candidate).
///
/// `evidence` is `Vec<EvidenceId>` — opaque pointers to evidence
/// values that justify the action. Policy intentionally does NOT
/// depend on `ironmaint-evidence`; the IDs are passed through.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct IssueAction {
    pub id: IssueActionId,
    pub issue: IssueRef,
    pub candidate: Option<CandidateFingerprint>,
    pub kind: IssueActionKind,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceId>,
    pub authorization: AuthorizationState,
}

impl IssueAction {
    /// Construct an [`IssueAction`] in [`AuthorizationState::Proposed`].
    ///
    /// # Errors
    /// Returns `Err(&'static str)` if `rationale` exceeds
    /// [`RATIONALE_MAX`] bytes.
    pub fn new(
        issue: IssueRef,
        candidate: Option<CandidateFingerprint>,
        kind: IssueActionKind,
        rationale: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let r = rationale.into();
        if r.len() > RATIONALE_MAX {
            return Err("issue action rationale exceeds 2048 bytes");
        }
        Ok(Self {
            id: IssueActionId::new(),
            issue,
            candidate,
            kind,
            rationale: r,
            evidence: Vec::new(),
            authorization: AuthorizationState::Proposed,
        })
    }

    #[must_use]
    pub fn with_evidence(mut self, id: EvidenceId) -> Self {
        self.evidence.push(id);
        self
    }

    /// Transition forward through the authorization lifecycle.
    ///
    /// Agents and adapters may create an action in `Proposed`; only
    /// the privileged service should call `authorized()` /
    /// `executing()` / `succeeded()` / `failed()` /
    /// `cancelled()`. The methods live on the type for testability.
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
    use ironmaint_core::{CandidateFingerprint, IssueProviderId};

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    fn ref_() -> IssueRef {
        IssueRef::new(IssueProviderId::new(), "823451").unwrap()
    }

    #[test]
    fn kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&IssueActionKind::MarkPending).unwrap(),
            r#""mark_pending""#
        );
        assert_eq!(
            serde_json::to_string(&IssueActionKind::SetSeverity).unwrap(),
            r#""set_severity""#
        );
    }

    #[test]
    fn kind_other_carries_string() {
        let k = IssueActionKind::Other("debian.dak.accept".to_string());
        let json = serde_json::to_string(&k).unwrap();
        let parsed: IssueActionKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, k);
    }

    #[test]
    fn action_starts_in_proposed_state() {
        let a = IssueAction::new(
            ref_(),
            Some(fp()),
            IssueActionKind::MarkPending,
            "automated upload",
        )
        .unwrap();
        assert_eq!(a.authorization, AuthorizationState::Proposed);
        assert!(a.evidence.is_empty());
    }

    #[test]
    fn action_rejects_oversize_rationale() {
        let long = "x".repeat(RATIONALE_MAX + 1);
        let err = IssueAction::new(ref_(), None, IssueActionKind::Comment, long).unwrap_err();
        assert!(err.contains("2048"));
    }

    #[test]
    fn action_with_evidence() {
        let a = IssueAction::new(
            ref_(),
            Some(fp()),
            IssueActionKind::SetSeverity,
            "raised to critical per upstream",
        )
        .unwrap()
        .with_evidence(EvidenceId::new())
        .with_evidence(EvidenceId::new());
        assert_eq!(a.evidence.len(), 2);
    }

    #[test]
    fn action_lifecycle_progression() {
        let a = IssueAction::new(ref_(), Some(fp()), IssueActionKind::Forward, "r").unwrap();
        let a = a.authorized().unwrap();
        let a = a.executing().unwrap();
        let a = a.succeeded().unwrap();
        assert_eq!(a.authorization, AuthorizationState::Succeeded);
    }

    #[test]
    fn action_authorized_requires_proposed() {
        let a = IssueAction::new(ref_(), None, IssueActionKind::Close, "r").unwrap();
        // Skipping Authorized.
        assert!(a.clone().executing().is_none());
        // After Proposed -> Authorized -> Executing, Authorized again fails.
        let a = a.authorized().unwrap().executing().unwrap();
        assert!(a.authorized().is_none());
    }

    #[test]
    fn action_cancelled_from_any_non_terminal() {
        let a = IssueAction::new(ref_(), Some(fp()), IssueActionKind::Comment, "r").unwrap();
        let c = a.cancelled().unwrap();
        assert_eq!(c.authorization, AuthorizationState::Cancelled);

        let a = IssueAction::new(ref_(), None, IssueActionKind::Comment, "r").unwrap();
        let a = a.authorized().unwrap();
        let c = a.cancelled().unwrap();
        assert_eq!(c.authorization, AuthorizationState::Cancelled);
    }

    #[test]
    fn action_round_trips() {
        let a = IssueAction::new(
            ref_(),
            Some(fp()),
            IssueActionKind::MarkPending,
            "automated",
        )
        .unwrap()
        .with_evidence(EvidenceId::new());
        let json = serde_json::to_string(&a).unwrap();
        let parsed: IssueAction = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, a);
    }
}
