//! Gates — deterministic pre-conditions over evidence (§§27–29).
//!
//! A [`GateDefinition`] describes "what evidence must be true for a
//! transition to be allowed"; a [`GateResult`] is the record of an
//! evaluation. The state machine in `ironmaint-state` consumes both
//! types via its `TransitionContext`.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::{CandidateFingerprint, EvidenceId, GateId};

use crate::evidence::EvidenceKind;

/// Workflow stage a gate is anchored to.
///
/// One variant per non-exceptional [`ironmaint_core::JobState`] stage
/// plus `Publication` (the post-Approved segment). Maps 1:1 to the
/// edges in PHASE-0A.md §20; the state machine's `TransitionRule`
/// table references these stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateStage {
    SourcePreparation,
    SourceAnalysis,
    IssueAnalysis,
    Maintenance,
    PolicyEvaluation,
    BuildValidation,
    PackageQa,
    FunctionalValidation,
    UpgradeValidation,
    ReleaseReview,
    CandidateAssembly,
    FinalValidation,
    Publication,
}

impl fmt::Display for GateStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl GateStage {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SourcePreparation => "source_preparation",
            Self::SourceAnalysis => "source_analysis",
            Self::IssueAnalysis => "issue_analysis",
            Self::Maintenance => "maintenance",
            Self::PolicyEvaluation => "policy_evaluation",
            Self::BuildValidation => "build_validation",
            Self::PackageQa => "package_qa",
            Self::FunctionalValidation => "functional_validation",
            Self::UpgradeValidation => "upgrade_validation",
            Self::ReleaseReview => "release_review",
            Self::CandidateAssembly => "candidate_assembly",
            Self::FinalValidation => "final_validation",
            Self::Publication => "publication",
        }
    }
}

/// What a [`GateRequirement`] is willing to accept (§28).
///
/// `Pass` is strict — `NotApplicable` does NOT satisfy. `PassOrNotApplicable`
/// is the "this check is meaningful only for some candidates" variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequiredEvidenceStatus {
    Pass,
    PassOrNotApplicable,
}

impl fmt::Display for RequiredEvidenceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => f.write_str("pass"),
            Self::PassOrNotApplicable => f.write_str("pass_or_not_applicable"),
        }
    }
}

/// Status of a gate evaluation (§29).
///
/// `NotEvaluated` is the initial state (no evaluation has run).
/// `Blocked` means the evaluation couldn't complete (e.g. dependency
/// missing). `ReviewRequired` means the evaluation ran but a human
/// needs to weigh in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// Initial state: no evaluation has run yet.
    NotEvaluated,
    /// Evaluation succeeded; gate is satisfied.
    Pass,
    /// Evaluation ran and found the gate violated.
    Fail,
    /// Evaluation found the gate not applicable to this candidate.
    NotApplicable,
    /// Evaluation couldn't complete (missing dependency, tool error).
    Blocked,
    /// Evaluation ran but requires a human decision.
    ReviewRequired,
}

impl GateStatus {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NotEvaluated => "not_evaluated",
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::NotApplicable => "not_applicable",
            Self::Blocked => "blocked",
            Self::ReviewRequired => "review_required",
        }
    }
}

impl fmt::Display for GateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What evidence a gate is asking for (§28).
///
/// A gate does NOT invoke tools — it describes the pre-condition.
/// Adapters and the policy engine produce evidence; the gate is
/// satisfied by the existence of a [`GateResult`] whose status meets
/// `minimum_status` and whose `candidate` matches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct GateRequirement {
    pub evidence_kind: EvidenceKind,
    pub minimum_status: RequiredEvidenceStatus,
}

impl GateRequirement {
    #[must_use]
    pub fn new(evidence_kind: EvidenceKind, minimum_status: RequiredEvidenceStatus) -> Self {
        Self {
            evidence_kind,
            minimum_status,
        }
    }
}

/// A declared pre-condition on a candidate (§27).
///
/// A gate is "mandatory" when its result blocks transitions if not
/// satisfied; non-mandatory gates are advisory.
///
/// - **Represents**: a *declaration* of what must be true before a
///   candidate may advance — the evidence kind required, the
///   minimum acceptable status, and the gate stage it gates.
/// - **Mutation ownership**: gate evaluations are produced only by
///   the deterministic gate-evaluation logic in §29; the definition
///   itself is a plan, not a result. Creating a `GateDefinition`
///   does not produce any `GateResult`.
/// - **Invariants**: `GateDefinition` describes the pre-condition;
///   it does not invoke tools. The mandatory/non-mandatory bit is
///   the bridge between gate evaluation and the transition engine:
///   when a mandatory gate lacks a passing `GateResult`, the
///   transition is blocked.
/// - **Does NOT represent**: NOT a tool invocation, NOT an evidence
///   record, NOT a workflow state. A gate is the *question*; the
///   matching `GateResult` is the *answer*.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct GateDefinition {
    pub id: GateId,
    pub candidate: CandidateFingerprint,
    pub stage: GateStage,
    pub requirement: GateRequirement,
    pub mandatory: bool,
}

