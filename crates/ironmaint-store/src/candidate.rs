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
    fn put_source_candidate(
        &self,
        candidate: &SourceCandidate,
    ) -> impl std::future::Future<Output = Result<CandidateId, StoreError>> + Send;

    /// Look up a `SourceCandidate` by id.
    fn get_source_candidate(
        &self,
        id: CandidateId,
    ) -> impl std::future::Future<Output = Result<SourceCandidate, StoreError>> + Send;

    /// Persist a new `ReleaseCandidate`.
    fn put_release_candidate(
        &self,
        candidate: &ReleaseCandidate,
    ) -> impl std::future::Future<Output = Result<ReleaseCandidateId, StoreError>> + Send;

    /// Look up a `ReleaseCandidate` by id.
    fn get_release_candidate(
        &self,
        id: ReleaseCandidateId,
    ) -> impl std::future::Future<Output = Result<ReleaseCandidate, StoreError>> + Send;

    /// Every `ReleaseCandidate` for a job, in creation order.
    ///
    /// Not a convenience over [`CandidateStore::get_release_candidate`]:
    /// a snapshot is addressed by id, and a caller that has only a
    /// `JobId` — the runtime, when asked for a job's release
    /// candidate — has no id to ask with. `ReleaseCandidate` carries
    /// its own `job_id` and `source` fingerprint, so the selection
    /// belongs to the caller and the store's job is only to hand
    /// back the candidates.
    ///
    /// A job normally has at most one of these per candidate, but
    /// "normally" is a fact about the workflow rather than a
    /// constraint the store enforces, so this returns all of them
    /// and lets the caller decide which one it meant.
    fn list_release_candidates_for_job(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Vec<ReleaseCandidate>, StoreError>> + Send;

    /// List `SourceCandidate` ids for a job, in capture order.
    fn list_source_candidates_for_job(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Vec<CandidateId>, StoreError>> + Send;

    /// The active `SourceCandidate` for a job (the one driving the
    /// current workflow), if any.
    fn active_source_candidate(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Option<CandidateId>, StoreError>> + Send;

    /// Set the active `SourceCandidate` for a job. No-op if already
    /// set to the same id.
    fn set_active_source_candidate(
        &self,
        job_id: JobId,
        id: CandidateId,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// Look up a `SourceCandidate` by its immutable fingerprint.
    /// Used to deduplicate captures that resolve to the same bytes.
    fn find_source_by_fingerprint(
        &self,
        fingerprint: &CandidateFingerprint,
    ) -> impl std::future::Future<Output = Result<Option<CandidateId>, StoreError>> + Send;
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

    async fn list_release_candidates_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<ReleaseCandidate>, StoreError> {
        (**self).list_release_candidates_for_job(job_id).await
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
