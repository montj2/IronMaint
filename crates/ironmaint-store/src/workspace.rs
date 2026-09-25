//! Workspace metadata persistence (`PHASE-0B.md §16-22`).
//!
//! The store records the current `WorkspaceRevision` per workspace,
//! supporting optimistic-concurrency `apply_patch` (PHASE-0B.md §78).
//! The `WorkspaceRevision` newtype itself lives in
//! `ironmaint-workspace` (commit 6 of Phase 0B).

use ironmaint_core::JobId;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Opaque workspace id. Phase 0A only had `JobId`; workspaces are
/// a 0B concept. The store accepts any `String`-shaped id; backends
/// store it verbatim. Phase 1 will introduce a typed `WorkspaceId`
/// in `ironmaint-workspace`.
pub type WorkspaceHandle = String;

/// Current state of a workspace managed by `ironmaint-workspace`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub handle: WorkspaceHandle,
    pub job_id: JobId,
    pub revision: u64,
}

/// Durable storage for workspace metadata.
pub trait WorkspaceMetadataStore: Send + Sync {
    /// Read the current state for a workspace.
    async fn get_workspace_state(
        &self,
        handle: &WorkspaceHandle,
    ) -> Result<WorkspaceState, StoreError>;

    /// Persist a new state for a workspace, taking the previous
    /// revision as `expected_revision` for CAS. Returns
    /// [`crate::error::StoreErrorKind::Conflict`] on mismatch.
    async fn put_workspace_state(
        &self,
        state: &WorkspaceState,
        expected_revision: u64,
    ) -> Result<(), StoreError>;

    /// List workspaces attached to a job.
    async fn list_workspaces_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<WorkspaceHandle>, StoreError>;
}
