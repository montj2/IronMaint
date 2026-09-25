//! `WorkspaceManager` — the workspace service object.

use std::path::PathBuf;
use std::str::FromStr;

use ironmaint_core::JobId;
use ironmaint_store::WorkspaceMetadataStore;
use ironmaint_store::workspace::WorkspaceState;

use crate::error::{WorkspaceError, WorkspaceErrorKind};
use crate::id::WorkspaceId;
use crate::revision::WorkspaceRevision;

/// Coordinates creation, lookup, and CAS-guarded mutation of
/// per-job workspace metadata.
///
/// The manager is intentionally thin: it persists a
/// `(handle → WorkspaceState{revision, job_id})` row per
/// workspace and exposes a CAS helper for `apply_patch`.
#[derive(Debug, Clone)]
pub struct WorkspaceManager<S: WorkspaceMetadataStore> {
    root: PathBuf,
    store: S,
}

impl<S: WorkspaceMetadataStore> WorkspaceManager<S> {
    pub fn new(root: impl Into<PathBuf>, store: S) -> Self {
        Self {
            root: root.into(),
            store,
        }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// Allocate a fresh workspace and persist its initial state.
    pub async fn create(&self, job_id: JobId) -> Result<WorkspaceId, WorkspaceError> {
        let id = WorkspaceId::new();
        let handle = format!("{}", id);
        let state = WorkspaceState {
            handle: handle.clone(),
            job_id,
            revision: 0,
        };
        self.store
            .put_workspace_state(&state, 0)
            .await
            .map_err(|e| match e.kind() {
                ironmaint_store::StoreErrorKind::Conflict => {
                    WorkspaceError::new(WorkspaceErrorKind::Conflict {
                        expected: 0,
                        found: 1,
                    })
                }
                ironmaint_store::StoreErrorKind::NotFound => WorkspaceError::new(
                    WorkspaceErrorKind::Other("missing job projection".to_string()),
                ),
                other => WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                    "store error: {other:?}"
                ))),
            })?;
        let _ = self.root.join(&handle);
        Ok(id)
    }

    /// Read the current revision for an existing workspace.
    pub async fn current_revision(
        &self,
        id: WorkspaceId,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        let handle = format!("{id}");
        let state = self.store.get_workspace_state(&handle).await.map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!("store error: {e:?}")))
        })?;
        Ok(WorkspaceRevision(state.revision))
    }

    /// CAS check: returns `Ok(true)` if `expected` matches the
    /// persisted revision, `Ok(false)` if it does not.
    pub async fn check_revision(
        &self,
        id: WorkspaceId,
        expected: WorkspaceRevision,
    ) -> Result<bool, WorkspaceError> {
        let current = self.current_revision(id).await?;
        Ok(current == expected)
    }

    /// Bump the persisted revision for `id` from `expected` to
    /// `expected.next()`. Returns the new revision or
    /// `WorkspaceError::Conflict` on CAS failure.
    pub async fn bump_revision(
        &self,
        id: WorkspaceId,
        expected: WorkspaceRevision,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        let handle = format!("{id}");
        let current = self.current_revision(id).await?;
        if current != expected {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Conflict {
                expected: expected.0,
                found: current.0,
            }));
        }
        let next = current.next();
        let state = self.store.get_workspace_state(&handle).await.map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!("store error: {e:?}")))
        })?;
        let new_state = WorkspaceState {
            handle: state.handle,
            job_id: state.job_id,
            revision: next.0,
        };
        self.store
            .put_workspace_state(&new_state, state.revision)
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Other(format!("store: {e:?}"))))?;
        Ok(next)
    }

    pub async fn state(&self, id: WorkspaceId) -> Result<WorkspaceState, WorkspaceError> {
        let handle = format!("{id}");
        self.store.get_workspace_state(&handle).await.map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!("store error: {e:?}")))
        })
    }

    /// Parse an id from a string (the canonical handle used by
    /// the daemon's persisted records). The raw form is the
    /// hex representation of the UUIDv7.
    pub fn id_from_str(raw: &str) -> Result<WorkspaceId, WorkspaceError> {
        WorkspaceId::from_str(raw).map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                "invalid workspace id `{raw}`: {e}"
            )))
        })
    }
}
