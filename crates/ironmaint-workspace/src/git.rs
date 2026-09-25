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
            let status_char = line.chars().next().unwrap_or(' ');
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
}
