//! `ProcessExecutor` — the real executor.
//!
//! Replaces the `NullExecutor` skeleton with subprocess spawning
//! bound by [`ExecutionLimits`]. Responsibilities (PHASE-0B.md
//! §21, §32, §33):
//!
//! 1. Resolve a registered tool by capability key. Unknown
//!    capability → [`ExecutorErrorKind::UnknownCapability`].
//! 2. Reject `ExecutionClass::PrivilegedExternal` with a complete
//!    error path (PHASE-0B.md §27 — the variant exists for the
//!    conformance suite but using it is reserved for a future
//!    sub-phase).
//! 3. Spawn the tool subprocess with `tool.executable()` +
//!    `tool.fixed_args()` (no shell). Sanitised env from
//!    [`ProcessEnvironment`] merged with the request's
//!    allow-listed overrides.
//! 4. Read stdout / stderr concurrently through [`bounded_read`]
//!    caps so a child that emits gigabytes cannot exhaust memory.
//! 5. Enforce the wall-clock timeout via [`tokio::time::timeout`]:
//!    on expiry the child is killed and the executor returns
//!    [`ExecutorErrorKind::ToolFailed`] with `timed_out = true`.
//! 6. Classify spawn failures and wait failures as
//!    [`ExecutorErrorKind::InfrastructureFailed`] (distinct from
//!    `ToolFailed`, per §33).
//!
//! §92 exit-checkpoint taxonomy (`Pass` / `Fail` / `Timeout` /
//! `Interrupted` / `InfrastructureFailed`) is surfaced via the
//! [`crate::fixture::outcome_from_record`] helper and the
//! [`crate::fixture::Outcome`] enum.

use std::process::Stdio;
use std::sync::Arc;

use ironmaint_artifacts::ArtifactStore;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::artifact_guard::{JobArtifactGuardFactory, permissive_guard};
use crate::env::ProcessEnvironment;
use crate::error::{ExecutorError, ExecutorErrorKind};
use crate::executor::Executor;
use crate::limits::ExecutionClass;
use crate::record::ExecutionRecord;
use crate::registry::ToolRegistry;
use crate::request::ExecutionRequest;

/// Factory for wall-clock reads. Core has no `now()`; this
/// closure is the only way an `ExecutionRecord`'s `started_at` /
/// `finished_at` get a timestamp.
pub type TimeFactory = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

/// Default time factory: the real `OffsetDateTime::now_utc()`.
#[must_use]
pub fn wall_clock_time() -> TimeFactory {
    Arc::new(OffsetDateTime::now_utc)
}

/// Real executor: spawns the registered subprocess, captures
/// bounded stdout/stderr, enforces wall-clock timeout.
///
/// `artifact_store` and `guard_factory` are held for future wiring
/// (C5) where the runtime attaches stdout/stderr-derived
/// artifacts to evidence rows. The 0B.4 executor itself does not
/// write artifacts — it only captures bounded output into the
/// `ExecutionRecord`.
pub struct ProcessExecutor {
    registry: Arc<ToolRegistry>,
    // Reserved for C5 wiring: stdout/stderr that exceed `String`
    // ergonomic limits will be spilled to the store as bounded
    // artifacts. Held here so `ProcessExecutor` owns the whole
    // capture path.
    #[allow(dead_code)]
    artifact_store: Arc<ArtifactStore>,
    env: ProcessEnvironment,
    time_factory: TimeFactory,
    #[allow(dead_code)]
    guard_factory: JobArtifactGuardFactory,
}

impl std::fmt::Debug for ProcessExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessExecutor")
            .field("registry", &self.registry)
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

impl ProcessExecutor {
    /// Construct a process executor. Uses the real wall clock by
    /// default; tests should supply a fixed-time factory via
    /// [`Self::with_time_factory`].
    #[must_use]
    pub fn new(
        registry: Arc<ToolRegistry>,
        artifact_store: Arc<ArtifactStore>,
        env: ProcessEnvironment,
    ) -> Self {
        Self {
            registry,
            artifact_store,
            env,
            time_factory: wall_clock_time(),
            guard_factory: Arc::new(permissive_guard),
        }
    }

    /// Override the time factory (used by tests for deterministic
    /// `started_at` / `finished_at`).
    #[must_use]
    pub fn with_time_factory(mut self, factory: TimeFactory) -> Self {
        self.time_factory = factory;
        self
    }

    /// Override the artifact-guard factory.
    #[must_use]
    pub fn with_guard_factory(mut self, factory: JobArtifactGuardFactory) -> Self {
        self.guard_factory = factory;
        self
    }

    #[must_use]
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }
}

