//! `capture_candidate` — snapshot a workspace's HEAD into an
//! immutable `SourceCandidate` and persist the §22 maintenance
//! commit that anchors the capture in the workspace's history.
//!
//! §91 exit-checkpoint invariant: the captured candidate's
//! fingerprint is computed over the *pre-commit* underlying
//! commit and the working tree's tree OID, NOT over the §22
//! maintenance commit itself. That keeps capture deterministic —
//! two captures of the same tree produce different maintenance
//! commits (different OIDs) but the same fingerprint, because
//! the fingerprint inputs are unchanged.
//!
//! `capture_candidate` does NOT touch `WorkspaceState.dirty` or
//! the revision. The revision only reflects `apply_patch`-style
//! mutations; capture is a snapshot of an in-progress workspace
//! and the clean state is only established at
//! `activate_candidate` time (PHASE-0B.md §17).

use ironmaint_core::{
    GitHashAlgorithm, GitObjectId, JobId, PackageIdentity, PackageRevision, RepositoryRef,
    SourceCandidate, VcsKind,
};
use ironmaint_store::{CandidateStore, WorkspaceMetadataStore};
use time::OffsetDateTime;
use url::Url;

use crate::error::{WorkspaceError, WorkspaceErrorKind};
use crate::git::GitInvocation;
use crate::id::WorkspaceId;
use crate::manager::WorkspaceManager;

impl<S: WorkspaceMetadataStore + CandidateStore> WorkspaceManager<S> {
    /// Capture the workspace's HEAD commit as a `SourceCandidate`,
    /// persist the candidate via `CandidateStore`, and write the
    /// §22 internal maintenance commit that anchors this capture
    /// in the workspace's history (PHASE-0B.md §22-23).
    ///
    /// `job_id` is the owning job (the runtime supplies this);
    /// `repository_url` is the canonical upstream URL the
    /// candidate is bound to.
    pub async fn capture_candidate(
        &self,
        id: WorkspaceId,
        job_id: JobId,
        package: PackageIdentity,
        repository_url: &str,
    ) -> Result<SourceCandidate, WorkspaceError> {
        let handle = format!("{id}");
        let ws_root = self.root().join(&handle);

        // The fingerprint inputs are the underlying commit
        // (the most recent commit that is NOT a §22 maintenance
        // commit) and the working tree's tree OID (via
        // `git add -A && git write-tree`). The maintenance commit
        // itself has a different OID per capture and would break
        // capture determinism — two captures of the same tree
        // must produce one fingerprint (§91 exit checkpoint).
        // The `current_tree` call captures dirty-tree state too:
        // if `apply_patch` modified files between captures, the
        // tree OID changes accordingly.
        let inv = GitInvocation::sanitised_env(&ws_root);
        let commit_sha = inv.most_recent_non_maintenance_commit().await?;
        let tree_sha = inv.current_tree().await?;

        let commit_oid =
            GitObjectId::new(GitHashAlgorithm::Sha1, commit_sha.clone()).map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Other(format!("bad commit oid: {e}")))
            })?;
        let tree_oid = GitObjectId::new(GitHashAlgorithm::Sha1, tree_sha.clone()).map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!("bad tree oid: {e}")))
        })?;

        let repo = RepositoryRef::new(
            VcsKind::Git,
            Url::parse(repository_url).map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                    "bad repository url: {e}"
                )))
            })?,
        )
        .map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                "construct RepositoryRef: {e}"
            )))
        })?;

        // The 0A invariant requires the package version on the
        // candidate. The capture primitive uses
        // `PackageVersion::new` with the special
        // "0+ironmaint" sentinel — the real version is set by
        // the runtime after the package policy baseline is
        // resolved (see 0B.5).
        let version = ironmaint_core::PackageVersion::new("0+ironmaint").map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!("bad version: {e}")))
        })?;
        let package_revision = PackageRevision::new(package, version);

        let candidate = SourceCandidate::new(
            job_id,
            package_revision,
            repo,
            commit_oid,
            tree_oid,
            OffsetDateTime::now_utc(),
        );

        // Re-capturing an unchanged tree resolves to the same
        // fingerprint, and the fingerprint is the candidate's
        // identity — `migrations/0001` declares it UNIQUE and 0A
        // §22 says a source change creates a *new* candidate, so no
        // change means no new candidate. So the fingerprint is
        // looked up before the insert, and a hit resolves to the
        // stored row.
        //
        // Without this, the second `candidate.capture` of an
        // unchanged tree reached `put_source_candidate` and died on
        // the UNIQUE constraint, surfacing to the agent as
        // "UNIQUE constraint failed: source_candidates.fingerprint".
        // `RuntimeCommand::CaptureCandidate` has always deduped this
        // way; this path reaches the store directly, and so had
        // neither the guard nor its benefit. Mock-backed tests could
        // not see it — `MockStore::put_source_candidate` overwrites
        // and mints a second row silently — and `MockStore` is not
        // what the daemon runs.
        let existing = self
            .store()
            .find_source_by_fingerprint(candidate.fingerprint())
            .await
            .map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                    "find_source_by_fingerprint: {e:?}"
                )))
            })?;

        let (candidate_id, candidate) = match existing {
            Some(existing_id) => {
                let stored = self
                    .store()
                    .get_source_candidate(existing_id)
                    .await
                    .map_err(|e| {
                        WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                            "get_source_candidate: {e:?}"
                        )))
                    })?;
                // Return the stored candidate, not the freshly
                // built one. They agree on every fingerprinted
                // field and disagree on `captured_at`, and the
                // caller must not be handed a record that was
                // never written.
                (existing_id, stored)
            }
            None => {
                // Persist first to assign the CandidateId, then
                // write the maintenance commit with that id in the
                // message.
                let new_id = self
                    .store()
                    .put_source_candidate(&candidate)
                    .await
                    .map_err(|e| {
                        WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                            "put_source_candidate: {e:?}"
                        )))
                    })?;
                (new_id, candidate)
            }
        };

        // Read the workspace revision that this capture
        // captures under. The maintenance commit's message
        // records it so future audits can correlate the capture
        // with the workspace state at that point. It is written on
        // the dedup path too, naming the *existing* id — a commit
        // that names a candidate id no row carries would be a
        // dangling reference in the one place the audit trail
        // points at.
        let revision = self.current_revision(id).await?;
        let message = format!(
            "IronMaint internal maintenance commit\n\njob_id={job_id}\ncandidate_id={candidate_id}\nworkspace_revision={}",
            revision.0,
        );
        let maint_inv = GitInvocation::for_maintenance_commit(&ws_root);
        maint_inv.commit(&message).await?;

        Ok(candidate)
    }
}