impl GateDefinition {
    #[must_use]
    pub fn new(
        candidate: CandidateFingerprint,
        stage: GateStage,
        requirement: GateRequirement,
        mandatory: bool,
    ) -> Self {
        Self {
            id: GateId::new(),
            candidate,
            stage,
            requirement,
            mandatory,
        }
    }
}

/// The record of a gate evaluation (§29).
///
/// Created only by deterministic gate-evaluation logic; never directly
/// by adapters. Carries the [`EvidenceId`]s that justify the status,
/// and the candidate fingerprint so the state machine can verify it
/// matches the active candidate before consulting this result.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct GateResult {
    pub gate_id: GateId,
    pub candidate: CandidateFingerprint,
    pub status: GateStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceId>,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "ironmaint_core::json_schema_impls::Rfc3339DateTime")]
    pub evaluated_at: OffsetDateTime,
}

impl GateResult {
    /// Build a `Pass` result with a candidate and evaluation time.
    #[must_use]
    pub fn pass(
        gate_id: GateId,
        candidate: CandidateFingerprint,
        evidence: Vec<EvidenceId>,
        evaluated_at: OffsetDateTime,
    ) -> Self {
        Self {
            gate_id,
            candidate,
            status: GateStatus::Pass,
            evidence,
            evaluated_at,
        }
    }

    /// Build a `Fail` result.
    #[must_use]
    pub fn fail(
        gate_id: GateId,
        candidate: CandidateFingerprint,
        evidence: Vec<EvidenceId>,
        evaluated_at: OffsetDateTime,
    ) -> Self {
        Self {
            gate_id,
            candidate,
            status: GateStatus::Fail,
            evidence,
            evaluated_at,
        }
    }

    /// Build a `NotApplicable` result.
    #[must_use]
    pub fn not_applicable(
        gate_id: GateId,
        candidate: CandidateFingerprint,
        evaluated_at: OffsetDateTime,
    ) -> Self {
        Self {
            gate_id,
            candidate,
            status: GateStatus::NotApplicable,
            evidence: Vec::new(),
            evaluated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    #[test]
    fn gate_stage_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&GateStage::SourcePreparation).unwrap(),
            r#""source_preparation""#
        );
        assert_eq!(
            serde_json::to_string(&GateStage::CandidateAssembly).unwrap(),
            r#""candidate_assembly""#
        );
    }

    #[test]
    fn required_status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&RequiredEvidenceStatus::PassOrNotApplicable).unwrap(),
            r#""pass_or_not_applicable""#
        );
    }

    #[test]
    fn gate_status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&GateStatus::ReviewRequired).unwrap(),
            r#""review_required""#
        );
    }

    #[test]
    fn gate_definition_new_mints_id_and_keeps_mandatory_flag() {
        let def = GateDefinition::new(
            fp(),
            GateStage::BuildValidation,
            GateRequirement::new(EvidenceKind::Build, RequiredEvidenceStatus::Pass),
            true,
        );
        assert!(def.mandatory);
        assert_eq!(def.stage, GateStage::BuildValidation);
    }

    #[test]
    fn gate_definition_round_trips() {
        let def = GateDefinition::new(
            fp(),
            GateStage::BuildValidation,
            GateRequirement::new(EvidenceKind::Build, RequiredEvidenceStatus::Pass),
            true,
        );
        let json = serde_json::to_string(&def).unwrap();
        let parsed: GateDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, def);
    }

    #[test]
    fn gate_result_pass_factory() {
        let r = GateResult::pass(
            GateId::new(),
            fp(),
            vec![EvidenceId::new()],
            datetime!(2026-01-01 00:00:00 UTC),
        );
        assert_eq!(r.status, GateStatus::Pass);
        assert_eq!(r.evidence.len(), 1);
    }

    #[test]
    fn gate_result_fail_factory() {
        let r = GateResult::fail(
            GateId::new(),
            fp(),
            Vec::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        assert_eq!(r.status, GateStatus::Fail);
    }

    #[test]
    fn gate_result_not_applicable_factory() {
        let r = GateResult::not_applicable(GateId::new(), fp(), datetime!(2026-01-01 00:00:00 UTC));
        assert_eq!(r.status, GateStatus::NotApplicable);
        assert!(r.evidence.is_empty());
    }

    #[test]
    fn gate_result_round_trips() {
        let r = GateResult::pass(
            GateId::new(),
            fp(),
            vec![EvidenceId::new(), EvidenceId::new()],
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&r).unwrap();
        let parsed: GateResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn gate_result_serializes_rfc3339() {
        let r = GateResult::pass(
            GateId::new(),
            fp(),
            Vec::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"2026-01-01T00:00:00Z\""), "got: {json}");
    }
}
