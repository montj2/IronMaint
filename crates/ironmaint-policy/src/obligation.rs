//! Obligations — sourced policy assertions (§§34–35).
//!
//! An [`Obligation`] is "this authority, in section X, says the
//! package must satisfy Y, and here's the evidence we've gathered
//! to support the claim". The §35 invariant — "a mandatory
//! applicable obligation with `Fail`, `RequiresReview`, or
//! `NotEvaluated` must block the transition" — is enforced by the
//! state machine, not by [`Obligation`] itself. [`ObligationSet`]
//! surfaces the violation as a list for the engine to consume.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use ironmaint_core::{CandidateFingerprint, EvidenceId, ObligationId, SchemaVersion};

use crate::policy::PolicyReference;

/// How strongly an obligation binds (§34).
///
/// `Mandatory` is the §35-invariant trigger. The rest are advisory;
/// `Recommended` / `BestPractice` / `Procedural` / `LegalReview` /
/// `LocalPolicy` carry distinct normative weights that adapters
/// may rank, but none of them block on `Fail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStrength {
    Mandatory,
    Recommended,
    BestPractice,
    Procedural,
    LegalReview,
    LocalPolicy,
}

impl fmt::Display for ObligationStrength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl ObligationStrength {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mandatory => "mandatory",
            Self::Recommended => "recommended",
            Self::BestPractice => "best_practice",
            Self::Procedural => "procedural",
            Self::LegalReview => "legal_review",
            Self::LocalPolicy => "local_policy",
        }
    }
}

/// Whether an obligation applies to a given candidate (§34).
///
/// Three-valued: `Unknown` is the pre-evaluation default, `Applicable`
/// counts toward §35, `NotApplicable` exempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    Unknown,
    Applicable,
    NotApplicable,
}

/// Status of an obligation's evaluation (§34).
///
/// `ExceptionApproved` is valid only if an explicit approval record
/// exists (§35). The state machine checks for that record when it
/// evaluates the obligation; without it, `ExceptionApproved` is
/// treated as `Fail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStatus {
    NotEvaluated,
    Pass,
    Fail,
    RequiresReview,
    ExceptionApproved,
}

impl ObligationStatus {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NotEvaluated => "not_evaluated",
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::RequiresReview => "requires_review",
            Self::ExceptionApproved => "exception_approved",
        }
    }
}

/// A sourced policy assertion bound to a candidate (§34).
///
/// The `evidence` field holds opaque [`EvidenceId`]s — the policy
/// crate intentionally does NOT depend on `ironmaint-evidence` and
/// never inspects evidence values. Linking evidence to obligations
/// is a runtime concern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Obligation {
    pub id: ObligationId,
    pub candidate: CandidateFingerprint,
    pub reference: PolicyReference,
    pub strength: ObligationStrength,
    pub applicability: Applicability,
    /// Human-readable assertion text. Capped at 1 KiB.
    pub requirement: String,
    pub status: ObligationStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceId>,
    /// Wire-format schema version (§60). Always serializes as
    /// `SchemaVersion::V1`; on parse, a missing field defaults to `V1`.
    #[serde(default)]
    pub schema_version: SchemaVersion,
}

const REQUIREMENT_MAX: usize = 1024;

impl Obligation {
    /// Construct an [`Obligation`] in the `NotEvaluated` state.
    ///
    /// # Errors
    /// Returns `Err(&'static str)` if `requirement` exceeds
    /// [`REQUIREMENT_MAX`] bytes.
    pub fn new(
        candidate: CandidateFingerprint,
        reference: PolicyReference,
        strength: ObligationStrength,
        applicability: Applicability,
        requirement: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let r = requirement.into();
        if r.len() > REQUIREMENT_MAX {
            return Err("obligation requirement exceeds 1024 bytes");
        }
        Ok(Self {
            id: ObligationId::new(),
            candidate,
            reference,
            strength,
            applicability,
            requirement: r,
            status: ObligationStatus::NotEvaluated,
            evidence: Vec::new(),
            schema_version: SchemaVersion::default(),
        })
    }

