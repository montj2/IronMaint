//! Evidence graph — the storage layer for [`Evidence`] and [`GateResult`].
//!
//! Per PHASE-0A.md §30, the graph deliberately does NOT implement
//! cross-candidate evidence reuse. Sophisticated navigation lands in
//! 0A.5 with the conformance suite. For 0A.3 the graph is a
//! candidate-scoped store with the explicit pre-condition that every
//! attached evidence is candidate-bound.

use std::collections::BTreeMap;

use thiserror::Error;

use ironmaint_core::{CandidateFingerprint, EvidenceId, GateId};

use crate::evidence::Evidence;
use crate::gate::GateResult;

/// Errors that can arise when building an [`EvidenceGraph`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EvidenceError {
    /// Evidence is already attached with this id; duplicates are rejected
    /// to keep the graph append-only.
    #[error("evidence `{0}` is already attached")]
    DuplicateEvidence(EvidenceId),
    /// A gate result with this id is already attached.
    #[error("gate result `{0}` is already attached")]
    DuplicateGateResult(GateId),
    /// Evidence's candidate doesn't match the gate result's candidate.
    /// Per §30 the binding is strict in 0A.3.
    #[error("evidence candidate `{evidence}` does not match gate result candidate `{gate_result}`")]
    CandidateMismatch {
        evidence: CandidateFingerprint,
        gate_result: CandidateFingerprint,
    },
}

/// Storage for evidence and gate results keyed by id.
///
/// `BTreeMap` gives deterministic iteration order for snapshotting —
/// important for the conformance suite (0A.5).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EvidenceGraph {
    evidence: BTreeMap<EvidenceId, Evidence>,
    gate_results: BTreeMap<GateId, GateResult>,
}

