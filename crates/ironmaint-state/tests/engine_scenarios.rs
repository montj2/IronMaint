//! §72 scenario matrix for [`ironmaint_state::TransitionEngine`].
//!
//! Each test names a specific scenario from PHASE-0A.md §72 and
//! asserts the engine's decision. Shared fixtures live in
//! `tests/fixtures.rs`; the `with_empty_ctx!` macro binds empty
//! `BTreeMap` / `ObligationSet` locals so the references inside
//! [`TransitionContext`] stay valid for the closure body.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::map_err_ignore
)]

use std::collections::BTreeMap;

use ironmaint_core::{AuthorityId, CandidateFingerprint, GateId, JobId, JobState, ObligationId};
use ironmaint_evidence::{
    GateDefinition, GateRequirement, GateResult, GateStage, GateStatus, RequiredEvidenceStatus,
};
use ironmaint_policy::{
    Applicability, ApprovalCategory, ApprovalDecision, ApprovalRequirement, Obligation,
    ObligationSet, ObligationStatus, ObligationStrength, PolicyReference,
};
use ironmaint_state::{
    ResumeRecord, Transition, TransitionApplyError, TransitionBlocker, TransitionContext,
    TransitionDecision, TransitionEngine, TransitionRequest,
};
use time::macros::datetime;

mod fixtures;

use fixtures::{fp, job};