    #[must_use]
    pub fn with_status(mut self, status: ObligationStatus) -> Self {
        self.status = status;
        self
    }

    #[must_use]
    pub fn with_evidence(mut self, id: EvidenceId) -> Self {
        self.evidence.push(id);
        self
    }

    /// Whether this obligation triggers the §35 invariant — i.e.
    /// a Mandatory+Applicable obligation with a non-passing status.
    ///
    /// `ExceptionApproved` is treated as passing here; the
    /// existence of a corresponding approval record is checked
    /// separately by the state machine.
    #[must_use]
    pub fn triggers_section_35_invariant(&self) -> bool {
        self.strength == ObligationStrength::Mandatory
            && self.applicability == Applicability::Applicable
            && !matches!(
                self.status,
                ObligationStatus::Pass | ObligationStatus::ExceptionApproved,
            )
    }
}

impl fmt::Display for ObligationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for Applicability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown"),
            Self::Applicable => f.write_str("applicable"),
            Self::NotApplicable => f.write_str("not_applicable"),
        }
    }
}

/// A [`BTreeMap`]-backed set of [`Obligation`]s keyed by id.
///
/// The state machine consumes one of these per transition. The
/// `mandatory_applicable_unmet` helper is the §35 invariant surface:
/// any obligation it returns must block the transition.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ObligationSet {
    obligations: BTreeMap<ObligationId, Obligation>,
}

