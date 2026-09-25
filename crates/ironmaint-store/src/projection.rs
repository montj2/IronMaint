//! Cached `JobProjection` (`PHASE-0B.md §8`).
//!
//! The store holds the latest projection per job. Backends rebuild
//! it on startup by walking the event log via [`crate::event::EventStore`]
//! and replaying through [`ironmaint_core::JobProjection::apply`].
//!
//! Optimistic concurrency: `put` takes an `expected_version` and
//! returns [`crate::error::StoreErrorKind::Conflict`] on mismatch.

use ironmaint_core::{JobId, JobProjection};

use crate::error::StoreError;

/// Durable cache of [`JobProjection`] keyed by `JobId`.
pub trait ProjectionStore: Send + Sync {
    /// Read the projection for `job_id`.
    async fn get_projection(&self, job_id: JobId) -> Result<JobProjection, StoreError>;

    /// Write `projection` only if the stored version equals
    /// `expected_version`. On success the stored version becomes
    /// `projection.version`.
    async fn put_projection(
        &self,
        projection: &JobProjection,
        expected_version: u64,
    ) -> Result<(), StoreError>;

    /// Reconstruct the projection from the event log. The default
    /// implementation walks events and applies them; backends can
    /// override for efficiency.
    async fn rebuild_projection(&self, job_id: JobId) -> Result<JobProjection, StoreError>;
}
