//! `WorkspaceManager` — the workspace service object.

use std::path::PathBuf;
use std::str::FromStr;

use ironmaint_adapter_api::BuildInputDescriptor;
use ironmaint_core::JobId;
use ironmaint_store::WorkspaceMetadataStore;
use ironmaint_store::workspace::WorkspaceState;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::BuildInputHandle;
use crate::error::{WorkspaceError, WorkspaceErrorKind};
use crate::git::GitInvocation;
use crate::id::WorkspaceId;
use crate::revision::WorkspaceRevision;

/// A frozen, read-only view of a workspace at a captured tree.
///
/// PHASE-1.1 PR 1.1.B (evidence integrity): the `check.run`
/// path operates on this view, not on the live workspace, so a
/// concurrent `apply_patch` cannot change the evidence's
/// binding to the captured tree. The view is materialised via
/// `git worktree add --detach` (the only path that ever
/// inspects the tree for a check); on `Drop` the worktree is
/// removed and the directory deleted.
///
/// The architectural property the runtime relies on is that
/// the check tool *only reads* from `path()`. A tool that
/// writes is a contract violation, and the drift check the
/// runtime runs after the check (`git rev-parse HEAD^{tree}`
/// on the worktree) catches it. The path is also a
/// sub-directory of the workspace handle so the
/// `.ironmaint-checks` namespace is owned by the runtime and
/// never overlaps with a patch the agent might apply.
pub struct FrozenWorktree {
    path: PathBuf,
    repo_root: PathBuf,
}

impl FrozenWorktree {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The tree OID of the frozen worktree, computed via
    /// `git rev-parse HEAD^{tree}`. The runtime calls this
    /// both before and after the check (the pre-check value
    /// is bound to the source candidate; the post-check value
    /// must match — a mismatch means the tool wrote).
    pub async fn tree_sha(&self) -> Result<String, WorkspaceError> {
        let inv = GitInvocation::sanitised_env(&self.path);
        inv.head_tree().await
    }
}

