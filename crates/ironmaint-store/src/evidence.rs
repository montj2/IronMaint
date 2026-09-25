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
    async fn put_evidence(
        &self,
        evidence: &Evidence,
        job_id: JobId,
    ) -> Result<EvidenceId, StoreError>;

    /// Look up an `Evidence` record by id.
    async fn get_evidence(&self, id: EvidenceId) -> Result<Evidence, StoreError>;

    /// List `Evidence` records attached to a job, newest-first.
    async fn list_evidence_for_job(&self, job_id: JobId) -> Result<Vec<Evidence>, StoreError>;

    /// List `Evidence` records bound to a specific candidate
    /// fingerprint. Used by gate evaluation.
    async fn list_evidence_for_candidate(
        &self,
        fingerprint: &CandidateFingerprint,
    ) -> Result<Vec<Evidence>, StoreError>;
}
