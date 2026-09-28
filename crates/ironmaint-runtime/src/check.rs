//! Check planning + materialisation (`PHASE-0B.md §44`, §45).
//!
//! Adapters produce [`PlannedCheck`] records (the *plan*). The
//! runtime materialises those into durable
//! [`CheckDefinition`] rows tied to a candidate and a
//! `GateDefinition` (the *contract*). Materialisation is the
//! only step where the adapter's tool namespace crosses the
//! persistence boundary.
//!
//! ## Evidence-kind → gate-stage mapping
//!
//! [`PlannedCheck`] carries an `evidence_kind` rather than a
//! `GateStage`. The mapping below is the canonical lookup;
//! tests in `tests/materialize_checks.rs` pin it down so a
//! future change is intentional.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{CandidateFingerprint, GateId, JobId};
use ironmaint_evidence::{
    EvidenceKind, GateDefinition, GateRequirement, GateStage, RequiredEvidenceStatus,
};
use ironmaint_store::{CheckDefinition, CheckStore, GateStore, StoreError};

/// Map a `PlannedCheck`'s `evidence_kind` to the gate stage it
/// contributes to.
///
/// `Other(_)` does not correspond to a known gate stage — those
/// checks are filtered out by `materialize_planned_checks`
/// rather than silently mapped.
#[must_use]
pub fn gate_stage_for(kind: &EvidenceKind) -> Option<GateStage> {
    match kind {
        EvidenceKind::SourceIntegrity => Some(GateStage::SourceAnalysis),
        EvidenceKind::Build => Some(GateStage::BuildValidation),
        EvidenceKind::PackageQa => Some(GateStage::PackageQa),
        EvidenceKind::FunctionalTest => Some(GateStage::FunctionalValidation),
        EvidenceKind::UpgradeTest => Some(GateStage::UpgradeValidation),
        EvidenceKind::Reproducibility => Some(GateStage::FinalValidation),
        EvidenceKind::PolicyEvaluation => Some(GateStage::PolicyEvaluation),
        EvidenceKind::LicenseReview => Some(GateStage::FinalValidation),
        EvidenceKind::IssueCorrelation => Some(GateStage::IssueAnalysis),
        EvidenceKind::ReleaseAssembly => Some(GateStage::CandidateAssembly),
        EvidenceKind::PublicationValidation => Some(GateStage::Publication),
        EvidenceKind::Other(_) => None,
    }
}

/// A planned check as an adapter hands it back, after the
/// runtime has stripped the `key` and pinned it to a candidate
/// and a gate stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialisedCheck {
    pub gate_definition: GateDefinition,
    pub tool_key: ToolCapabilityKey,
    pub evidence_kind: EvidenceKind,
    pub mandatory: bool,
}

/// Convert a list of `(PlannedCheck, ToolCapabilityKey, mandatory)`
/// tuples into durable `GateDefinition`s. Pure: no I/O.
///
/// The caller is responsible for persisting each returned
/// `gate_definition` via `GateStore::put_gate_definition` and
/// building the corresponding `CheckDefinition` rows.
#[must_use]
pub fn plan_to_materialised(
    candidate: CandidateFingerprint,
    planned: &[(ToolCapabilityKey, EvidenceKind, bool)],
) -> Vec<MaterialisedCheck> {
    planned
        .iter()
        .filter_map(|(tool_key, evidence_kind, mandatory)| {
            let stage = gate_stage_for(evidence_kind)?;
            let requirement = GateRequirement {
                evidence_kind: evidence_kind.clone(),
                minimum_status: RequiredEvidenceStatus::Pass,
            };
            // `GateDefinition::new` auto-mints a GateId.
            let gate_definition =
                GateDefinition::new(candidate.clone(), stage, requirement, *mandatory);
            Some(MaterialisedCheck {
                gate_definition,
                tool_key: tool_key.clone(),
                evidence_kind: evidence_kind.clone(),
                mandatory: *mandatory,
            })
        })
        .collect()
}

/// Persist the materialised checks (gate definitions + check
/// definitions) for a job/candidate pair. Returns the freshly
/// minted `CheckDefinition`s in the same order as the input.
///
/// Idempotent: a second call with the same `(job_id, candidate,
/// tool_key)` mints fresh `CheckId`s but does not collide on
/// any store uniqueness key (gate definitions are upserted by
/// their auto-minted `GateId`, and check definitions are
/// upserted by the new `CheckId`).
///
/// # Errors
///
/// Returns `StoreError` on persistence failure. Callers map
/// to `RuntimeErrorKind::Store`.
pub async fn persist_materialised<S>(
    store: &S,
    job_id: JobId,
    candidate: CandidateFingerprint,
    materialised: &[MaterialisedCheck],
) -> Result<Vec<CheckDefinition>, StoreError>
where
    S: GateStore + CheckStore + ?Sized,
{
    let mut out = Vec::with_capacity(materialised.len());
    for m in materialised {
        store
            .put_gate_definition(&m.gate_definition, job_id)
            .await?;
        let gate_id: GateId = m.gate_definition.id;
        let stage = m.gate_definition.stage;
        let check = CheckDefinition::new(
            job_id,
            candidate.clone(),
            gate_id,
            stage,
            m.tool_key.clone(),
            m.evidence_kind.clone(),
            m.mandatory,
        );
        store.put_check(&check).await?;
        out.push(check);
    }
    Ok(out)
}