impl Drop for FrozenWorktree {
    fn drop(&mut self) {
        // Best-effort cleanup. The git worktree remove is
        // synchronous because the runtime's check pipeline is
        // not async at the drop site; the directory is then
        // deleted with `remove_dir_all` so the next capture
        // does not see stale files. Errors are swallowed
        // because the worktree will be removed the next time
        // `git worktree add` is called with `--force` if a
        // test crashes mid-check.
        let _ = std::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .current_dir(&self.repo_root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", "/var/empty")
            .output();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Coordinates creation, lookup, and CAS-guarded mutation of
/// per-job workspace metadata.
///
/// The manager persists the full `WorkspaceState` (§17) per
/// workspace and exposes a CAS helper for `apply_patch`. The
/// `dirty` flag is owned by callers (`apply_patch` sets it
/// `true`, `activate_candidate` sets it `false`); `bump_revision`
/// preserves it but always updates `updated_at`.
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
    /// A new workspace starts dirty: no `SourceCandidate` has been
    /// captured yet for its tree, so the §17 `dirty` flag is `true`.
    pub async fn create(&self, job_id: JobId) -> Result<WorkspaceId, WorkspaceError> {
        let id = WorkspaceId::new();
        let handle = format!("{}", id);
        let now = OffsetDateTime::now_utc();
        let state = WorkspaceState {
            handle: handle.clone(),
            job_id,
            revision: 0,
            base_candidate: None,
            dirty: true,
            created_at: now,
            updated_at: now,
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
    ///
    /// `dirty` is preserved (callers set it before/after this call
    /// as their semantics require). `updated_at` is bumped to now.
    /// `base_candidate` is preserved verbatim.
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
            base_candidate: state.base_candidate,
            dirty: state.dirty,
            created_at: state.created_at,
            updated_at: OffsetDateTime::now_utc(),
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

    /// Resolve the workspace attached to `job_id`, if one exists.
    pub async fn id_for_job(&self, job_id: JobId) -> Result<WorkspaceId, WorkspaceError> {
        let handles = self
            .store
            .list_workspaces_for_job(job_id)
            .await
            .map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Other(format!("store error: {e:?}")))
            })?;
        let handle = handles.last().ok_or_else(|| {
            WorkspaceError::new(WorkspaceErrorKind::NotFound(format!("job {job_id}").into()))
        })?;
        Self::id_from_str(handle)
    }

    /// PHASE-1.1 PR 1.1.G (source import, P2 #3): populate
    /// the workspace by cloning a remote. The current
    /// contents of the workspace directory are removed
    /// (the directory itself is kept, so the
    /// `WorkspaceState` row the manager persists in
    /// `ensure_workspace` survives the clone) and
    /// `git clone --depth 1 --branch <ref> <url>` is run
    /// into the empty directory. The workspace is left
    /// dirty so the next `candidate.capture` writes a
    /// §22 maintenance commit on top of the cloned
    /// branch.
    ///
    /// The URL is *not* checked here — the runtime's
    /// `source.import` dispatcher checks the allowlist
    /// before calling this method, so the manager
    /// preserves the single-responsibility split
    /// (URL policy lives in the MCP layer; tree
    /// materialisation lives in the workspace layer).
    ///
    /// Returns the workspace handle so the dispatcher
    /// can echo it back to the caller; this is the same
    /// string the manager's own `state(id)` returns, so
    /// a tool that captures next can resolve the same
    /// path the manager would.
    pub async fn import_git_remote(
        &self,
        job_id: JobId,
        url: &str,
        ref_name: Option<&str>,
    ) -> Result<String, WorkspaceError> {
        // `ensure_workspace` both resolves the existing
        // handle (or creates one) and materialises a
        // fresh git repo if none is there. We then
        // empty the directory so the clone has a clean
        // target.
        let id = self.ensure_workspace(job_id).await?;
        let dir = self.root.join(format!("{id}"));
        // The `.ironmaint-checks` directory is owned by
        // the runtime's frozen-worktree path; preserve
        // it across the import (clearing it would
        // orphan any in-flight `frozen_for_check`
        // call, which is exceedingly unlikely on a
        // brand-new workspace but possible on a re-use
        // case).
        let checks = dir.join(".ironmaint-checks");
        if checks.exists() {
            tokio::fs::remove_dir_all(&checks).await?;
        }
        // Remove everything else in the handle. The
        // `init` step in `ensure_workspace` left a
        // single commit on `main`; for a real import
        // we throw that away and start from the
        // remote. `remove_dir_all` on the handle
        // itself would also work, but the directory
        // must remain (its `WorkspaceState` row
        // references the handle string).
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
        // `git clone` writes the cloned tree into the
        // directory. The command runs from the parent
        // and targets the directory by name so the
        // workspace root is the clone destination.
        let inv = GitInvocation::sanitised_env(&dir);
        inv.clone_into(url, ref_name).await?;
        Ok(format!("{id}"))
    }

    /// Resolve the workspace for `job_id`, creating and
    /// materialising one if the job has none yet.
    ///
    /// `create` only persists metadata — it never touches the
    /// filesystem (`let _ = self.root.join(&handle);` in this
    /// module discards the path). Until 0B.9 the only callers were
    /// tests, which each built their own repository by hand. A
    /// daemon has no such caller, so the directory and its `git
    /// init` are produced here instead.
    ///
    /// Auto-provisioning is what makes the workspace tools usable
    /// at all: `workspace.stat`, `workspace.apply_patch` and
    /// `candidate.capture` are all keyed by `job_id`, and nothing
    /// in the MCP surface creates a workspace. Provisioning here
    /// keeps the tool count at nine rather than adding a
    /// `workspace.create` that an agent would have to discover
    /// first.
    pub async fn ensure_workspace(&self, job_id: JobId) -> Result<WorkspaceId, WorkspaceError> {
        let id = match self.id_for_job(job_id).await {
            Ok(id) => id,
            Err(e) if matches!(e.kind(), WorkspaceErrorKind::NotFound(_)) => {
                self.create(job_id).await?
            }
            Err(e) => return Err(e),
        };

        let dir = self.root.join(format!("{id}"));
        tokio::fs::create_dir_all(&dir).await?;
        if !dir.join(".git").exists() {
            let inv = GitInvocation::sanitised_env(&dir);
            inv.init().await?;
            // Seed an initial commit. `capture_candidate` derives
            // the candidate's commit OID from
            // `most_recent_non_maintenance_commit`, which is a
            // `git log` and therefore fails on a repository with no
            // commits. The seed is what breaks the otherwise
            // circular dependency: the maintenance commit that
            // `capture_candidate` writes is itself the first thing
            // that would give the repo a HEAD.
            //
            // It is authored by `sanitised_env`
            // ("ironmaint-workspace@invalid"), not
            // `for_maintenance_commit`, so it is not filtered out
            // as a §22 maintenance commit and remains eligible as
            // the fingerprint's commit input.
            inv.commit("IronMaint workspace initialisation\n\nempty tree")
                .await?;
        }
        Ok(id)
    }

    /// Materialise a frozen view of `id` at the captured
    /// tree `tree_sha`, returning a `FrozenWorktree` whose
    /// `path()` the runtime hands to the check tool. PHASE-1.1
    /// PR 1.1.B (evidence integrity): the previous `check.run`
    /// path passed the live workspace to the tool, so a
    /// concurrent `apply_patch` would change the evidence's
    /// tree hash between capture and check. The worktree is
    /// created at
    /// `<root>/<handle>/.ironmaint-checks/<worktree_id>` via
    /// `git worktree add --detach`; the directory is owned by
    /// the runtime and never overlaps with a patch the agent
    /// might apply.
    ///
    /// `tree_sha` is the candidate's tree OID. The
    /// `frozen.tree_sha()` method (a `git rev-parse
    /// HEAD^{tree}` on the worktree) returns the same value
    /// after the worktree is created, which is the
    /// post-check drift check the runtime runs.
    ///
    /// `worktree_id` is a caller-supplied label (typically the
    /// evidence id) that namespaces the worktree within the
    /// `.ironmaint-checks` directory so concurrent checks
    /// against the same workspace do not collide.
    pub async fn frozen_for_check(
        &self,
        id: WorkspaceId,
        worktree_id: &str,
        tree_sha: &str,
    ) -> Result<FrozenWorktree, WorkspaceError> {
        let handle = format!("{id}");
        let ws_root = self.root.join(&handle);
        let checks_dir = ws_root.join(".ironmaint-checks");
        tokio::fs::create_dir_all(&checks_dir).await?;
        let frozen_path = checks_dir.join(worktree_id);
        // A pre-existing directory (a stale worktree from a
        // crashed run, or a name collision) is removed before
        // the worktree is created. The `git worktree add`
        // command will not overwrite a populated directory.
        if frozen_path.exists() {
            let _ = std::process::Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(&frozen_path)
                .current_dir(&ws_root)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", "/var/empty")
                .output();
            let _ = tokio::fs::remove_dir_all(&frozen_path).await;
        }
        let inv = GitInvocation::sanitised_env(&ws_root);
        inv.worktree_add_detached(&frozen_path, tree_sha).await?;
        Ok(FrozenWorktree {
            path: frozen_path,
            repo_root: ws_root,
        })
    }

    /// PHASE-2 PR 2.0-D: stage the candidate into a
    /// disposable build input. See `doc/PHASE-2-D.md`
    /// and `crate::build_input::BuildInputHandle`.
    ///
    /// The 2.0-D slice is **quilt only**. The full
    /// implementation: `git archive` the candidate's
    /// commit into a tempdir, place the upstream
    /// tarball alongside, re-derive the tree OID via
    /// `git init && git add -A && git write-tree`
    /// (asserted equal to the candidate's tree),
    /// `chmod -R a-w` the staged root, create the
    /// output dir as a sibling, and return the
    /// `BuildInputHandle`.
    pub async fn stage_build_input(
        &self,
        job_id: &JobId,
        descriptor: &BuildInputDescriptor,
    ) -> Result<BuildInputHandle, WorkspaceError> {
        // 1. Resolve the JobId to a workspace id and
        //    workspace root path. The staging lives
        //    under the workspace handle, parallel to
        //    `.ironmaint-checks` (which is the
        //    `frozen_for_check` namespace).
        let id = self.id_for_job(*job_id).await?;
        let ws_root = self.root.join(format!("{id}"));
        if !ws_root.join(".git").exists() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                "workspace {} has no .git; cannot stage build input",
                ws_root.display()
            ))));
        }

        // 2. Compute the staged root and output dir
        //    paths. The fingerprint is the candidate's
        //    BLAKE3 hex (64 chars). 2.0-D is
        //    single-fingerprint-per-workspace: a
        //    re-stage of the same candidate
        //    overwrites the prior staged root and
        //    output dir. The output dir is a sibling
        //    of the staged root (`.output` next to
        //    the staged dir), not a sub-directory of
        //    it — the staged dir is read-only, the
        //    output dir is the only writable
        //    sibling.
        let fingerprint = descriptor.candidate.fingerprint().as_ref();
        let staged_root = ws_root.join("build-input").join(fingerprint);
        let output_dir = ws_root
            .join("build-input")
            .join(format!("{fingerprint}.output"));
        let upstream_tarball = &descriptor.source_format.upstream_tarball;

        // 3. Materialise the staging tempdir under
        //    `.ironmaint-staging/<uuid>/`. The
        //    `.ironmaint-staging` namespace is owned
        //    by the runtime (mirrors
        //    `.ironmaint-checks`); the uuid is a
        //    fresh one per call so concurrent
        //    stagings cannot collide.
        let staging_namespace = ws_root.join(".ironmaint-staging");
        tokio::fs::create_dir_all(&staging_namespace).await?;
        let staging_id = Uuid::now_v7();
        let staging_dir = staging_namespace.join(format!("{staging_id}"));
        tokio::fs::create_dir_all(&staging_dir).await?;

        // The staging operation is fallible at every
        // step. A failure after the tempdir is
        // created must remove the tempdir so the
        // next call starts clean. We structure the
        // body as a single async block returning
        // Result<BuildInputHandle, WorkspaceError>;
        // the cleanup runs after the block exits,
        // regardless of the outcome.
        let stage_result = async {
            // 4. Verify the upstream tarball's
            //    SHA-256 BEFORE placing it in the
            //    staged root. The runtime must not
            //    silently accept a tarball that does
            //    not match the descriptor's expected
            //    hash — a mismatch means the
            //    descriptor was built against a
            //    different tarball than the one on
            //    disk, which is a class of bug the
            //    build path cannot recover from.
            let tarball_bytes = tokio::fs::read(&upstream_tarball.path).await?;
            let mut hasher = Sha256::new();
            hasher.update(&tarball_bytes);
            let computed: [u8; 32] = hasher.finalize().into();
            if computed != upstream_tarball.sha256 {
                let expected = hex_lower(&upstream_tarball.sha256);
                let actual = hex_lower(&computed);
                return Err(WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                    "upstream tarball SHA-256 mismatch: descriptor expected {expected}, \
                     file {} computed {actual}",
                    upstream_tarball.path.display()
                ))));
            }

            // 5. `git archive <commit> | tar -xC
            //    <staging_dir>`. The archive contains
            //    the captured tree's files (the
            //    fixture's Makefile, hello.c,
            //    debian/, etc.). The command is run
            //    from the workspace dir, which is the
            //    git repo where the candidate's
            //    commit lives.
            let inv = GitInvocation::sanitised_env(&ws_root);
            inv.archive_to(descriptor.candidate.commit().as_str(), &staging_dir)
                .await?;

            // 6. Re-derive the staged tree OID via
            //    `git init && git add -A &&
            //    git write-tree` and assert it
            //    equals the candidate's tree. This
            //    is the structural test for 2.0-D's
            //    bullet 5: the staged workdir is
            //    exactly what the candidate's
            //    snapshot says it should be, and the
            //    read-only chmod prevents drift.
            //
            //    The tarball is placed AFTER this
            //    check (step 7 below) so the
            //    re-derivation sees only the
            //    candidate's captured tree, not the
            //    tarball that lives alongside.
            let derived_tree = inv.write_tree_from(&staging_dir).await?;
            if derived_tree != descriptor.candidate.tree().as_str() {
                return Err(WorkspaceError::new(WorkspaceErrorKind::Other(format!(
                    "staged tree OID {derived_tree} does not match the candidate's \
                     tree OID {}; the staging was lossy",
                    descriptor.candidate.tree().as_str()
                ))));
            }

            // 7. Place the upstream tarball
            //    alongside the extracted tree. The
            //    placement name is the descriptor's
            //    tarball filename (typically
            //    `<pkg>_<ver>.orig.tar.gz`); Debian's
            //    `dpkg-source -b` finds it by name.
            //    The tarball is excluded from the
            //    tree-OID re-derivation; the
            //    descriptor's `tree` field is the
            //    captured source-package tree, not
            //    the directory the build tool will
            //    see.
            let staged_tarball = staging_dir.join(upstream_tarball.filename());
            tokio::fs::copy(&upstream_tarball.path, &staged_tarball).await?;

            // 8. Create the final staged root
            //    (parent first), `rename` the
            //    staging dir into place (atomic on
            //    POSIX), and `chmod -R a-w` it so
            //    the build tool cannot modify the
            //    candidate. If the final path
            //    already exists from a prior stage
            //    of the same candidate, remove it
            //    first — re-staging is the common
            //    case for the same fingerprint.
            if let Some(parent) = staged_root.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            if staged_root.exists() {
                let _ = tokio::fs::remove_dir_all(&staged_root).await;
            }
            tokio::fs::rename(&staging_dir, &staged_root).await?;

            // `chmod -R a-w` is best-effort: a
            // filesystem that does not honour the
            // mode bits (e.g. Windows, or some
            // FAT-formatted volumes) is not
            // affected, but the `is_staged_root_readonly`
            // accessor on the handle will return
            // whatever the filesystem reports. 2A.3
            // adds the read-only bind-mount as
            // defence in depth.
            chmod_readonly_recursive(&staged_root)?;

            // 9. The tarball path inside the handle
            //    must point at the final location
            //    (post-rename), not the staging
            //    location (which no longer exists
            //    after the rename). The rename moved
            //    the staging dir to `staged_root`,
            //    so the tarball is at
            //    `staged_root/<basename>`.
            let final_tarball = staged_root.join(upstream_tarball.filename());

            // 10. Create the output dir as a sibling
            //     of the staged root
            //     (`<staged>.output`), not a
            //     sub-directory of it. The executor
            //     (2A.1) sets the build tool's CWD
            //     here.
            if output_dir.exists() {
                let _ = tokio::fs::remove_dir_all(&output_dir).await;
            }
            tokio::fs::create_dir_all(&output_dir).await?;

            // 11. Return the handle.
            Ok(BuildInputHandle::new(
                staged_root.clone(),
                derived_tree,
                descriptor.candidate.clone(),
                descriptor.source_format.clone(),
                final_tarball,
                output_dir,
            ))
        }
        .await;

        // 11. Cleanup: if the staging failed before
        //     the rename, remove the staging
        //     tempdir. If the rename succeeded, the
        //     tempdir no longer exists at the
        //     staging path (it was renamed to
        //     `staged_root`).
        if staging_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
        }
        stage_result
    }
}

fn hex_lower(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn chmod_readonly_recursive(path: &std::path::Path) -> Result<(), WorkspaceError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path)?;
    let mut perms = metadata.permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(path, perms)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            chmod_readonly_recursive(&entry.path())?;
        }
    }
    Ok(())
}
