//! `activate_candidate` — promote a captured `SourceCandidate` to
//! the active candidate for a workspace and a job.
//!
//! Per PHASE-0B.md §91, the §91 exit checkpoint requires the
//! workspace + candidate runtime to support:
//! C1 → edit → C2 → edit → C3 with immutable fingerprints and
//! preserved history, then activate_candidate(C3) such that
//! state.base_candidate == Some(C3.id()), state.dirty == false,
//! and the active_source_candidates row points at C3.
//!
//! `activate_candidate` performs three writes in this order:
//! 1. CAS-protected update of WorkspaceState:
//!    base_candidate = Some(candidate.id()), dirty = false,
//!    updated_at = now_utc(), under CAS with the *pre-bump*
//!    revision as expected.
//! 2. CAS-protected revision bump (bump_revision preserves dirty
//!    = false and re-stamps updated_at).
//! 3. UPSERT into active_source_candidates via the existing
//!    CandidateStore::set_active_source_candidate trait method.
//!
//! All three writes are idempotent: re-activating the same
//! candidate under the same job produces the same final state.
//! Activating a different candidate under the same job replaces
//! the active pointer (single-row UPSERT, by design).

use std::str::FromStr;

use ironmaint_store::{CandidateStore, WorkspaceMetadataStore};
use time::OffsetDateTime;

use crate::error::{WorkspaceError, WorkspaceErrorKind};
use crate::id::WorkspaceId;
use crate::manager::WorkspaceManager;

impl<S: WorkspaceMetadataStore + CandidateStore> WorkspaceManager<S> {
    /// Promote `candidate` to the active source candidate for its
    /// job and the workspace identified by `id`. Returns the new
    /// workspace revision on success.
    pub async fn activate_candidate(
        &self,
        id: WorkspaceId,
        candidate: &ironmaint_core::SourceCandidate,
    ) -> Result<crate::revision::WorkspaceRevision, WorkspaceError> {
        let handle = format!("{id}");

        // Step 1: CAS write that flips base_candidate + dirty=false
        // and re-stamps updated_at. The expected revision is the
        // *current* persisted revision (no bump yet).
        let mut state = self
            .store()
            .get_workspace_state(&handle)
            .await
            .map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Other(format!("store error: {e:?}")))
            })?;
        let pre_revision = state.revision;
        state.base_candidate = Some(candidate.id());
        state.dirty = false;
        state.updated_at = OffsetDateTime::now_utc();
        self.store()
            .put_workspace_state(&state, pre_revision)
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Other(format!("store: {e:?}"))))?;

        // Step 2: bump the revision to record the activation as a
        // distinct event. bump_revision preserves dirty=false and
        // re-stamps updated_at, so the post-activation state has
        // revision+1, dirty=false, base_candidate=Some(candidate.id()).
        let new_rev = self
            .bump_revision(id, crate::revision::WorkspaceRevision(pre_revision))
            .await?;

        // Step 3: UPSERT the active_source_candidates row.
        self.store()
            .set_active_source_candidate(candidate.job_id(), candidate.id())
            .await
            .map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                    "set_active_source_candidate: {e:?}"
                )))
            })?;

        Ok(new_rev)
    }

    /// Parse a hex `WorkspaceId` (the canonical handle). Used by
    /// callers that store the handle as a string and need to
    /// recover a typed id. (Convenience re-export so callers do
    /// not need a separate dependency on `crate::id`.)
    pub fn parse_id(raw: &str) -> Result<WorkspaceId, WorkspaceError> {
        WorkspaceId::from_str(raw).map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                "invalid workspace id `{raw}`: {e}"
            )))
        })
    }
}
