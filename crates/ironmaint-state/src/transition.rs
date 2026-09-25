//! Transition vocabulary — request, decision, blocker, rule (§§18–20).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::{ApprovalId, CandidateFingerprint, GateId, JobId, ObligationId};
use ironmaint_evidence::{GateDefinition, GateStage};
use ironmaint_policy::{ApprovalCategory, ApprovalDecision, ApprovalRequirement, ObligationSet};

/// Caller's request to evaluate (and, on `Allowed`, apply) a transition (§18).
///
/// `expected_version` is the optimistic-concurrency token — the
/// engine returns `StaleProjection` if the supplied projection's
/// `version` doesn't match. `now` is the wall-clock timestamp the
/// engine stamps onto the new projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRequest {
    pub job_id: JobId,
    pub expected_version: u64,
    pub target: ironmaint_core::JobState,
    pub now: OffsetDateTime,
}

/// A single transition decision (§18).
///
/// `rule_index` is the index into the engine's static transition
/// table — useful in logs and audit trails to identify exactly which
/// §20 rule fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub from: ironmaint_core::JobState,
    pub to: ironmaint_core::JobState,
    pub rule_index: usize,
}

/// Outcome of [`crate::TransitionEngine::evaluate`] (§18).
///
/// `Blocked` carries the complete list of blockers; the engine does
/// not short-circuit, so the caller can render all problems in one
/// error response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionDecision {
    Allowed(Transition),
    Blocked(Vec<TransitionBlocker>),
}

/// A specific reason a transition is blocked (§19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionBlocker {
    /// `from` → `to` is not in the §20 graph.
    InvalidStatePath,
    /// The projection version doesn't match the request.
    StaleProjection { expected: u64, found: u64 },
    /// A mandatory gate has no [`GateResult`] at all.
    MissingGate(GateId),
    /// A mandatory gate evaluated to `Fail` or `ReviewRequired`.
    FailedGate(GateId),
    /// A mandatory gate evaluated to `NotEvaluated` or `Blocked`.
    IncompleteGate(GateId),
    /// A mandatory obligation is `NotEvaluated`.
    MissingObligation(ObligationId),
    /// A mandatory obligation is `Fail`.
    FailedObligation(ObligationId),
    /// A mandatory obligation is `RequiresReview`.
    ReviewRequired(ObligationId),
    /// A mandatory obligation is `ExceptionApproved` but the
    /// supporting approval decision is not `Approved`.
    UnbackedException(ObligationId),
    /// A required approval category has no `Approved` decision.
    MissingApproval(ApprovalId),
    /// Evidence was stale — a candidate change invalidated it (§30).
    StaleEvidence(ApprovalId),
    /// Gate or evidence references a different candidate than the
    /// active one.
    CandidateMismatch {
        expected: CandidateFingerprint,
        found: CandidateFingerprint,
    },
    /// The runtime reported an infrastructure problem and the target
    /// is not one of the recovery states.
    InfrastructureBlocked,
}

/// Per-rule requirements — the second column of the §20 graph.
///
/// `gates` lists the [`GateStage`]s that must have a passing result
/// keyed to the active candidate before this transition is allowed.
/// `require_policy_completion` triggers the §35 obligation check.
/// `approvals` lists [`ApprovalCategory`]s that must each have at
/// least one matching `Approved` decision.
#[derive(Debug, Clone, Copy)]
pub struct TransitionRequirements {
    pub gates: &'static [GateStage],
    pub require_policy_completion: bool,
    pub approvals: &'static [ApprovalCategory],
}

/// One row of the §20 transition graph.
#[derive(Debug, Clone, Copy)]
pub struct TransitionRule {
    pub from: ironmaint_core::JobState,
    pub to: ironmaint_core::JobState,
    pub requirements: TransitionRequirements,
}

impl TransitionRule {
    #[must_use]
    pub const fn new(
        from: ironmaint_core::JobState,
        to: ironmaint_core::JobState,
        requirements: TransitionRequirements,
    ) -> Self {
        Self {
            from,
            to,
            requirements,
        }
    }
}

