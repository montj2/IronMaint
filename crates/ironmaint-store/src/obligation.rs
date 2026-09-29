//! Obligation persistence (`PHASE-0B.md §6`).
//!
//! Obligations are derived from a `PolicyBaseline` and tracked per
//! job. Their `ObligationStatus` transitions through `NotEvaluated`,
//! `Pass`, `Fail`, `RequiresReview`, `ExceptionApproved`. The
//! runtime is the only writer.

use ironmaint_core::{JobId, ObligationId};
use ironmaint_policy::Obligation;

use crate::error::StoreError;

/// Durable storage for [`Obligation`] records.
pub trait ObligationStore: Send + Sync {
    /// Persist a new `Obligation` (initial state is `NotEvaluated`).
    /// `job_id` is supplied by the caller (see [`EvidenceStore`]).
    ///
    /// [`EvidenceStore`]: crate::evidence::EvidenceStore
    fn put_obligation(
        &self,
        obligation: &Obligation,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<ObligationId, StoreError>> + Send;

    /// Look up an `Obligation` by id.
    fn get_obligation(
        &self,
        id: ObligationId,
    ) -> impl std::future::Future<Output = Result<Obligation, StoreError>> + Send;

    /// Update the status of an existing obligation. Returns
    /// [`crate::error::StoreErrorKind::NotFound`] if the id is unknown.
    fn update_obligation(
        &self,
        id: ObligationId,
        updated: &Obligation,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// List obligations attached to a job, in declaration order.
    fn list_obligations_for_job(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Vec<ObligationId>, StoreError>> + Send;
}
