//! `capture_candidate` — snapshot a workspace's HEAD into an
//! immutable `SourceCandidate`.
//!
//! Phase 0B.8 introduces the *primitive* that lifts a
//! workspace's HEAD into a 0A-shaped `SourceCandidate`. The
//! runtime (0B.13) supplies the owning `JobId` and the URL of
//! the workspace's remote; commit 8 wires the bits that
//! don't need the runtime.

use ironmaint_core::{
    GitHashAlgorithm, GitObjectId, JobId, PackageIdentity, PackageRevision, RepositoryRef,
    SourceCandidate, VcsKind,
};
use time::OffsetDateTime;
use url::Url;

use ironmaint_store::WorkspaceMetadataStore;

use crate::error::{WorkspaceError, WorkspaceErrorKind};
use crate::git::GitInvocation;
use crate::id::WorkspaceId;
use crate::manager::WorkspaceManager;

impl<S: WorkspaceMetadataStore> WorkspaceManager<S> {
    /// Capture the workspace's HEAD commit as a `SourceCandidate`.
    /// `job_id` is the owning job (the runtime supplies this);
    /// `repository_url` is the canonical upstream URL the
    /// candidate is bound to. The fingerprint is computed from
    /// the immutable byte representation (0A §30 invariant).
    pub async fn capture_candidate(
        &self,
        id: WorkspaceId,
        job_id: JobId,
        package: PackageIdentity,
        repository_url: &str,
    ) -> Result<SourceCandidate, WorkspaceError> {
        let handle = format!("{id}");
        let ws_root = self.root().join(&handle);
        let inv = GitInvocation::sanitised_env(&ws_root);
        let commits = inv.log(1).await?;
        let commit = commits.first().ok_or_else(|| {
            WorkspaceError::new(WorkspaceErrorKind::Other(
                "no HEAD commit; workspace has no commits yet".to_string(),
            ))
        })?;

        let commit_oid =
            GitObjectId::new(GitHashAlgorithm::Sha1, commit.sha.clone()).map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Other(format!("bad commit oid: {e}")))
            })?;
        let tree_sha = read_tree_sha(&inv).await?;
        let tree_oid = GitObjectId::new(GitHashAlgorithm::Sha1, tree_sha).map_err(|e| {
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
        // resolved (see 0B.13).
        let version = ironmaint_core::PackageVersion::new("0+ironmaint").map_err(|e| {
            WorkspaceError::new(WorkspaceErrorKind::Other(format!("bad version: {e}")))
        })?;
        let package_revision = PackageRevision::new(package, version);

        Ok(SourceCandidate::new(
            job_id,
            package_revision,
            repo,
            commit_oid,
            tree_oid,
            OffsetDateTime::now_utc(),
        ))
    }
}

async fn read_tree_sha(inv: &GitInvocation) -> Result<String, WorkspaceError> {
    use tokio::process::Command;
    let out = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD^{tree}")
        .current_dir(&inv.workspace_root)
        .env_clear()
        .envs(&inv.env)
        .output()
        .await
        .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("spawn: {e}"))))?;
    if !out.status.success() {
        return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
            "rev-parse failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
