//! Evidence persistence (`PHASE-0B.md §6`).
//!
//! Evidence records are bound to a `CandidateFingerprint` (PHASE-0A
//! §1085). The store indexes by id and by `(job_id, fingerprint)`.

use ironmaint_core::{CandidateFingerprint, EvidenceId, JobId};
use ironmaint_evidence::Evidence;

use crate::error::StoreError;

/// Durable storage for [`Evidence`] records.
pub trait EvidenceStore: Send + Sync {
    /// Persist a new `Evidence` record. The store indexes it by
    /// `evidence.id` and by `job_id`. `job_id` is supplied by the
    /// caller (the runtime) because [`Evidence`] itself only carries
    /// a `CandidateFingerprint`; the job that produced the candidate
    /// is known to the caller at insert time.
    fn put_evidence(
        &self,
        evidence: &Evidence,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<EvidenceId, StoreError>> + Send;

    /// Look up an `Evidence` record by id.
    fn get_evidence(
        &self,
        id: EvidenceId,
    ) -> impl std::future::Future<Output = Result<Evidence, StoreError>> + Send;

    /// List `Evidence` records attached to a job, newest-first.
    fn list_evidence_for_job(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Vec<Evidence>, StoreError>> + Send;

    /// List `Evidence` records bound to a specific candidate
    /// fingerprint. Used by gate evaluation.
    fn list_evidence_for_candidate(
        &self,
        fingerprint: &CandidateFingerprint,
    ) -> impl std::future::Future<Output = Result<Vec<Evidence>, StoreError>> + Send;
}