impl Executor for ProcessExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        // (1) Resolve the tool.
        let tool = self.registry.get(&request.tool_key).ok_or_else(|| {
            ExecutorError::new(
                ExecutorErrorKind::UnknownCapability,
                format!("capability not registered: {}", request.tool_key.as_str()),
            )
        })?;

        // (2) Reject the reserved-for-future class.
        if tool.class() == ExecutionClass::PrivilegedExternal {
            return Err(ExecutorError::new(
                ExecutorErrorKind::Other("PrivilegedExternal unsupported in 0B.4".to_string()),
                format!(
                    "tool {} uses ExecutionClass::PrivilegedExternal, which is reserved for a future sub-phase",
                    request.tool_key.as_str()
                ),
            ));
        }

        let limits = tool.limits();

        // (3) Build the Command. executable + fixed_args, no shell.
        let mut cmd = tokio::process::Command::new(tool.executable());
        for arg in tool.fixed_args() {
            cmd.arg(arg);
        }
        // (4) Sanitised env.
        for (k, v) in self.env.iter() {
            cmd.env(k, v);
        }
        for (k, v) in &request.env_overrides {
            cmd.env(k, v);
        }
        // (5) Pipe stdout/stderr; no stdin needed.
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        cmd.kill_on_drop(true);

        let started_at = (self.time_factory)();

        // (6) Spawn.
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Err(ExecutorError::new(
                    ExecutorErrorKind::InfrastructureFailed,
                    format!("spawn failed for tool {}: {e}", request.tool_key.as_str()),
                ));
            }
        };

        let stdout_pipe = child.stdout.take().ok_or_else(|| {
            ExecutorError::new(
                ExecutorErrorKind::InfrastructureFailed,
                "child stdout pipe missing after spawn",
            )
        })?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| {
            ExecutorError::new(
                ExecutorErrorKind::InfrastructureFailed,
                "child stderr pipe missing after spawn",
            )
        })?;

        // (7) Read stdout/stderr concurrently through bounded readers.
        let stdout_task = tokio::spawn(bounded_read(stdout_pipe, limits.stdout_max_bytes));
        let stderr_task = tokio::spawn(bounded_read(stderr_pipe, limits.stderr_max_bytes));

        // (8) Wait with wall-clock timeout.
        let exit_status = match tokio::time::timeout(limits.timeout, child.wait()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                return Err(ExecutorError::new(
                    ExecutorErrorKind::InfrastructureFailed,
                    format!("wait failed for tool {}: {e}", request.tool_key.as_str()),
                ));
            }
            Err(_elapsed) => {
                // Timeout: kill and reap the IO tasks.
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                return Err(ExecutorError::new(
                    ExecutorErrorKind::ToolFailed { timed_out: true },
                    format!(
                        "tool {} exceeded timeout {:?}",
                        request.tool_key.as_str(),
                        limits.timeout
                    ),
                ));
            }
        };

        let (stdout_bytes, stdout_truncated): (Vec<u8>, bool) =
            stdout_task.await.unwrap_or_default();
        let (stderr_bytes, stderr_truncated): (Vec<u8>, bool) =
            stderr_task.await.unwrap_or_default();
        let truncated = stdout_truncated || stderr_truncated;

        let finished_at = (self.time_factory)();
        // On Unix `code()` is `Some(code)` when the child exited
        // normally. `-1` is the sentinel for "killed by signal";
        // outcome mapping (`outcome_from_record`) treats any
        // unknown non-zero as `Fail`.
        let exit_code = exit_status.code().unwrap_or(-1);

        Ok(ExecutionRecord {
            tool_key: request.tool_key,
            retry_class: request.retry_class,
            started_at,
            finished_at,
            exit_code,
            stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
            retries_exhausted: false,
            truncated,
        })
    }
}

/// Read up to `max_bytes` from `reader` and return the kept
/// prefix plus a `truncated` flag.
///
/// When the cap is exceeded, subsequent bytes are *drained*
/// (consumed but discarded) so the child can finish writing
/// without blocking on a full pipe buffer. The drain terminates
/// when the child closes its end (EOF).
async fn bounded_read<R>(reader: R, max_bytes: u64) -> (Vec<u8>, bool)
where
    R: AsyncRead + Unpin,
{
    let mut reader = reader;
    let mut kept: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut chunk = vec![0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if truncated {
                    // Already over the cap; just drain.
                    continue;
                }
                let remaining = max_bytes.saturating_sub(kept.len() as u64);
                if remaining == 0 {
                    truncated = true;
                    continue;
                }
                if (n as u64) <= remaining {
                    kept.extend_from_slice(&chunk[..n]);
                } else {
                    kept.extend_from_slice(&chunk[..remaining as usize]);
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (kept, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_read_under_limit_returns_full() {
        let bytes: Vec<u8> = (0..100).collect();
        let (kept, truncated) = bounded_read(bytes.as_slice(), 1024).await;
        assert_eq!(kept.len(), 100);
        assert!(!truncated);
    }

    #[tokio::test]
    async fn bounded_read_at_exact_limit_not_truncated() {
        let bytes: Vec<u8> = vec![0u8; 100];
        let (kept, truncated) = bounded_read(bytes.as_slice(), 100).await;
        assert_eq!(kept.len(), 100);
        assert!(!truncated);
    }

    #[tokio::test]
    async fn bounded_read_over_limit_truncates() {
        let bytes: Vec<u8> = (0..200).collect();
        let (kept, truncated) = bounded_read(bytes.as_slice(), 100).await;
        assert_eq!(kept.len(), 100);
        assert!(truncated);
        assert_eq!(kept, (0..100).collect::<Vec<u8>>());
    }

    #[tokio::test]
    async fn bounded_read_empty_input() {
        let (kept, truncated) = bounded_read([].as_slice(), 1024).await;
        assert!(kept.is_empty());
        assert!(!truncated);
    }
}
