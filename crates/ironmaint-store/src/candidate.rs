//! Candidate persistence (`PHASE-0B.md §6`).
//!
//! Both `SourceCandidate` (Phase 0A) and `ReleaseCandidate`
//! (Phase 0A) are immutable records. The store holds the latest
//! per-job active candidate and the historical record by id.

use std::sync::Arc;

use ironmaint_core::{
    CandidateFingerprint, CandidateId, JobId, ReleaseCandidateId, SourceCandidate,
};
use ironmaint_policy::ReleaseCandidate;

use crate::error::StoreError;

/// Durable storage for `SourceCandidate` and `ReleaseCandidate`.
pub trait CandidateStore: Send + Sync {
    /// Persist a new `SourceCandidate`. The store indexes it by
    /// `candidate.id` and by `(job_id, fingerprint)` for lookup.
    async fn put_source_candidate(
        &self,
        candidate: &SourceCandidate,
    ) -> Result<CandidateId, StoreError>;

    /// Look up a `SourceCandidate` by id.
    async fn get_source_candidate(&self, id: CandidateId) -> Result<SourceCandidate, StoreError>;

    /// Persist a new `ReleaseCandidate`.
    async fn put_release_candidate(
        &self,
        candidate: &ReleaseCandidate,
    ) -> Result<ReleaseCandidateId, StoreError>;

    /// Look up a `ReleaseCandidate` by id.
    async fn get_release_candidate(
        &self,
        id: ReleaseCandidateId,
    ) -> Result<ReleaseCandidate, StoreError>;

    /// List `SourceCandidate` ids for a job, in capture order.
    async fn list_source_candidates_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<CandidateId>, StoreError>;

    /// The active `SourceCandidate` for a job (the one driving the
    /// current workflow), if any.
    async fn active_source_candidate(
        &self,
        job_id: JobId,
    ) -> Result<Option<CandidateId>, StoreError>;

    /// Set the active `SourceCandidate` for a job. No-op if already
    /// set to the same id.
    async fn set_active_source_candidate(
        &self,
        job_id: JobId,
        id: CandidateId,
    ) -> Result<(), StoreError>;

    /// Look up a `SourceCandidate` by its immutable fingerprint.
    /// Used to deduplicate captures that resolve to the same bytes.
    async fn find_source_by_fingerprint(
        &self,
        fingerprint: &CandidateFingerprint,
    ) -> Result<Option<CandidateId>, StoreError>;
}

/// Forwarding impl so a shared, reference-counted store can be
/// handed to a second consumer. See the rationale on
/// `WorkspaceMetadataStore for Arc<T>` in `workspace.rs` — the
/// workspace manager needs the runtime's `Arc<S>`, not a second
/// connection to a database that holds an exclusive lock.
impl<T: CandidateStore + ?Sized> CandidateStore for Arc<T> {
    async fn put_source_candidate(
        &self,
        candidate: &SourceCandidate,
    ) -> Result<CandidateId, StoreError> {
        (**self).put_source_candidate(candidate).await
    }

    async fn get_source_candidate(&self, id: CandidateId) -> Result<SourceCandidate, StoreError> {
        (**self).get_source_candidate(id).await
    }

    async fn put_release_candidate(
        &self,
        candidate: &ReleaseCandidate,
    ) -> Result<ReleaseCandidateId, StoreError> {
        (**self).put_release_candidate(candidate).await
    }

    async fn get_release_candidate(
        &self,
        id: ReleaseCandidateId,
    ) -> Result<ReleaseCandidate, StoreError> {
        (**self).get_release_candidate(id).await
    }

    async fn list_source_candidates_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<CandidateId>, StoreError> {
        (**self).list_source_candidates_for_job(job_id).await
    }

    async fn find_source_by_fingerprint(
        &self,
        fingerprint: &CandidateFingerprint,
    ) -> Result<Option<CandidateId>, StoreError> {
        (**self).find_source_by_fingerprint(fingerprint).await
    }

    async fn active_source_candidate(
        &self,
        job_id: JobId,
    ) -> Result<Option<CandidateId>, StoreError> {
        (**self).active_source_candidate(job_id).await
    }

    async fn set_active_source_candidate(
        &self,
        job_id: JobId,
        id: CandidateId,
    ) -> Result<(), StoreError> {
        (**self).set_active_source_candidate(job_id, id).await
    }
}