/// Input to [`crate::TransitionEngine::evaluate`].
///
/// The engine is purely synchronous and never mutates its inputs;
/// the caller is responsible for replacing the projection after
/// applying an `Allowed` decision (§18).
#[derive(Debug)]
pub struct TransitionContext<'a> {
    /// Current projection. The engine reads `state`, `version`, and
    /// `job.id`; it does NOT mutate the projection itself.
    pub current: &'a ironmaint_core::JobProjection,
    /// Active candidate fingerprint, looked up by the caller from
    /// storage. `None` in early states (before [`JobState::CandidateAssembly`]).
    pub active_candidate_fingerprint: Option<CandidateFingerprint>,
    /// Evaluated gate results keyed by gate id.
    pub gates: &'a BTreeMap<GateId, ironmaint_evidence::GateResult>,
    /// Gate definitions keyed by gate id. The engine looks up by
    /// `(stage, candidate)` to decide which gate each
    /// [`TransitionRequirements::gates`] entry refers to.
    pub gate_definitions: &'a BTreeMap<GateId, GateDefinition>,
    /// Approval requirements keyed by approval id. The engine
    /// matches by `(category, candidate)` to satisfy
    /// [`TransitionRequirements::approvals`].
    pub approval_requirements: &'a BTreeMap<ApprovalId, ApprovalRequirement>,
    /// Candidate-scoped obligations.
    pub obligations: &'a ObligationSet,
    /// Recorded approval decisions keyed by approval id.
    pub approvals: &'a BTreeMap<ApprovalId, ApprovalDecision>,
    /// Whether the runtime reports an infrastructure problem.
    pub infrastructure_blocked: bool,
    /// Resume record for exceptional state transitions (§21). `None`
    /// for normal-flow transitions; required for transitions out of
    /// `HumanReviewRequired` or `InfrastructureBlocked`.
    pub resume_event: Option<&'a ResumeRecord>,
}

/// Recorded event allowing an exceptional state to be resumed (§21).
///
/// Captured at runtime by the executor; the engine treats it as an
/// opaque input that bounds which transitions out of an exceptional
/// state are valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRecord {
    pub from: ironmaint_core::JobState,
    pub to: ironmaint_core::JobState,
    #[serde(with = "time::serde::rfc3339")]
    pub recorded_at: OffsetDateTime,
}

impl ResumeRecord {
    #[must_use]
    pub fn new(
        from: ironmaint_core::JobState,
        to: ironmaint_core::JobState,
        recorded_at: OffsetDateTime,
    ) -> Self {
        Self {
            from,
            to,
            recorded_at,
        }
    }
}

/// Errors returned by [`crate::TransitionEngine::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionApplyError {
    Blocked(Vec<TransitionBlocker>),
    StaleProjection { expected: u64, found: u64 },
}

/// A placeholder type for adapter-supplied requirements (§12, deferred to 0A.4).
///
/// 0A.3 does not yet have adapter ports; this marker exists so the
/// [`TransitionContext`] slot is fixed for 0A.4 to populate without
/// breaking the engine API.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdapterRequirement;

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::JobState;

    #[test]
    fn transition_decision_allowed_carries_rule_index() {
        let t = Transition {
            from: JobState::EventDetected,
            to: JobState::Intake,
            rule_index: 0,
        };
        match TransitionDecision::Allowed(t.clone()) {
            TransitionDecision::Allowed(t2) => assert_eq!(t2, t),
            TransitionDecision::Blocked(_) => unreachable!(),
        }
    }

    #[test]
    fn transition_decision_blocked_carries_blockers() {
        let bs = vec![TransitionBlocker::InvalidStatePath];
        match TransitionDecision::Blocked(bs.clone()) {
            TransitionDecision::Blocked(b) => assert_eq!(b, bs),
            TransitionDecision::Allowed(_) => unreachable!(),
        }
    }

    #[test]
    fn resume_record_construction() {
        let r = ResumeRecord::new(
            JobState::HumanReviewRequired,
            JobState::SourceReview,
            time::macros::datetime!(2026-01-01 00:00:00 UTC),
        );
        assert_eq!(r.from, JobState::HumanReviewRequired);
        assert_eq!(r.to, JobState::SourceReview);
    }

    #[test]
    fn resume_record_round_trips() {
        let r = ResumeRecord::new(
            JobState::InfrastructureBlocked,
            JobState::BuildValidation,
            time::macros::datetime!(2026-02-02 12:00:00 UTC),
        );
        let json = serde_json::to_string(&r).unwrap();
        let parsed: ResumeRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn transition_rule_construction() {
        let r = TransitionRule::new(
            JobState::EventDetected,
            JobState::Intake,
            TransitionRequirements {
                gates: &[GateStage::SourcePreparation],
                require_policy_completion: false,
                approvals: &[],
            },
        );
        assert_eq!(r.from, JobState::EventDetected);
        assert_eq!(r.to, JobState::Intake);
    }
}
