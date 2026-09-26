//! Minimal `git` invocation primitive.
//!
//! Wraps `tokio::process::Command` for the four subcommands the
//! workspace manager needs (PHASE-0B.md §39): `apply`, `status`,
//! `diff`, and `log -n N`. No rebasing, no fetch, no push.
//!
//! The environment is a *sanitised* map: callers construct an
//! explicit `BTreeMap` of env vars. There is no path where the
//! ambient process environment reaches the subprocess without
//! being explicitly selected by the caller.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

use crate::error::{WorkspaceError, WorkspaceErrorKind};

#[derive(Debug, Clone)]
pub struct GitInvocation {
    pub workspace_root: std::path::PathBuf,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitFileStatus {
    Untracked,
    Modified,
    Deleted,
    Renamed,
    Added,
}

impl GitInvocation {
    /// Construct a minimal, sanitised environment suitable for
    /// `git apply` / `git status` / `git diff` / `git log -n`.
    pub fn sanitised_env(cwd: &Path) -> Self {
        let mut env = BTreeMap::new();
        env.insert(
            "GIT_AUTHOR_NAME".to_string(),
            "ironmaint-workspace".to_string(),
        );
        env.insert(
            "GIT_AUTHOR_EMAIL".to_string(),
            "ironmaint-workspace@invalid".to_string(),
        );
        env.insert(
            "GIT_COMMITTER_NAME".to_string(),
            "ironmaint-workspace".to_string(),
        );
        env.insert(
            "GIT_COMMITTER_EMAIL".to_string(),
            "ironmaint-workspace@invalid".to_string(),
        );
        env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        env.insert("HOME".to_string(), "/var/empty".to_string());
        env.insert("LC_ALL".to_string(), "C.UTF-8".to_string());
        Self {
            workspace_root: cwd.to_path_buf(),
            env,
        }
    }

