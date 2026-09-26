//! `read` / `list` / `diff` / `status` — the workspace content
//! surface (PHASE-0B.md §16, §19).
//!
//! These methods give callers a way to inspect what's in a
//! workspace without going through the git working tree directly.
//! They apply the §19 symlink policy:
//!
//! - `read`, `list`, `status` may follow symlinks within the
//!   workspace; they reject resolutions that escape the root.
//! - `diff` operates on the git tree (no filesystem access);
//!   symlinks in the diff are reported by `git diff` itself.
//!
//! `apply_patch` and `capture_candidate` use the strict no-follow
//! variant via `WorkspacePath::resolve_strict_no_follow`; see
//! `apply.rs` and `capture.rs`.

use std::path::PathBuf;

use ironmaint_store::WorkspaceMetadataStore;

use crate::error::WorkspaceError;
use crate::git::{GitFileStatus, GitInvocation};
use crate::id::WorkspaceId;
use crate::manager::WorkspaceManager;
use crate::path::WorkspacePath;

/// One entry from `WorkspaceManager::list`.
///
/// Symlinks are reported by `git status` but not by `list` —
/// the §19 policy reads them via `canonicalize` (or rejects
/// them as escapes); they are never surfaced as a separate kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDirEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

impl<S: WorkspaceMetadataStore> WorkspaceManager<S> {
    /// Read the file at `path` under the workspace's root.
    /// Uses the read-friendly symlink policy (§19): symlinks
    /// inside the workspace may be followed, but resolutions
    /// that escape the root are rejected.
    pub async fn read(
        &self,
        id: WorkspaceId,
        path: &std::path::Path,
    ) -> Result<Vec<u8>, WorkspaceError> {
        let handle = format!("{id}");
        let ws_root = self.root().join(&handle);
        let wp = WorkspacePath::resolve_for_read(&ws_root, path)?;
        tokio::fs::read(wp.full())
            .await
            .map_err(WorkspaceError::from)
    }

    /// List entries in `dir` under the workspace's root. Each
    /// entry's `kind` is `File` or `Directory`; symlinks are
    /// followed via `metadata` and reported by their target's
    /// type (or rejected as escapes if the target is outside
    /// the root).
    pub async fn list(
        &self,
        id: WorkspaceId,
        dir: &std::path::Path,
    ) -> Result<Vec<WorkspaceDirEntry>, WorkspaceError> {
        let handle = format!("{id}");
        let ws_root = self.root().join(&handle);
        let wp = WorkspacePath::resolve_for_read(&ws_root, dir)?;
        let full = wp.full();
        let mut entries = Vec::new();
        let mut rd = tokio::fs::read_dir(&full)
            .await
            .map_err(WorkspaceError::from)?;
        while let Some(e) = rd.next_entry().await.map_err(WorkspaceError::from)? {
            let name = e.file_name().to_string_lossy().into_owned();
            let md = e.metadata().await.map_err(WorkspaceError::from)?;
            let size = md.len();
            // Use symlink_metadata so a symlink is reported by
            // its target's type. The list policy permits in-root
            // symlinks; cross-root symlinks are rejected by
            // resolve_for_read before we ever get here.
            let target_md = std::fs::metadata(e.path()).map_err(WorkspaceError::from)?;
            let kind = if target_md.is_dir() {
                EntryKind::Directory
            } else if target_md.is_file() {
                EntryKind::File
            } else {
                // Special file (socket, fifo, block, char). We
                // surface these as File entries — the §19 policy
                // does not forbid them, and tools that list
                // workspace contents may need to handle them.
                EntryKind::File
            };
            entries.push(WorkspaceDirEntry { name, kind, size });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    /// Unified diff of the working tree against `base`. `base =
    /// None` diffs against HEAD. Delegates to `git diff` via
    /// `GitInvocation::diff_against`.
    pub async fn diff(
        &self,
        id: WorkspaceId,
        base: Option<&str>,
    ) -> Result<String, WorkspaceError> {
        let handle = format!("{id}");
        let ws_root = self.root().join(&handle);
        let inv = GitInvocation::sanitised_env(&ws_root);
        inv.diff_against(base).await
    }

    /// Working-tree status: paths with their `GitFileStatus`.
    /// Delegates to `git status --porcelain` via
    /// `GitInvocation::status`.
    pub async fn status(
        &self,
        id: WorkspaceId,
    ) -> Result<Vec<(PathBuf, GitFileStatus)>, WorkspaceError> {
        let handle = format!("{id}");
        let ws_root = self.root().join(&handle);
        let inv = GitInvocation::sanitised_env(&ws_root);
        let raw = inv.status().await?;
        Ok(raw
            .into_iter()
            .map(|(p, st)| (PathBuf::from(p), st))
            .collect())
    }
}
