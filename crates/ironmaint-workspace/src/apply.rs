//! Controlled `apply_patch` flow with optimistic concurrency.

use ironmaint_store::WorkspaceMetadataStore;

use crate::error::{WorkspaceError, WorkspaceErrorKind};
use crate::git::GitInvocation;
use crate::id::WorkspaceId;
use crate::manager::WorkspaceManager;
use crate::revision::WorkspaceRevision;

impl<S: WorkspaceMetadataStore> WorkspaceManager<S> {
    /// Apply `patch` under `<workspace_root>/<handle>` with a CAS
    /// check against `expected_rev`.
    ///
    /// Returns the new revision after the apply, or
    /// `WorkspaceError::Conflict` if the persisted revision
    /// does not match `expected_rev`.
    pub async fn apply_patch(
        &self,
        id: WorkspaceId,
        expected_rev: WorkspaceRevision,
        patch: &str,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        if !self.check_revision(id, expected_rev).await? {
            let cur = self.current_revision(id).await?;
            return Err(WorkspaceError::new(WorkspaceErrorKind::Conflict {
                expected: expected_rev.0,
                found: cur.0,
            }));
        }

        let handle = format!("{id}");
        let ws_root = self.root().join(&handle);
        let dirty_check = GitInvocation::sanitised_env(&ws_root);
        let status = dirty_check.status().await?;
        if !status.is_empty() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Other(
                "workspace is dirty; refusing to apply patch".to_string(),
            )));
        }

        let inv = GitInvocation::sanitised_env(&ws_root);
        inv.apply(patch).await?;
        self.bump_revision(id, expected_rev).await
    }
}
