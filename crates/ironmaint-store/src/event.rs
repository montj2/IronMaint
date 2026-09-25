//! Per-job event log (`PHASE-0B.md §7, §56`).
//!
//! The store appends events in monotonically-increasing
//! `sequence`-per-`job_id` order; out-of-order appends return
//! [`crate::error::StoreErrorKind::SequenceOutOfRange`].

use ironmaint_core::JobId;

use crate::envelope::EventEnvelope;
use crate::error::StoreError;

/// Durable append-only event log keyed by `JobId`.
///
/// All methods use native `async fn` in traits (stable on MSRV 1.85).
pub trait EventStore: Send + Sync {
    /// Append `event` to the per-job log. The store enforces
    /// `sequence == max(seen.sequence) + 1` for the same `job_id`;
    /// callers obtain the next sequence via [`Self::next_sequence`].
    async fn append_event(&self, event: &EventEnvelope) -> Result<(), StoreError>;

    /// Look up a single event by `(job_id, sequence)`.
    async fn get_event(&self, job_id: JobId, sequence: u64) -> Result<EventEnvelope, StoreError>;

    /// Walk the per-job log in monotonic `sequence` order. Returns
    /// events with `start <= sequence <= end`. If `end` is `None`,
    /// walks to the tail.
    async fn list_events_for_job(
        &self,
        job_id: JobId,
        start: u64,
        end: Option<u64>,
    ) -> Result<Vec<EventEnvelope>, StoreError>;

    /// The next sequence number the store will accept for `job_id`
    /// (i.e. `max(seen.sequence) + 1`). Returns `1` for a fresh job.
    async fn next_sequence(&self, job_id: JobId) -> Result<u64, StoreError>;
}
