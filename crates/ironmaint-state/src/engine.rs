//! The [`TransitionEngine`] — the only writer of workflow state (§§18–21).
//!
//! [`TransitionEngine::evaluate`] is a pure function: it does not
//! mutate its inputs. [`TransitionEngine::apply`] builds a
//! [`StateTransitioned`] event from an allowed decision; the caller
//! is responsible for persisting the new projection (per §18).

use ironmaint_core::{ApprovalId, CandidateFingerprint, GateId, JobState};
use ironmaint_evidence::{GateStage, GateStatus, RequiredEvidenceStatus};
use ironmaint_policy::{Applicability, ApprovalCategory, ApprovalDecision, ObligationStatus};

use crate::event::StateTransitioned;
use crate::transition::{
    ResumeRecord, Transition, TransitionApplyError, TransitionBlocker, TransitionContext,
    TransitionDecision, TransitionRequest, TransitionRequirements, TransitionRule,
};

/// `rule_index` value used for transitions the engine synthesizes
/// rather than reading from the static table (exceptional targets,
/// cancel, resume).
///
/// Callers should treat `usize::MAX` as "not from the table".
pub const SYNTHETIC_RULE_INDEX: usize = usize::MAX;

/// The static §20 transition graph (forward, normal flow).
///
/// Exceptional transitions (`→HumanReviewRequired`,
/// `→InfrastructureBlocked`, `→Cancelled`) and resume transitions
/// (`HumanReviewRequired→` / `InfrastructureBlocked→`) are matched
/// by [`TransitionEngine::evaluate`] directly — they have no fixed
/// `from` (excellentials are reachable from anywhere non-terminal)
/// or are gated by [`ResumeRecord`] rather than by static rules
/// (resume).
pub const TRANSITION_RULES: &[TransitionRule] = &[
    // 1. EventDetected → Intake
    TransitionRule::new(
        JobState::EventDetected,
        JobState::Intake,
        TransitionRequirements {
            gates: &[GateStage::SourcePreparation],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 2. Intake → SourceReview
    TransitionRule::new(
        JobState::Intake,
        JobState::SourceReview,
        TransitionRequirements {
            gates: &[GateStage::SourceAnalysis],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 3. SourceReview → CandidateAssembly
    TransitionRule::new(
        JobState::SourceReview,
        JobState::CandidateAssembly,
        TransitionRequirements {
            gates: &[GateStage::IssueAnalysis],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 4. CandidateAssembly → SourceRevision
    TransitionRule::new(
        JobState::CandidateAssembly,
        JobState::SourceRevision,
        TransitionRequirements {
            gates: &[GateStage::Maintenance],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 5. SourceRevision → SourceIntegrity
    TransitionRule::new(
        JobState::SourceRevision,
        JobState::SourceIntegrity,
        TransitionRequirements {
            gates: &[GateStage::PolicyEvaluation],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 6. SourceIntegrity → BuildValidation
    TransitionRule::new(
        JobState::SourceIntegrity,
        JobState::BuildValidation,
        TransitionRequirements {
            gates: &[GateStage::BuildValidation],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 7. BuildValidation → PackageQaValidation
    TransitionRule::new(
        JobState::BuildValidation,
        JobState::PackageQaValidation,
        TransitionRequirements {
            gates: &[GateStage::PackageQa],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 8. PackageQaValidation → FunctionalValidation
    TransitionRule::new(
        JobState::PackageQaValidation,
        JobState::FunctionalValidation,
        TransitionRequirements {
            gates: &[GateStage::FunctionalValidation],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 9. FunctionalValidation → UpgradeValidation
    TransitionRule::new(
        JobState::FunctionalValidation,
        JobState::UpgradeValidation,
        TransitionRequirements {
            gates: &[GateStage::UpgradeValidation],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 10. UpgradeValidation → ReleaseReview
    TransitionRule::new(
        JobState::UpgradeValidation,
        JobState::ReleaseReview,
        TransitionRequirements {
            gates: &[GateStage::ReleaseReview],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 11. ReleaseReview → FinalValidation
    TransitionRule::new(
        JobState::ReleaseReview,
        JobState::FinalValidation,
        TransitionRequirements {
            gates: &[GateStage::CandidateAssembly],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 12. FinalValidation → ReadyForApproval
    TransitionRule::new(
        JobState::FinalValidation,
        JobState::ReadyForApproval,
        TransitionRequirements {
            gates: &[GateStage::FinalValidation],
            require_policy_completion: true,
            approvals: &[],
        },
    ),
    // 13. ReadyForApproval → Approved
    TransitionRule::new(
        JobState::ReadyForApproval,
        JobState::Approved,
        TransitionRequirements {
            gates: &[],
            require_policy_completion: false,
            approvals: &[ApprovalCategory::HumanReview],
        },
    ),
    // 14. Approved → PublicationPending
    TransitionRule::new(
        JobState::Approved,
        JobState::PublicationPending,
        TransitionRequirements {
            gates: &[],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
    // 15. PublicationPending → Published
    TransitionRule::new(
        JobState::PublicationPending,
        JobState::Published,
        TransitionRequirements {
            gates: &[GateStage::Publication],
            require_policy_completion: false,
            approvals: &[],
        },
    ),
];

/// The deterministic state machine.
#[derive(Debug, Clone, Copy)]
pub struct TransitionEngine {
    rules: &'static [TransitionRule],
}

impl Default for TransitionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TransitionEngine {
    /// Construct an engine bound to the static §20 graph.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rules: TRANSITION_RULES,
        }
    }

    /// Look up a static rule by `(from, to)`.
    #[must_use]
    pub fn rule_for(&self, from: JobState, to: JobState) -> Option<(usize, &TransitionRule)> {
        self.rules
            .iter()
            .position(|r| r.from == from && r.to == to)
            .map(|idx| (idx, &self.rules[idx]))
    }

    /// Pure evaluation — does not mutate [`TransitionContext`].
    pub fn evaluate(
        &self,
        request: &TransitionRequest,
        ctx: &TransitionContext<'_>,
    ) -> TransitionDecision {
        // 1. Optimistic-concurrency check (§17).
        if request.expected_version != ctx.current.version {
            return TransitionDecision::Blocked(vec![TransitionBlocker::StaleProjection {
                expected: request.expected_version,
                found: ctx.current.version,
            }]);
        }

        // 2. Terminal states: nothing leaves.
        if ctx.current.state.is_terminal() {
            return TransitionDecision::Blocked(vec![TransitionBlocker::InvalidStatePath]);
        }

        // 3. Exceptional targets and Cancelled: allowed from any
        //    non-terminal state with no requirements.
        if matches!(
            request.target,
            JobState::HumanReviewRequired | JobState::InfrastructureBlocked | JobState::Cancelled
        ) {
            return TransitionDecision::Allowed(Transition {
                from: ctx.current.state,
                to: request.target,
                rule_index: SYNTHETIC_RULE_INDEX,
            });
        }

        // 4. Resume from exceptional state: require a matching
        //    ResumeRecord (§21). No further requirements — the job
        //    has already proven the upstream gates before entering
        //    the exceptional state.
        if ctx.current.state.is_exceptional() {
            return match ctx.resume_event {
                Some(ResumeRecord { from, to, .. })
                    if *from == ctx.current.state && *to == request.target =>
                {
                    TransitionDecision::Allowed(Transition {
                        from: ctx.current.state,
                        to: request.target,
                        rule_index: SYNTHETIC_RULE_INDEX,
                    })
                }
                _ => TransitionDecision::Blocked(vec![TransitionBlocker::InvalidStatePath]),
            };
        }

        // 5. Normal flow: find a static rule.
        let (rule_index, rule) = match self.rule_for(ctx.current.state, request.target) {
            Some(r) => r,
            None => {
                return TransitionDecision::Blocked(vec![TransitionBlocker::InvalidStatePath]);
            }
        };

        // 6. Infrastructure blocked (§19). Cancelled and exceptional
        //    targets are handled above; for normal targets the
        //    blocker fires.
        if ctx.infrastructure_blocked {
            return TransitionDecision::Blocked(vec![TransitionBlocker::InfrastructureBlocked]);
        }

        // 7. Gate, obligation, and approval checks.
        let mut blockers = Vec::new();
        let active_candidate = ctx.active_candidate_fingerprint.as_ref();
        check_gates(&rule.requirements, active_candidate, ctx, &mut blockers);
        check_obligations(&rule.requirements, active_candidate, ctx, &mut blockers);
        check_approvals(&rule.requirements, active_candidate, ctx, &mut blockers);

        if blockers.is_empty() {
            TransitionDecision::Allowed(Transition {
                from: rule.from,
                to: rule.to,
                rule_index,
            })
        } else {
            TransitionDecision::Blocked(blockers)
        }
    }

    /// Apply an allowed transition. Returns the emitted
    /// [`StateTransitioned`] event, or an error if the decision was
    /// blocked.
    pub fn apply(
        &self,
        request: TransitionRequest,
        ctx: &TransitionContext<'_>,
    ) -> Result<StateTransitioned, TransitionApplyError> {
        match self.evaluate(&request, ctx) {
            TransitionDecision::Allowed(transition) => {
                let projection_after = ironmaint_core::JobProjection {
                    job: ctx.current.job.clone(),
                    state: transition.to,
                    active_candidate: ctx.current.active_candidate,
                    version: ctx.current.version + 1,
                    updated_at: request.now,
                };
                Ok(StateTransitioned::new(
                    transition,
                    projection_after,
                    request.now,
                ))
            }
            TransitionDecision::Blocked(blockers) => {
                if let Some(TransitionBlocker::StaleProjection { expected, found }) =
                    blockers.first()
                {
                    return Err(TransitionApplyError::StaleProjection {
                        expected: *expected,
                        found: *found,
                    });
                }
                Err(TransitionApplyError::Blocked(blockers))
            }
        }
    }
}

/// Sentinel [`GateId`] attached to a `MissingGate` blocker when no
/// definition exists for the active candidate and stage. The
/// caller-facing identifier is opaque; future code can map it to a
/// "stage with no definition" report.
fn missing_gate_sentinel() -> GateId {
    GateId::default()
}

/// Sentinel [`ApprovalId`] used when the required approval
/// category has no matching [`ironmaint_policy::ApprovalRequirement`]
/// in the context at all.
fn sentinel_approval_id() -> ApprovalId {
    ApprovalId::default()
}

fn check_gates(
    requirements: &TransitionRequirements,
    active_candidate: Option<&CandidateFingerprint>,
    ctx: &TransitionContext<'_>,
    blockers: &mut Vec<TransitionBlocker>,
) {
    if requirements.gates.is_empty() {
        return;
    }

    let Some(fp) = active_candidate else {
        // No active candidate yet — every required gate is missing.
        for _ in requirements.gates {
            blockers.push(TransitionBlocker::MissingGate(missing_gate_sentinel()));
        }
        return;
    };

    for &stage in requirements.gates {
        // Find a mandatory definition for this stage and candidate.
        let definition = ctx
            .gate_definitions
            .values()
            .find(|d| d.stage == stage && d.candidate == *fp && d.mandatory);

        let Some(def) = definition else {
            blockers.push(TransitionBlocker::MissingGate(missing_gate_sentinel()));
            continue;
        };

        let Some(result) = ctx.gates.get(&def.id) else {
            blockers.push(TransitionBlocker::MissingGate(def.id));
            continue;
        };

        if result.candidate != *fp {
            blockers.push(TransitionBlocker::CandidateMismatch {
                expected: fp.clone(),
                found: result.candidate.clone(),
            });
            continue;
        }

        match result.status {
            GateStatus::Pass => {
                // Always satisfies; no need to check minimum_status
                // because Pass is the strictest possible result.
            }
            GateStatus::NotApplicable => {
                if def.requirement.minimum_status == RequiredEvidenceStatus::Pass {
                    blockers.push(TransitionBlocker::FailedGate(def.id));
                }
                // PassOrNotApplicable accepts NotApplicable.
            }
            GateStatus::NotEvaluated | GateStatus::Blocked => {
                blockers.push(TransitionBlocker::IncompleteGate(def.id));
            }
            GateStatus::Fail | GateStatus::ReviewRequired => {
                blockers.push(TransitionBlocker::FailedGate(def.id));
            }
        }
    }
}

fn check_obligations(
    requirements: &TransitionRequirements,
    active_candidate: Option<&CandidateFingerprint>,
    ctx: &TransitionContext<'_>,
    blockers: &mut Vec<TransitionBlocker>,
) {
    if !requirements.require_policy_completion {
        return;
    }
    let Some(fp) = active_candidate else {
        return;
    };
    for obligation in ctx.obligations.iter().map(|(_, o)| o) {
        if obligation.candidate != *fp {
            continue;
        }
        if obligation.strength != ironmaint_policy::ObligationStrength::Mandatory {
            continue;
        }
        if obligation.applicability != Applicability::Applicable {
            continue;
        }
        match obligation.status {
            ObligationStatus::Pass => {}
            ObligationStatus::ExceptionApproved => {
                // §35: ExceptionApproved is valid only if an
                // explicit approval record exists. The Obligation
                // struct does not carry a direct approval-id link
                // in 0A.3, so the engine checks that *some*
                // approval decision is `Approved` against a
                // `PolicyException` requirement for the active
                // candidate. Richer mapping lands with the executor
                // in 0B.
                let backed = ctx.approval_requirements.values().any(|req| {
                    req.candidate == *fp
                        && matches!(req.category, ApprovalCategory::PolicyException)
                }) && ctx
                    .approvals
                    .values()
                    .any(|d| *d == ApprovalDecision::Approved);
                if !backed {
                    blockers.push(TransitionBlocker::UnbackedException(obligation.id));
                }
            }
            ObligationStatus::NotEvaluated => {
                blockers.push(TransitionBlocker::MissingObligation(obligation.id));
            }
            ObligationStatus::Fail => {
                blockers.push(TransitionBlocker::FailedObligation(obligation.id));
            }
            ObligationStatus::RequiresReview => {
                blockers.push(TransitionBlocker::ReviewRequired(obligation.id));
            }
        }
    }
}

fn check_approvals(
    requirements: &TransitionRequirements,
    active_candidate: Option<&CandidateFingerprint>,
    ctx: &TransitionContext<'_>,
    blockers: &mut Vec<TransitionBlocker>,
) {
    if requirements.approvals.is_empty() {
        return;
    }
    let Some(fp) = active_candidate else {
        for _ in requirements.approvals {
            blockers.push(TransitionBlocker::MissingApproval(sentinel_approval_id()));
        }
        return;
    };

    for category in requirements.approvals {
        let requirement = ctx
            .approval_requirements
            .values()
            .find(|req| req.candidate == *fp && &req.category == category);

        let Some(req) = requirement else {
            blockers.push(TransitionBlocker::MissingApproval(sentinel_approval_id()));
            continue;
        };

        match ctx.approvals.get(&req.id) {
            Some(ApprovalDecision::Approved) => {}
            _ => blockers.push(TransitionBlocker::MissingApproval(req.id)),
        }
    }
}
