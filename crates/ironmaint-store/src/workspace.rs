//! Workspace metadata persistence (`PHASE-0B.md §16-22`).
//!
//! The store records the current `WorkspaceRevision` per workspace,
//! supporting optimistic-concurrency `apply_patch` (PHASE-0B.md §78).
//! The `WorkspaceRevision` newtype itself lives in
//! `ironmaint-workspace` (commit 6 of Phase 0B).

use std::sync::Arc;

use ironmaint_core::{CandidateId, JobId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::StoreError;

/// Opaque workspace id. Phase 0A only had `JobId`; workspaces are
/// a 0B concept. The store accepts any `String`-shaped id; backends
/// store it verbatim. The typed `WorkspaceId` itself lives in
/// `ironmaint-workspace`.
pub type WorkspaceHandle = String;

/// Current state of a workspace managed by `ironmaint-workspace`.
///
/// `base_candidate`, `dirty`, `created_at`, and `updated_at` are the
/// four §17 fields populated by 0B.3's lifecycle stamping. Each is
/// `#[serde(default)]` so older JSON blobs (none exist yet, since
/// the fields are net-new) deserialize cleanly: missing values
/// become `None` / `false` / the `Default` `OffsetDateTime`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub handle: WorkspaceHandle,
    pub job_id: JobId,
    pub revision: u64,
    /// The currently-active source candidate for this workspace.
    /// `None` until `WorkspaceManager::activate_candidate` runs.
    #[serde(default)]
    pub base_candidate: Option<CandidateId>,
    /// `true` when the workspace tree has uncommitted changes
    /// (i.e., a `git apply` has landed but the changes have not
    /// been captured into a `SourceCandidate`). Owned by callers;
    /// `bump_revision` preserves it.
    #[serde(default)]
    pub dirty: bool,
    /// When the workspace was created.
    pub created_at: OffsetDateTime,
    /// When the workspace was last mutated (CAS write or bump).
    pub updated_at: OffsetDateTime,
}

/// Durable storage for workspace metadata.
pub trait WorkspaceMetadataStore: Send + Sync {
    /// Read the current state for a workspace.
    fn get_workspace_state(
        &self,
        handle: &WorkspaceHandle,
    ) -> impl std::future::Future<Output = Result<WorkspaceState, StoreError>> + Send;

    /// Persist a new state for a workspace, taking the previous
    /// revision as `expected_revision` for CAS. Returns
    /// [`crate::error::StoreErrorKind::Conflict`] on mismatch.
    fn put_workspace_state(
        &self,
        state: &WorkspaceState,
        expected_revision: u64,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// List workspaces attached to a job.
    fn list_workspaces_for_job(
        &self,
        job_id: JobId,
    ) -> impl std::future::Future<Output = Result<Vec<WorkspaceHandle>, StoreError>> + Send;
}

/// Forwarding impl so a shared, reference-counted store can be
/// handed to a second consumer.
///
/// `RuntimeService` owns its store privately as `Arc<S>` and
/// exposes no accessor (PHASE-0B.md §98.6 — the MCP layer reaches
/// persistence through the runtime, never around it). The workspace
/// manager needs the *same* store, not a second connection: a
/// daemon's SQLite backend takes an exclusive `fs2` lock on its
/// state directory, so opening it twice is not an option. This impl
/// lets `WorkspaceManager` borrow the runtime's handle instead of
/// demanding ownership of an `S` it cannot construct.
impl<T: WorkspaceMetadataStore + ?Sized> WorkspaceMetadataStore for Arc<T> {
    async fn get_workspace_state(
        &self,
        handle: &WorkspaceHandle,
    ) -> Result<WorkspaceState, StoreError> {
        (**self).get_workspace_state(handle).await
    }

    async fn put_workspace_state(
        &self,
        state: &WorkspaceState,
        expected_revision: u64,
    ) -> Result<(), StoreError> {
        (**self).put_workspace_state(state, expected_revision).await
    }

    async fn list_workspaces_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<WorkspaceHandle>, StoreError> {
        (**self).list_workspaces_for_job(job_id).await
    }
}