/// Bind empty `BTreeMap` / `ObligationSet` locals and a
/// [`TransitionContext`] referencing them, then run `$body` with
/// the context bound as `$ctx`. The locals live in the caller's
/// scope, so the references stay valid for the duration of the
/// closure body.
macro_rules! with_empty_ctx {
    ($proj:expr, $fp:expr, |$ctx:ident| $body:block) => {{
        let gates = BTreeMap::<GateId, GateResult>::new();
        let gate_definitions = BTreeMap::<GateId, GateDefinition>::new();
        let obligations = ObligationSet::new();
        let approval_requirements =
            BTreeMap::<ironmaint_core::ApprovalId, ApprovalRequirement>::new();
        let approvals = BTreeMap::<ironmaint_core::ApprovalId, ApprovalDecision>::new();
        let $ctx = TransitionContext {
            current: $proj,
            active_candidate_fingerprint: $fp,
            gates: &gates,
            gate_definitions: &gate_definitions,
            obligations: &obligations,
            approval_requirements: &approval_requirements,
            approvals: &approvals,
            infrastructure_blocked: false,
            resume_event: None,
        };
        $body
    }};
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.1 — Valid progression: EventDetected → Intake allowed.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_event_detected_to_intake_allowed() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::EventDetected, 1);

    let (gate_definitions, gates) =
        make_gate_pair(&fp, GateStage::SourcePreparation, GateStatus::Pass);

    with_empty_ctx!(&proj, Some(fp.clone()), |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 1,
            target: JobState::Intake,
            now: datetime!(2026-01-02 00:00:00 UTC),
        };
        // Replace the empty gates/gate_definitions in the macro's ctx with the
        // fixture-defined ones for this test.
        let ctx = TransitionContext {
            current: ctx.current,
            active_candidate_fingerprint: ctx.active_candidate_fingerprint.clone(),
            gates: &gates,
            gate_definitions: &gate_definitions,
            obligations: ctx.obligations,
            approval_requirements: ctx.approval_requirements,
            approvals: ctx.approvals,
            infrastructure_blocked: ctx.infrastructure_blocked,
            resume_event: ctx.resume_event,
        };
        match engine.evaluate(&request, &ctx) {
            TransitionDecision::Allowed(t) => {
                assert_eq!(t.from, JobState::EventDetected);
                assert_eq!(t.to, JobState::Intake);
            }
            TransitionDecision::Blocked(b) => panic!("expected Allowed, got Blocked: {b:?}"),
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.2 — Invalid skip: Intake → BuildValidation blocked (not in §20 graph).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_invalid_skip_intake_to_build_blocked() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::Intake, 1);

    with_empty_ctx!(&proj, Some(fp), |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 1,
            target: JobState::BuildValidation,
            now: datetime!(2026-01-02 00:00:00 UTC),
        };
        match engine.evaluate(&request, &ctx) {
            TransitionDecision::Blocked(blockers) => {
                assert!(
                    blockers
                        .iter()
                        .any(|b| matches!(b, TransitionBlocker::InvalidStatePath)),
                    "expected InvalidStatePath in {blockers:?}",
                );
            }
            TransitionDecision::Allowed(_) => panic!("expected Blocked"),
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.3 — Failed mandatory gate prevents progression.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_failed_mandatory_gate_blocks() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::SourceIntegrity, 1);

    let (gate_definitions, gates) =
        make_gate_pair(&fp, GateStage::BuildValidation, GateStatus::Fail);

    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::BuildValidation,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    match engine.evaluate(&request, &ctx) {
        TransitionDecision::Blocked(blockers) => {
            assert!(
                blockers
                    .iter()
                    .any(|b| matches!(b, TransitionBlocker::FailedGate(_))),
                "expected FailedGate in {blockers:?}",
            );
        }
        TransitionDecision::Allowed(_) => panic!("expected Blocked"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.4 — Missing mandatory gate (no result at all) prevents progression.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_missing_mandatory_gate_blocks() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::SourceIntegrity, 1);

    let def = GateDefinition::new(
        fp.clone(),
        GateStage::BuildValidation,
        GateRequirement::new(
            ironmaint_evidence::EvidenceKind::Build,
            RequiredEvidenceStatus::Pass,
        ),
        true,
    );
    let gate_id = def.id;
    let mut gate_definitions = BTreeMap::new();
    gate_definitions.insert(gate_id, def);
    let gates = BTreeMap::<GateId, GateResult>::new();

    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::BuildValidation,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    match engine.evaluate(&request, &ctx) {
        TransitionDecision::Blocked(blockers) => {
            assert!(
                blockers
                    .iter()
                    .any(|b| matches!(b, TransitionBlocker::MissingGate(_))),
                "expected MissingGate in {blockers:?}",
            );
        }
        TransitionDecision::Allowed(_) => panic!("expected Blocked"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.5 — Not-applicable gate with PassOrNotApplicable requirement is allowed.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_not_applicable_gate_with_pass_or_not_applicable_allowed() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::SourceIntegrity, 1);

    let def = GateDefinition::new(
        fp.clone(),
        GateStage::BuildValidation,
        GateRequirement::new(
            ironmaint_evidence::EvidenceKind::Build,
            RequiredEvidenceStatus::PassOrNotApplicable,
        ),
        true,
    );
    let gate_id = def.id;
    let mut gate_definitions = BTreeMap::new();
    gate_definitions.insert(gate_id, def);
    let mut gates = BTreeMap::new();
    gates.insert(
        gate_id,
        GateResult::not_applicable(gate_id, fp.clone(), datetime!(2026-01-01 00:00:00 UTC)),
    );

    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::BuildValidation,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    assert!(matches!(
        engine.evaluate(&request, &ctx),
        TransitionDecision::Allowed(_)
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.6 — Candidate mismatch: Evidence bound to C1 cannot satisfy gate for C2.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_candidate_mismatch_blocks() {
    let fp_active = fp();
    let fp_other = CandidateFingerprint::from_hex("b".repeat(64)).unwrap();
    let engine = TransitionEngine::new();
    let proj = job(JobState::SourceIntegrity, 1);

    let def = GateDefinition::new(
        fp_active.clone(),
        GateStage::BuildValidation,
        GateRequirement::new(
            ironmaint_evidence::EvidenceKind::Build,
            RequiredEvidenceStatus::Pass,
        ),
        true,
    );
    let gate_id = def.id;
    let mut gate_definitions = BTreeMap::new();
    gate_definitions.insert(gate_id, def);
    let mut gates = BTreeMap::new();
    gates.insert(
        gate_id,
        GateResult::pass(
            gate_id,
            fp_other,
            vec![],
            datetime!(2026-01-01 00:00:00 UTC),
        ),
    );

    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp_active),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::BuildValidation,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    match engine.evaluate(&request, &ctx) {
        TransitionDecision::Blocked(blockers) => {
            assert!(
                blockers
                    .iter()
                    .any(|b| matches!(b, TransitionBlocker::CandidateMismatch { .. })),
                "expected CandidateMismatch in {blockers:?}",
            );
        }
        TransitionDecision::Allowed(_) => panic!("expected Blocked"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.7 — Mandatory obligation, status = Fail blocks policy completion.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_failed_mandatory_obligation_blocks() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::FinalValidation, 1);

    let (gate_definitions, gates) =
        make_gate_pair(&fp, GateStage::FinalValidation, GateStatus::Pass);

    let failing = Obligation::new(
        fp.clone(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "policy complete",
    )
    .unwrap()
    .with_status(ObligationStatus::Fail);
    let failing_id: ObligationId = failing.id;

    let mut obligations = ObligationSet::new();
    obligations.insert(failing);

    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::ReadyForApproval,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    match engine.evaluate(&request, &ctx) {
        TransitionDecision::Blocked(blockers) => {
            assert!(
                blockers.iter().any(|b| matches!(
                    b,
                    TransitionBlocker::FailedObligation(id) if *id == failing_id
                )),
                "expected FailedObligation({failing_id:?}) in {blockers:?}",
            );
        }
        TransitionDecision::Allowed(_) => panic!("expected Blocked"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.8 — Recommended obligation, status = Fail does NOT block.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_failed_recommended_obligation_allowed() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::FinalValidation, 1);

    let (gate_definitions, gates) =
        make_gate_pair(&fp, GateStage::FinalValidation, GateStatus::Pass);

    let recommended = Obligation::new(
        fp.clone(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Recommended,
        Applicability::Applicable,
        "recommended",
    )
    .unwrap()
    .with_status(ObligationStatus::Fail);

    let mut obligations = ObligationSet::new();
    obligations.insert(recommended);

    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::ReadyForApproval,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    assert!(matches!(
        engine.evaluate(&request, &ctx),
        TransitionDecision::Allowed(_)
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.9 — Required human approval with no decision blocks progression.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_required_human_review_missing_blocks() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::ReadyForApproval, 1);

    with_empty_ctx!(&proj, Some(fp), |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 1,
            target: JobState::Approved,
            now: datetime!(2026-01-02 00:00:00 UTC),
        };
        match engine.evaluate(&request, &ctx) {
            TransitionDecision::Blocked(blockers) => {
                assert!(
                    blockers
                        .iter()
                        .any(|b| matches!(b, TransitionBlocker::MissingApproval(_))),
                    "expected MissingApproval in {blockers:?}",
                );
            }
            TransitionDecision::Allowed(_) => panic!("expected Blocked"),
        }
    });
}

#[test]
fn s72_required_human_review_recorded_allows() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::ReadyForApproval, 1);

    let req =
        ApprovalRequirement::new(fp.clone(), ApprovalCategory::HumanReview, "review").unwrap();
    let mut approval_requirements = BTreeMap::new();
    approval_requirements.insert(req.id, req.clone());
    let mut approvals = BTreeMap::new();
    approvals.insert(req.id, ApprovalDecision::Approved);

    let gates = BTreeMap::<GateId, GateResult>::new();
    let gate_definitions = BTreeMap::<GateId, GateDefinition>::new();
    let obligations = ObligationSet::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::Approved,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    assert!(matches!(
        engine.evaluate(&request, &ctx),
        TransitionDecision::Allowed(_)
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.10 — Infrastructure blocked: target = Approved is blocked.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_infrastructure_blocked_blocks_normal_target() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::ReadyForApproval, 1);

    let gates = BTreeMap::<GateId, GateResult>::new();
    let gate_definitions = BTreeMap::<GateId, GateDefinition>::new();
    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: true,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::Approved,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    match engine.evaluate(&request, &ctx) {
        TransitionDecision::Blocked(blockers) => {
            assert!(
                blockers
                    .iter()
                    .any(|b| matches!(b, TransitionBlocker::InfrastructureBlocked)),
                "expected InfrastructureBlocked in {blockers:?}",
            );
        }
        TransitionDecision::Allowed(_) => panic!("expected Blocked"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.11 — Infrastructure blocked: target = InfrastructureBlocked allowed.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_infrastructure_blocked_target_allows() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::ReadyForApproval, 1);

    let gates = BTreeMap::<GateId, GateResult>::new();
    let gate_definitions = BTreeMap::<GateId, GateDefinition>::new();
    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: true,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::InfrastructureBlocked,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    assert!(matches!(
        engine.evaluate(&request, &ctx),
        TransitionDecision::Allowed(_)
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.12 — Human review resume: a recorded ResumeRecord allows transition
// out of HumanReviewRequired back to a normal state.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_human_review_resume_with_record_allows() {
    let engine = TransitionEngine::new();
    let proj = job(JobState::HumanReviewRequired, 5);
    let resume = ResumeRecord::new(
        JobState::HumanReviewRequired,
        JobState::SourceReview,
        datetime!(2026-03-01 00:00:00 UTC),
    );

    let gates = BTreeMap::<GateId, GateResult>::new();
    let gate_definitions = BTreeMap::<GateId, GateDefinition>::new();
    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: None,
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: Some(&resume),
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 5,
        target: JobState::SourceReview,
        now: datetime!(2026-03-01 00:00:01 UTC),
    };
    match engine.evaluate(&request, &ctx) {
        TransitionDecision::Allowed(Transition { from, to, .. }) => {
            assert_eq!(from, JobState::HumanReviewRequired);
            assert_eq!(to, JobState::SourceReview);
        }
        TransitionDecision::Blocked(b) => panic!("expected Allowed, got Blocked: {b:?}"),
    }
}

#[test]
fn s72_human_review_resume_without_record_blocks() {
    let engine = TransitionEngine::new();
    let proj = job(JobState::HumanReviewRequired, 5);

    with_empty_ctx!(&proj, None, |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 5,
            target: JobState::SourceReview,
            now: datetime!(2026-03-01 00:00:01 UTC),
        };
        assert!(matches!(
            engine.evaluate(&request, &ctx),
            TransitionDecision::Blocked(_)
        ));
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.13 — Stale projection: version mismatch is rejected.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_stale_projection_blocks() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::EventDetected, 3);

    with_empty_ctx!(&proj, Some(fp), |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 1,
            target: JobState::Intake,
            now: datetime!(2026-01-02 00:00:00 UTC),
        };
        match engine.evaluate(&request, &ctx) {
            TransitionDecision::Blocked(blockers) => {
                assert!(
                    blockers
                        .iter()
                        .any(|b| matches!(b, TransitionBlocker::StaleProjection { .. })),
                    "expected StaleProjection in {blockers:?}",
                );
            }
            TransitionDecision::Allowed(_) => panic!("expected Blocked"),
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// §72.14 — Terminal state: nothing transitions out of Published/Cancelled.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_terminal_published_blocks_all() {
    let engine = TransitionEngine::new();
    let proj = job(JobState::Published, 10);

    with_empty_ctx!(&proj, None, |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 10,
            target: JobState::Intake,
            now: datetime!(2026-04-01 00:00:00 UTC),
        };
        assert!(matches!(
            engine.evaluate(&request, &ctx),
            TransitionDecision::Blocked(_)
        ));
    });
}

#[test]
fn s72_terminal_cancelled_blocks_all() {
    let engine = TransitionEngine::new();
    let proj = job(JobState::Cancelled, 8);

    with_empty_ctx!(&proj, None, |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 8,
            target: JobState::HumanReviewRequired,
            now: datetime!(2026-04-01 00:00:00 UTC),
        };
        assert!(matches!(
            engine.evaluate(&request, &ctx),
            TransitionDecision::Blocked(_)
        ));
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Exceptional target — any non-terminal state → HumanReviewRequired allowed.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_any_state_to_human_review_allowed() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::BuildValidation, 2);

    with_empty_ctx!(&proj, Some(fp), |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 2,
            target: JobState::HumanReviewRequired,
            now: datetime!(2026-02-15 00:00:00 UTC),
        };
        assert!(matches!(
            engine.evaluate(&request, &ctx),
            TransitionDecision::Allowed(_)
        ));
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Cancelled — any non-terminal state → Cancelled allowed (terminal).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_any_state_to_cancelled_allowed() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::BuildValidation, 2);

    with_empty_ctx!(&proj, Some(fp), |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 2,
            target: JobState::Cancelled,
            now: datetime!(2026-02-15 00:00:00 UTC),
        };
        assert!(matches!(
            engine.evaluate(&request, &ctx),
            TransitionDecision::Allowed(_)
        ));
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// apply — happy-path produces a StateTransitioned event with bumped version.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_apply_emits_state_transitioned_with_bumped_version() {
    let fp = fp();
    let engine = TransitionEngine::new();
    let proj = job(JobState::EventDetected, 1);

    let (gate_definitions, gates) =
        make_gate_pair(&fp, GateStage::SourcePreparation, GateStatus::Pass);
    let obligations = ObligationSet::new();
    let approval_requirements = BTreeMap::new();
    let approvals = BTreeMap::new();
    let ctx = TransitionContext {
        current: &proj,
        active_candidate_fingerprint: Some(fp),
        gates: &gates,
        gate_definitions: &gate_definitions,
        obligations: &obligations,
        approval_requirements: &approval_requirements,
        approvals: &approvals,
        infrastructure_blocked: false,
        resume_event: None,
    };
    let request = TransitionRequest {
        job_id: JobId::new(),
        expected_version: 1,
        target: JobState::Intake,
        now: datetime!(2026-01-02 00:00:00 UTC),
    };
    let event = engine.apply(request, &ctx).unwrap();
    assert_eq!(event.transition.from, JobState::EventDetected);
    assert_eq!(event.transition.to, JobState::Intake);
    assert_eq!(event.projection_after.version, 2);
    assert_eq!(event.projection_after.state, JobState::Intake);
}

// ─────────────────────────────────────────────────────────────────────────────
// apply — Blocked decision is returned as TransitionApplyError::Blocked.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn s72_apply_blocked_returns_error() {
    let engine = TransitionEngine::new();
    let proj = job(JobState::EventDetected, 1);

    with_empty_ctx!(&proj, None, |ctx| {
        let request = TransitionRequest {
            job_id: JobId::new(),
            expected_version: 1,
            target: JobState::Intake,
            now: datetime!(2026-01-02 00:00:00 UTC),
        };
        let err = engine.apply(request, &ctx).unwrap_err();
        assert!(matches!(err, TransitionApplyError::Blocked(_)));
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn make_gate_pair(
    candidate: &CandidateFingerprint,
    stage: GateStage,
    status: GateStatus,
) -> (
    BTreeMap<GateId, GateDefinition>,
    BTreeMap<GateId, GateResult>,
) {
    // Build the definition first; capture its `id` so the result is
    // keyed by the *same* identifier the engine will look up via
    // `GateDefinition::id`. `GateDefinition::new` mints its own id,
    // so we must use `def.id` here — not a fresh `GateId::new()`.
    let def = GateDefinition::new(
        candidate.clone(),
        stage,
        GateRequirement::new(
            ironmaint_evidence::EvidenceKind::Build,
            RequiredEvidenceStatus::Pass,
        ),
        true,
    );
    let gate_id = def.id;
    let mut defs = BTreeMap::new();
    defs.insert(gate_id, def);

    let mut results = BTreeMap::new();
    let result = match status {
        GateStatus::Pass => GateResult::pass(
            gate_id,
            candidate.clone(),
            vec![],
            datetime!(2026-01-01 00:00:00 UTC),
        ),
        GateStatus::Fail => GateResult::fail(
            gate_id,
            candidate.clone(),
            vec![],
            datetime!(2026-01-01 00:00:00 UTC),
        ),
        GateStatus::NotApplicable => GateResult::not_applicable(
            gate_id,
            candidate.clone(),
            datetime!(2026-01-01 00:00:00 UTC),
        ),
        other => panic!("make_gate_pair: unsupported status {other:?}; build a custom helper"),
    };
    results.insert(gate_id, result);
    (defs, results)
}