impl EvidenceGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach an [`Evidence`]. Idempotent on `id`: a duplicate id
    /// returns [`EvidenceError::DuplicateEvidence`] rather than
    /// silently overwriting.
    pub fn attach_evidence(&mut self, ev: Evidence) -> Result<(), EvidenceError> {
        if self.evidence.contains_key(&ev.id) {
            return Err(EvidenceError::DuplicateEvidence(ev.id));
        }
        self.evidence.insert(ev.id, ev);
        Ok(())
    }

    /// Attach a [`GateResult`]. Same idempotency contract as
    /// [`Self::attach_evidence`].
    pub fn attach_gate_result(&mut self, result: GateResult) -> Result<(), EvidenceError> {
        if self.gate_results.contains_key(&result.gate_id) {
            return Err(EvidenceError::DuplicateGateResult(result.gate_id));
        }
        self.gate_results.insert(result.gate_id, result);
        Ok(())
    }

    /// Borrow all evidence attached to this graph.
    pub fn evidence(&self) -> impl Iterator<Item = &Evidence> {
        self.evidence.values()
    }

    /// Borrow all gate results attached to this graph.
    pub fn gate_results(&self) -> impl Iterator<Item = &GateResult> {
        self.gate_results.values()
    }

    /// Look up a single [`GateResult`] by id.
    #[must_use]
    pub fn gate_result(&self, gate: GateId) -> Option<&GateResult> {
        self.gate_results.get(&gate)
    }

    /// Look up a single [`Evidence`] by id.
    #[must_use]
    pub fn evidence_by_id(&self, id: EvidenceId) -> Option<&Evidence> {
        self.evidence.get(&id)
    }

    /// All evidence bound to a specific candidate fingerprint.
    pub fn evidence_for_candidate(
        &self,
        fp: &CandidateFingerprint,
    ) -> impl Iterator<Item = &Evidence> {
        self.evidence.values().filter(move |e| &e.candidate == fp)
    }

    /// Number of attached evidence objects.
    #[must_use]
    pub fn evidence_count(&self) -> usize {
        self.evidence.len()
    }

    /// Number of attached gate results.
    #[must_use]
    pub fn gate_result_count(&self) -> usize {
        self.gate_results.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{EvidenceKind, EvidenceProducer, EvidenceStatus};
    use crate::gate::{GateRequirement, GateStage, GateStatus, RequiredEvidenceStatus};
    use crate::scope::EvidenceScope;
    use ironmaint_core::{CandidateFingerprint, EvidenceId, GateId, JobId};
    use time::macros::datetime;

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    fn other_fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("b".repeat(64)).unwrap()
    }

    fn evidence() -> Evidence {
        Evidence::new(
            fp(),
            EvidenceKind::Build,
            EvidenceStatus::Pass,
            EvidenceProducer::new("sbuild"),
            EvidenceScope::Job(JobId::new()),
            datetime!(2026-01-01 00:00:00 UTC),
        )
    }

    fn gate_definition_for_test() -> GateResult {
        GateResult::pass(
            GateId::new(),
            fp(),
            Vec::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        )
    }

    #[test]
    fn empty_graph_has_no_entries() {
        let g = EvidenceGraph::new();
        assert_eq!(g.evidence_count(), 0);
        assert_eq!(g.gate_result_count(), 0);
        assert!(g.gate_result(GateId::new()).is_none());
        assert!(g.evidence_by_id(EvidenceId::new()).is_none());
    }

    #[test]
    fn attach_evidence_adds_to_graph() {
        let mut g = EvidenceGraph::new();
        let ev = evidence();
        g.attach_evidence(ev.clone()).unwrap();
        assert_eq!(g.evidence_count(), 1);
        assert_eq!(g.evidence_by_id(ev.id), Some(&ev));
    }

    #[test]
    fn attach_duplicate_evidence_rejected() {
        let mut g = EvidenceGraph::new();
        let ev = evidence();
        g.attach_evidence(ev.clone()).unwrap();
        let err = g.attach_evidence(ev).unwrap_err();
        assert!(matches!(err, EvidenceError::DuplicateEvidence(_)));
    }

    #[test]
    fn attach_gate_result_adds_to_graph() {
        let mut g = EvidenceGraph::new();
        let r = gate_definition_for_test();
        g.attach_gate_result(r.clone()).unwrap();
        assert_eq!(g.gate_result_count(), 1);
        assert_eq!(g.gate_result(r.gate_id), Some(&r));
    }

    #[test]
    fn attach_duplicate_gate_result_rejected() {
        let mut g = EvidenceGraph::new();
        let r = gate_definition_for_test();
        g.attach_gate_result(r.clone()).unwrap();
        let err = g.attach_gate_result(r).unwrap_err();
        assert!(matches!(err, EvidenceError::DuplicateGateResult(_)));
    }

    #[test]
    fn evidence_for_candidate_filters_correctly() {
        let mut g = EvidenceGraph::new();
        // Two candidates' evidence.
        let ev1 = evidence();
        let mut ev2 = evidence();
        ev2.candidate = other_fp();
        g.attach_evidence(ev1.clone()).unwrap();
        g.attach_evidence(ev2.clone()).unwrap();

        let for_fp1: Vec<&Evidence> = g.evidence_for_candidate(&fp()).collect();
        assert_eq!(for_fp1.len(), 1);
        assert_eq!(for_fp1[0].id, ev1.id);

        let for_fp2: Vec<&Evidence> = g.evidence_for_candidate(&other_fp()).collect();
        assert_eq!(for_fp2.len(), 1);
        assert_eq!(for_fp2[0].id, ev2.id);
    }

    #[test]
    fn gate_result_unknown_id_returns_none() {
        let g = EvidenceGraph::new();
        assert!(g.gate_result(GateId::new()).is_none());
    }

    #[test]
    fn gate_result_status_propagates_through_lookup() {
        let mut g = EvidenceGraph::new();
        let r = GateResult::fail(
            GateId::new(),
            fp(),
            Vec::new(),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        g.attach_gate_result(r.clone()).unwrap();
        let looked_up = g.gate_result(r.gate_id).unwrap();
        assert_eq!(looked_up.status, GateStatus::Fail);
    }

    #[test]
    fn iterators_yield_all_attached_entries() {
        let mut g = EvidenceGraph::new();
        g.attach_evidence(evidence()).unwrap();
        g.attach_evidence(evidence()).unwrap();
        g.attach_gate_result(gate_definition_for_test()).unwrap();
        assert_eq!(g.evidence().count(), 2);
        assert_eq!(g.gate_results().count(), 1);
    }

    #[test]
    fn graph_default_is_empty() {
        let g = EvidenceGraph::default();
        assert!(g.evidence().next().is_none());
        assert!(g.gate_results().next().is_none());
    }

    // Reference the gate types so the test module compiles cleanly
    // when the only assertions are on the graph; gate types are also
    // exercised in the round-trip tests above via GateResult.
    #[test]
    fn gate_types_are_constructible() {
        let _ = GateRequirement::new(EvidenceKind::Build, RequiredEvidenceStatus::Pass);
        let _ = GateStage::BuildValidation;
        // The unused-warning silencer; reference the status enum.
        let _ = GateStatus::Pass;
    }
}