impl ObligationSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace an obligation by id.
    pub fn insert(&mut self, obligation: Obligation) {
        self.obligations.insert(obligation.id, obligation);
    }

    #[must_use]
    pub fn get(&self, id: ObligationId) -> Option<&Obligation> {
        self.obligations.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ObligationId, &Obligation)> {
        self.obligations.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.obligations.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.obligations.is_empty()
    }

    /// Mandatory+Applicable obligations with a non-passing status —
    /// the §35 invariant surface (§35: "must block any transition
    /// requiring policy completion").
    pub fn mandatory_applicable_unmet(&self) -> Vec<&Obligation> {
        self.obligations
            .values()
            .filter(|o| o.triggers_section_35_invariant())
            .collect()
    }

    /// All obligations that have an `ExceptionApproved` status —
    /// used by the state machine to verify that the approval
    /// record exists before treating the obligation as passing.
    pub fn exception_approved(&self) -> Vec<&Obligation> {
        self.obligations
            .values()
            .filter(|o| o.status == ObligationStatus::ExceptionApproved)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::Authority;
    use crate::authority::AuthorityClassification;
    use ironmaint_core::{AuthorityId, CandidateFingerprint, DistributionFamily};

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    fn reference() -> PolicyReference {
        let _ = AuthorityId::new(); // keep AuthorityId used (referenced by reference's API)
        PolicyReference::new(AuthorityId::new())
    }

    #[test]
    fn strength_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ObligationStrength::LegalReview).unwrap(),
            r#""legal_review""#
        );
    }

    #[test]
    fn applicability_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&Applicability::NotApplicable).unwrap(),
            r#""not_applicable""#
        );
    }

    #[test]
    fn status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ObligationStatus::ExceptionApproved).unwrap(),
            r#""exception_approved""#
        );
    }

    #[test]
    fn obligation_minimum_construction() {
        let o = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "package must build reproducibly",
        )
        .unwrap();
        assert_eq!(o.status, ObligationStatus::NotEvaluated);
        assert!(o.evidence.is_empty());
    }

    #[test]
    fn obligation_rejects_oversize_requirement() {
        let long = "x".repeat(REQUIREMENT_MAX + 1);
        let err = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            long,
        )
        .unwrap_err();
        assert!(err.contains("1024"));
    }

    #[test]
    fn obligation_with_status_and_evidence() {
        let o = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "package must build reproducibly",
        )
        .unwrap()
        .with_status(ObligationStatus::Pass)
        .with_evidence(EvidenceId::new())
        .with_evidence(EvidenceId::new());
        assert_eq!(o.status, ObligationStatus::Pass);
        assert_eq!(o.evidence.len(), 2);
    }

    #[test]
    fn section_35_invariant_predicate() {
        let base = || {
            Obligation::new(
                fp(),
                reference(),
                ObligationStrength::Mandatory,
                Applicability::Applicable,
                "x",
            )
            .unwrap()
        };
        assert!(
            base()
                .clone()
                .with_status(ObligationStatus::NotEvaluated)
                .triggers_section_35_invariant()
        );
        assert!(
            base()
                .clone()
                .with_status(ObligationStatus::Fail)
                .triggers_section_35_invariant()
        );
        assert!(
            base()
                .clone()
                .with_status(ObligationStatus::RequiresReview)
                .triggers_section_35_invariant()
        );
        assert!(
            !base()
                .clone()
                .with_status(ObligationStatus::Pass)
                .triggers_section_35_invariant()
        );
        assert!(
            !base()
                .clone()
                .with_status(ObligationStatus::ExceptionApproved)
                .triggers_section_35_invariant()
        );
    }

    #[test]
    fn recommended_obligation_does_not_trigger_section_35() {
        let o = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Recommended,
            Applicability::Applicable,
            "x",
        )
        .unwrap()
        .with_status(ObligationStatus::Fail);
        assert!(!o.triggers_section_35_invariant());
    }

    #[test]
    fn not_applicable_obligation_does_not_trigger_section_35() {
        let o = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::NotApplicable,
            "x",
        )
        .unwrap()
        .with_status(ObligationStatus::Fail);
        assert!(!o.triggers_section_35_invariant());
    }

    #[test]
    fn obligation_round_trips() {
        let o = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "x",
        )
        .unwrap()
        .with_status(ObligationStatus::Pass);
        let json = serde_json::to_string(&o).unwrap();
        let parsed: Obligation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, o);
    }

    #[test]
    fn obligation_set_insert_and_iter() {
        let mut s = ObligationSet::new();
        let o = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "x",
        )
        .unwrap();
        s.insert(o.clone());
        assert_eq!(s.len(), 1);
        assert!(!s.is_empty());
        assert_eq!(s.get(o.id).unwrap().id, o.id);
    }

    #[test]
    fn obligation_set_mandatory_applicable_unmet() {
        let mut s = ObligationSet::new();
        let passing = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "passing",
        )
        .unwrap()
        .with_status(ObligationStatus::Pass);
        let failing = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "failing",
        )
        .unwrap()
        .with_status(ObligationStatus::Fail);
        let recommended_failing = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Recommended,
            Applicability::Applicable,
            "recommended",
        )
        .unwrap()
        .with_status(ObligationStatus::Fail);
        s.insert(passing);
        s.insert(failing.clone());
        s.insert(recommended_failing);

        let unmet = s.mandatory_applicable_unmet();
        assert_eq!(unmet.len(), 1);
        assert_eq!(unmet[0].id, failing.id);
    }

    #[test]
    fn obligation_set_exception_approved_listed() {
        let mut s = ObligationSet::new();
        let ex = Obligation::new(
            fp(),
            reference(),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "exception",
        )
        .unwrap()
        .with_status(ObligationStatus::ExceptionApproved);
        s.insert(ex.clone());
        let listed = s.exception_approved();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, ex.id);
    }

    // Touch `Authority` so the unused-import warning is silenced.
    #[test]
    fn authority_helper_constructs_normative_authority() {
        let _ = Authority::new(
            Some(DistributionFamily::new("debian").unwrap()),
            "Debian Policy",
            AuthorityClassification::NormativePolicy,
        );
    }
}
