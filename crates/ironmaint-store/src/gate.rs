//! Gate definition and result persistence (`PHASE-0B.md §6`).
//!
//! Stores both the immutable [`GateDefinition`] plan and the
//! mutable [`GateResult`] that the runtime updates as checks
//! complete.

use ironmaint_core::{CandidateFingerprint, GateId, JobId};
use ironmaint_evidence::{GateDefinition, GateResult};

use crate::error::StoreError;

/// Durable storage for gate definitions and per-candidate results.
pub trait GateStore: Send + Sync {
    /// Persist a `GateDefinition`. The runtime calls this when
    /// materialising an adapter's `PlannedCheck` list.
    /// `job_id` is supplied by the caller (see [`EvidenceStore`] for
    /// the rationale).
    ///
    /// [`EvidenceStore`]: crate::evidence::EvidenceStore
    fn put_gate_definition(
        &self,
        gate: &GateDefinition,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<GateId, StoreError>> + Send;

    /// Look up a `GateDefinition` by id.
    fn get_gate_definition(
        &self,
        id: GateId,
    ) -> impl std::future::Future<Output = Result<GateDefinition, StoreError>> + Send;

    /// Persist (or update) the result for a gate on a specific
    /// candidate. The store treats `(gate_id, fingerprint)` as
    /// the key.
    fn put_gate_result(
        &self,
        gate_id: GateId,
        fingerprint: &CandidateFingerprint,
        result: &GateResult,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// Read the result for a gate on a specific candidate.
    fn get_gate_result(
        &self,
        gate_id: GateId,
        fingerprint: &CandidateFingerprint,
    ) -> impl std::future::Future<Output = Result<GateResult, StoreError>> + Send;

    /// List all gate definitions attached to a job.
    fn list_gates_for_job(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Vec<GateId>, StoreError>> + Send;
}
