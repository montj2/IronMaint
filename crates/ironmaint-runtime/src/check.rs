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
    EvidenceKind, EvidenceStatus, GateDefinition, GateRequirement, GateResult, GateStage,
    GateStatus, RequiredEvidenceStatus,
};
use ironmaint_store::{CheckDefinition, CheckStore, EvidenceStore, GateStore, StoreError};

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

/// For each `CheckDefinition` attached to a given gate (gate_id
/// matching), find the latest `Evidence` whose `evidence_kind`
/// matches the check's, scope is `Candidate(candidate)`, and
/// return its id. Returns `None` for any check with no matching
/// evidence.
async fn latest_evidence_for_check<S>(
    store: &S,
    candidate: &CandidateFingerprint,
    check: &CheckDefinition,
) -> Result<Option<ironmaint_core::EvidenceId>, StoreError>
where
    S: EvidenceStore + ?Sized,
{
    let all = store.list_evidence_for_candidate(candidate).await?;
    let mut latest: Option<(time::OffsetDateTime, ironmaint_core::EvidenceId)> = None;
    for ev in all {
        if ev.kind != check.evidence_kind {
            continue;
        }
        let better = match latest {
            None => true,
            Some((prior, _)) => ev.observed_at > prior,
        };
        if better {
            latest = Some((ev.observed_at, ev.id));
        }
    }
    Ok(latest.map(|(_, id)| id))
}

/// Aggregate the latest evidence for every check attached to a
/// gate into a single `GateResult`. Pure aggregation — does not
/// persist.
///
/// Aggregation rules (PHASE-0B.md §29):
///   - All non-empty sets of latest evidence reach `Pass` only
///     when every check shows `Pass`.
///   - Any check showing `Fail` blocks the gate with `Fail`.
///   - Any check showing `InfrastructureError` blocks with
///     `Blocked` (the gate cannot be answered by evidence).
///   - Any check showing `Inconclusive` blocks with
///     `ReviewRequired`.
///   - If a check has no evidence yet, the gate is `NotEvaluated`.
///   - `NotApplicable` checks contribute nothing (the gate
///     ignores them).
///
/// `now` is the evaluation time stamped on the result.
pub async fn evaluate_gate<S>(
    store: &S,
    gate_id: GateId,
    candidate: CandidateFingerprint,
    checks: &[CheckDefinition],
    now: time::OffsetDateTime,
) -> Result<GateResult, StoreError>
where
    S: EvidenceStore + ?Sized,
{
    // Empty: nothing to evaluate.
    if checks.is_empty() {
        return Ok(GateResult {
            gate_id,
            candidate,
            status: GateStatus::NotEvaluated,
            evidence: Vec::new(),
            evaluated_at: now,
        });
    }

    let mut evidence_ids: Vec<ironmaint_core::EvidenceId> = Vec::new();
    let mut aggregate: GateStatus = GateStatus::NotEvaluated;

    for check in checks {
        if check.gate_id != gate_id {
            continue;
        }
        let Some(evidence_id) = latest_evidence_for_check(store, &candidate, check).await? else {
            // No evidence yet — gate stays NotEvaluated.
            aggregate = combine_status(aggregate, GateStatus::NotEvaluated);
            continue;
        };
        let ev = store.get_evidence(evidence_id).await?;
        let mapped = evidence_to_gate_status(ev.status);
        evidence_ids.push(evidence_id);
        aggregate = combine_status(aggregate, mapped);
    }

    Ok(GateResult {
        gate_id,
        candidate,
        status: aggregate,
        evidence: evidence_ids,
        evaluated_at: now,
    })
}

/// Combine two gate-status values from checks into the worst
/// one seen so far. `NotEvaluated` is the absence of evidence;
/// it is *not* a downgrade signal — it simply leaves the
/// aggregate unchanged when paired with a real status.
fn combine_status(a: GateStatus, b: GateStatus) -> GateStatus {
    let rank = |s: GateStatus| -> u8 {
        match s {
            GateStatus::Fail => 5,
            GateStatus::Blocked => 4,
            GateStatus::ReviewRequired => 3,
            GateStatus::NotApplicable => 1,
            GateStatus::Pass => 2,
            // `NotEvaluated` carries no signal: rank 0 so it
            // never wins a comparison against a real status.
            GateStatus::NotEvaluated => 0,
        }
    };
    let ra = rank(a);
    let rb = rank(b);
    if ra == 0 && rb == 0 {
        // Both inert: gate still not evaluated.
        GateStatus::NotEvaluated
    } else if ra == 0 {
        b
    } else if rb == 0 {
        a
    } else if rb > ra {
        b
    } else {
        a
    }
}

/// Map an evidence-status to the gate-status it implies.
fn evidence_to_gate_status(status: EvidenceStatus) -> GateStatus {
    match status {
        EvidenceStatus::Pass => GateStatus::Pass,
        EvidenceStatus::Fail => GateStatus::Fail,
        EvidenceStatus::NotApplicable => GateStatus::NotApplicable,
        EvidenceStatus::Inconclusive => GateStatus::ReviewRequired,
        EvidenceStatus::InfrastructureError => GateStatus::Blocked,
    }
}