    /// Construct a sanitised env with author `IronMaint <ironmaint@localhost>`
    /// for §22 internal maintenance commits. Used by `capture_candidate`
    /// to land the immutable-history commit whose fingerprint each
    /// captured candidate derives from.
    pub fn for_maintenance_commit(cwd: &Path) -> Self {
        let mut env = BTreeMap::new();
        env.insert("GIT_AUTHOR_NAME".to_string(), "IronMaint".to_string());
        env.insert(
            "GIT_AUTHOR_EMAIL".to_string(),
            "ironmaint@localhost".to_string(),
        );
        env.insert("GIT_COMMITTER_NAME".to_string(), "IronMaint".to_string());
        env.insert(
            "GIT_COMMITTER_EMAIL".to_string(),
            "ironmaint@localhost".to_string(),
        );
        env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        env.insert("HOME".to_string(), "/var/empty".to_string());
        env.insert("LC_ALL".to_string(), "C.UTF-8".to_string());
        Self {
            workspace_root: cwd.to_path_buf(),
            env,
        }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new("git");
        c.current_dir(&self.workspace_root)
            .env_clear()
            .envs(&self.env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    pub async fn apply(&self, patch: &str) -> Result<(), WorkspaceError> {
        let mut child = self
            .cmd()
            .arg("apply")
            .arg("--check")
            .arg("-")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("spawn: {e}"))))?;
        {
            let stdin = child.stdin.as_mut().ok_or_else(|| {
                WorkspaceError::new(WorkspaceErrorKind::Command(
                    "stdin closed early".to_string(),
                ))
            })?;
            use tokio::io::AsyncWriteExt;
            stdin.write_all(patch.as_bytes()).await.map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Command(format!("write patch: {e}")))
            })?;
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git apply --check failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        let mut child = self
            .cmd()
            .arg("apply")
            .arg("-")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("spawn: {e}"))))?;
        {
            let stdin = child.stdin.as_mut().ok_or_else(|| {
                WorkspaceError::new(WorkspaceErrorKind::Command("stdin closed".to_string()))
            })?;
            use tokio::io::AsyncWriteExt;
            stdin.write_all(patch.as_bytes()).await.map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Command(format!("write: {e}")))
            })?;
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git apply failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(())
    }

    pub async fn status(&self) -> Result<Vec<(String, GitFileStatus)>, WorkspaceError> {
        let out = self
            .cmd()
            .arg("status")
            .arg("--porcelain")
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git status failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        let mut out_v = Vec::new();
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.len() < 3 {
                continue;
            }
            // `git status --porcelain` produces two status chars
            // followed by a space and the path. The first char is
            // the staged status, the second the unstaged status.
            // A non-space char in either position is a real status
            // we want to surface — ' M' (unstaged modification) is
            // the most common case after `apply_patch`.
            let chars: Vec<char> = line.chars().take(2).collect();
            let staged = chars.first().copied().unwrap_or(' ');
            let unstaged = chars.get(1).copied().unwrap_or(' ');
            let status_char = if staged != ' ' { staged } else { unstaged };
            let path = line[3..].to_string();
            let st = match status_char {
                '?' => GitFileStatus::Untracked,
                'M' => GitFileStatus::Modified,
                'D' => GitFileStatus::Deleted,
                'R' => GitFileStatus::Renamed,
                'A' => GitFileStatus::Added,
                _ => continue,
            };
            out_v.push((path, st));
        }
        Ok(out_v)
    }

    pub async fn diff(&self) -> Result<String, WorkspaceError> {
        let out =
            self.cmd().arg("diff").output().await.map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}")))
            })?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git diff failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub async fn log(&self, n: usize) -> Result<Vec<GitCommit>, WorkspaceError> {
        let out = self
            .cmd()
            .arg("log")
            .arg(format!("-n{n}"))
            .arg("--pretty=format:%H\t%s")
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git log failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        let mut out_v = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some((sha, subject)) = line.split_once('\t') {
                out_v.push(GitCommit {
                    sha: sha.to_string(),
                    subject: subject.to_string(),
                });
            }
        }
        Ok(out_v)
    }

    /// `git rev-parse HEAD` — the SHA of the current HEAD commit.
    pub async fn head_sha(&self) -> Result<String, WorkspaceError> {
        let out = self
            .cmd()
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git rev-parse HEAD failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// `git rev-parse HEAD^{tree}` — the SHA of the tree object
    /// referenced by HEAD. Used by `capture_candidate` to compute
    /// the immutable-fingerprint input.
    pub async fn tree_sha(&self) -> Result<String, WorkspaceError> {
        let out = self
            .cmd()
            .arg("rev-parse")
            .arg("HEAD^{tree}")
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git rev-parse HEAD^{{tree}} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// `git log --grep=... --invert-grep --pretty=format:%H -n 1` —
    /// the most recent commit whose message does NOT match the §22
    /// maintenance commit pattern. Used by `capture_candidate` to
    /// find the underlying commit whose OID anchors the immutable
    /// fingerprint (the §22 maintenance commit itself has a
    /// different OID per capture and would break capture
    /// determinism).
    pub async fn most_recent_non_maintenance_commit(&self) -> Result<String, WorkspaceError> {
        let out = self
            .cmd()
            .arg("log")
            .arg("--grep=IronMaint internal maintenance commit")
            .arg("--invert-grep")
            .arg("--pretty=format:%H")
            .arg("-n")
            .arg("1")
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git log failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// `git add -A && git write-tree` — stages all working-tree
    /// changes (modifications, deletions, and additions) and writes
    /// the resulting tree object, returning its OID. Used by
    /// `capture_candidate` to derive the fingerprint's `tree_oid`
    /// from the *current* working tree, not from HEAD^{tree} —
    /// so that captures taken after an `apply_patch` (which leaves
    /// the tree dirty) produce a fingerprint that reflects the
    /// new content. The `add -A` side effect on the index is
    /// acceptable: capture is the terminal step of the workflow
    /// for this workspace state.
    pub async fn current_tree(&self) -> Result<String, WorkspaceError> {
        let add_out = self
            .cmd()
            .arg("add")
            .arg("-A")
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !add_out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git add -A failed: {}",
                String::from_utf8_lossy(&add_out.stderr)
            ))));
        }
        let out =
            self.cmd().arg("write-tree").output().await.map_err(|e| {
                WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}")))
            })?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git write-tree failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// `git commit --allow-empty -m <message>` with this
    /// invocation's sanitised environment as author/committer.
    /// The `--allow-empty` flag is required so a capture flow
    /// that snapshots the same HEAD twice (deterministic fingerprint)
    /// can still land a §22 maintenance commit for the second
    /// capture without changing the tree.
    pub async fn commit(&self, message: &str) -> Result<(), WorkspaceError> {
        let out = self
            .cmd()
            .arg("commit")
            .arg("--allow-empty")
            .arg("-m")
            .arg(message)
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git commit failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(())
    }

    /// `git diff <base>` — unified diff against an optional base.
    /// `None` diffs against HEAD (no commits ahead of HEAD).
    pub async fn diff_against(&self, base: Option<&str>) -> Result<String, WorkspaceError> {
        let mut cmd = self.cmd();
        cmd.arg("diff");
        if let Some(b) = base {
            cmd.arg(b);
        }
        let out = cmd
            .output()
            .await
            .map_err(|e| WorkspaceError::new(WorkspaceErrorKind::Command(format!("wait: {e}"))))?;
        if !out.status.success() {
            return Err(WorkspaceError::new(WorkspaceErrorKind::Command(format!(
                "git diff failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}
