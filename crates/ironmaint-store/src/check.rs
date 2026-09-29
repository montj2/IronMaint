//! Check persistence (`PHASE-0B.md §44`).
//!
//! `CheckDefinition` is the durable counterpart to the adapter's
//! `PlannedCheck`. Where `PlannedCheck { key, evidence_kind,
//! mandatory }` is the *plan* (what the adapter said to run for a
//! gate), `CheckDefinition` is the *contract* that ties a
//! `(candidate, gate, tool)` triple to a stable `CheckId` that
//! evidence rows and `RunCheck` requests reference.
//!
//! Materialisation is the runtime's responsibility — see
//! `ironmaint-runtime::check::materialize_planned_checks` —
//! which converts adapter output into `CheckDefinition` rows
//! plus their corresponding `GateDefinition` rows.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{CandidateFingerprint, CheckId, GateId, JobId};
use ironmaint_evidence::{EvidenceKind, GateStage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// A single, durable check tied to a candidate and a gate.
///
/// The struct carries:
/// - `id`: stable UUIDv7 used by `RuntimeCommand::RunCheck { check_id }`
/// - `job_id`: the owning job (so per-job lookups are a single index
///   query and not a scan over all checks)
/// - `candidate`: which candidate this check is bound to (§30
///   evidence freshness — a check for an old candidate cannot
///   satisfy a new candidate's gate)
/// - `gate_id`: the `GateDefinition` this check contributes to
/// - `gate_stage`: denormalised cache of `GateDefinition::stage`
///   so ledger walks do not need to re-join `GateStore`
/// - `capability`: the registered tool key the executor will look up
/// - `evidence_kind`: how `RunCheck` classifies the executor's
///   `Outcome` into an `EvidenceStatus`
/// - `mandatory`: whether a `GateFailed` blocks the gate
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckDefinition {
    pub id: CheckId,
    pub job_id: JobId,
    pub candidate: CandidateFingerprint,
    pub gate_id: GateId,
    pub gate_stage: GateStage,
    pub capability: ToolCapabilityKey,
    pub evidence_kind: EvidenceKind,
    pub mandatory: bool,
}

impl CheckDefinition {
    /// Construct a new `CheckDefinition`, minting a fresh
    /// `CheckId`. The `id` is a UUIDv7 so it sorts by creation
    /// time — useful for deterministic test output and for
    /// debugging.
    #[must_use]
    pub fn new(
        job_id: JobId,
        candidate: CandidateFingerprint,
        gate_id: GateId,
        gate_stage: GateStage,
        capability: ToolCapabilityKey,
        evidence_kind: EvidenceKind,
        mandatory: bool,
    ) -> Self {
        Self {
            id: CheckId::new(),
            job_id,
            candidate,
            gate_id,
            gate_stage,
            capability,
            evidence_kind,
            mandatory,
        }
    }
}

/// Persistence trait for `CheckDefinition` rows.
///
/// Implementors must guarantee:
/// - `put_check` is idempotent on `check.id` (a second call
///   overwrites — the contract is that callers mint fresh ids
///   rather than re-using them).
/// - `list_checks_for_job` returns checks in `id`-sorted order
///   (UUIDv7 monotonic) so per-job iteration is deterministic.
/// - `get_check` returns `StoreErrorKind::NotFound` for unknown
///   ids; callers map this to `RuntimeErrorKind::InvalidInput`.
///
/// Uses native `async fn` in traits per `ironmaint-store`'s
/// MSRV-1.85 contract — see lib.rs for the rationale.
pub trait CheckStore: Send + Sync {
    /// Persist a `CheckDefinition`. The id is allocated by the
    /// caller (via `CheckDefinition::new`); the store does not
    /// re-mint.
    fn put_check(
        &self,
        check: &CheckDefinition,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// Fetch a `CheckDefinition` by id. Returns
    /// `StoreErrorKind::NotFound` for unknown ids.
    fn get_check(
        &self,
        id: CheckId,
    ) -> impl std::future::Future<Output = Result<CheckDefinition, StoreError>> + Send;

    /// List all checks attached to a job, in id-sorted order.
    fn list_checks_for_job(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Vec<CheckId>, StoreError>> + Send;
}
