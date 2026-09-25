//! Artifact metadata persistence (`PHASE-0B.md §12-15`).
//!
//! The metadata store records *where* each artifact is in the
//! content-addressed blob store, its content hash, and provenance.
//! The blob bytes live in `ironmaint-artifacts`; this trait only
//! covers the metadata side.

use ironmaint_core::{ArtifactId, JobId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::StoreError;

/// Metadata about a single artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub id: ArtifactId,
    pub job_id: JobId,
    pub producer: String,
    pub content_kind: String,
    pub bytes: u64,
    #[doc(hidden)]
    pub stored_at: OffsetDateTime,
}

/// Durable storage for [`ArtifactRecord`] metadata.
pub trait ArtifactMetadataStore: Send + Sync {
    /// Persist a new artifact's metadata. The blob itself is
    /// written separately by `ironmaint-artifacts`; this method
    /// only records the index entry.
    async fn put_artifact(&self, record: &ArtifactRecord) -> Result<ArtifactId, StoreError>;

    /// Look up an artifact's metadata by id.
    async fn get_artifact(&self, id: ArtifactId) -> Result<ArtifactRecord, StoreError>;

    /// Check whether metadata exists for `id`. Cheaper than
    /// `get_artifact` when the caller only needs a presence check.
    async fn artifact_exists(&self, id: ArtifactId) -> Result<bool, StoreError>;

    /// List artifact ids attached to a job, newest-first.
    async fn list_artifacts_for_job(&self, job_id: JobId) -> Result<Vec<ArtifactId>, StoreError>;
}
