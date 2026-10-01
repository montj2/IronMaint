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
//! 7. Spill the **complete** stdout and stderr to the
//!    [`ArtifactStore`], bounded by this job's
//!    [`crate::artifact_guard::JobArtifactGuard`], and name the
//!    digests on the [`ExecutionRecord`].
//!
//! §92 exit-checkpoint taxonomy (`Pass` / `Fail` / `Timeout` /
//! `Interrupted` / `InfrastructureFailed`) is surfaced via the
//! [`crate::fixture::outcome_from_record`] helper and the
//! [`crate::fixture::Outcome`] enum.

use std::process::Stdio;
use std::sync::Arc;

use ironmaint_artifacts::ArtifactStore;
use ironmaint_core::JobId;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::artifact_guard::{
    GuardCache, JobArtifactGuard, JobArtifactGuardFactory, permissive_guard,
};
use crate::env::ProcessEnvironment;
use crate::error::{ExecutorError, ExecutorErrorKind};
use crate::executor::Executor;
use crate::limits::ExecutionClass;
use crate::record::{DroppedArtifact, ExecutionRecord, OutputStream, SpilledArtifact};
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

/// Capacity of the channel between the pipe reader and the spill
/// writer.
///
/// This is the whole memory argument for spilling, so it is
/// deliberately small and fixed: the reader keeps at most
/// `stdout_max_bytes` in memory *and* pushes every chunk into a
/// bounded channel, and the writer drains that channel to disk. A
/// child emitting a gigabyte is slowed by backpressure rather than
/// buffered, which is the property the previous drain-and-discard
/// gave up to stay inside the cap.
const SPILL_CHANNEL_BYTES: usize = 64 * 1024;

/// Real executor: spawns the registered subprocess, captures
/// bounded stdout/stderr, enforces wall-clock timeout, and retains
/// the complete output as artifacts.
pub struct ProcessExecutor {
    registry: Arc<ToolRegistry>,
    artifact_store: Arc<ArtifactStore>,
    env: ProcessEnvironment,
    time_factory: TimeFactory,
    guard_factory: JobArtifactGuardFactory,
    /// Memoises `guard_factory` per job, so the caps are per-job
    /// rather than per-call. See [`GuardCache`] for why that
    /// distinction is the whole point of a per-job cap.
    guards: Arc<GuardCache>,
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
            guards: Arc::new(GuardCache::default()),
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
        //
        // Each reader gets a spill channel. `bounded_read` keeps at
        // most `max_bytes` in memory for the record's ergonomic copy
        // and forwards *every* chunk down the channel, so the
        // artifact is the whole stream rather than the prefix the
        // record shows. Before D-08's fix the reader drained and
        // discarded past the cap, which is how an oversized build
        // log disappeared with no artifact and no indication.
        let (stdout_sink, stdout_spill) = tokio::io::duplex(SPILL_CHANNEL_BYTES);
        let (stderr_sink, stderr_spill) = tokio::io::duplex(SPILL_CHANNEL_BYTES);
        // The spill tasks outlive this borrow of `self` — they are
        // joined below, but `tokio::spawn` requires `'static`, so
        // they get owned clones of the two `Arc`s they need rather
        // than a borrow of the executor.
        let spiller = Spiller {
            store: Arc::clone(&self.artifact_store),
            guards: Arc::clone(&self.guards),
            factory: Arc::clone(&self.guard_factory),
        };
        let stdout_writer = tokio::spawn(spiller.clone().spill(request.job_id, stdout_sink));
        let stderr_writer = tokio::spawn(spiller.spill(request.job_id, stderr_sink));
        let stdout_task = tokio::spawn(bounded_read(
            stdout_pipe,
            limits.stdout_max_bytes,
            stdout_spill,
        ));
        let stderr_task = tokio::spawn(bounded_read(
            stderr_pipe,
            limits.stderr_max_bytes,
            stderr_spill,
        ));

        // (8) Wait with wall-clock timeout.
        let exit_status = match tokio::time::timeout(limits.timeout, child.wait()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                let _ = stdout_writer.await;
                let _ = stderr_writer.await;
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
                let _ = stdout_writer.await;
                let _ = stderr_writer.await;
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

        // (9) The spill writers finish once their reader has
        // dropped the write half, so awaiting them here is what
        // guarantees the digest exists before it is named on the
        // record. A digest that might appear later is a reference
        // that might not resolve.
        let stdout_artifact = stdout_writer.await.unwrap_or(None);
        let stderr_artifact = stderr_writer.await.unwrap_or(None);

        let mut artifacts = Vec::new();
        let mut artifacts_dropped = Vec::new();
        for (stream, outcome) in [
            (OutputStream::Stdout, stdout_artifact),
            (OutputStream::Stderr, stderr_artifact),
        ] {
            match outcome {
                Some(Ok(kept)) => artifacts.push(SpilledArtifact {
                    stream,
                    digest: kept.digest,
                    bytes: kept.bytes,
                    dropped_bytes: kept.dropped,
                }),
                Some(Err(reason)) => artifacts_dropped.push(DroppedArtifact { stream, reason }),
                // The task was cancelled or panicked. Recording
                // it as a drop rather than omitting the stream is
                // the point: an empty `artifacts_dropped` is the
                // claim "nothing was lost", and a vanished task
                // must not make that claim.
                None => artifacts_dropped.push(DroppedArtifact {
                    stream,
                    reason: "writer_cancelled".to_string(),
                }),
            }
        }

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
            artifacts,
            artifacts_dropped,
        })
    }
}

/// The two collaborators a spill task needs, by value.
///
/// A task rather than a method because `tokio::spawn` wants
/// `'static` and `execute` only borrows `&self`; carrying the two
/// `Arc`s is cheaper than restructuring the executor around an
/// owned `Arc<ProcessExecutor>`.
#[derive(Clone)]
struct Spiller {
    store: Arc<ArtifactStore>,
    guards: Arc<GuardCache>,
    factory: JobArtifactGuardFactory,
}

impl Spiller {
    /// Stream `source` into the artifact store, subject to
    /// `job_id`'s guard.
    ///
    /// §15's cap is a **refusal**, not a trim: the guard either
    /// admits the write or it does not, and the executor records
    /// which. Nothing here fails the tool run — the tool already
    /// ran, its exit code is a fact, and an artifact that would
    /// have blown the job's retention budget does not make the
    /// build any less passed or any less failed. It only changes
    /// what an operator can go and read afterwards, and the record
    /// says so.
    ///
    /// The `Option` is the join handle's own shape: a task that was
    /// cancelled or panicked yields `None`, and `execute` records
    /// that as a dropped artifact rather than reading it as "this
    /// stream produced nothing".
    async fn spill<R>(self, job_id: JobId, source: R) -> Option<Result<Spilled, String>>
    where
        R: AsyncRead + Unpin,
    {
        Some(self.spill_guarded(job_id, source).await)
    }

    async fn spill_guarded<R>(self, job_id: JobId, source: R) -> Result<Spilled, String>
    where
        R: AsyncRead + Unpin,
    {
        let guard: Arc<JobArtifactGuard> = self.guards.get(job_id, &self.factory);

        // Decide the bound *before* writing, so the artifact can
        // never exceed what the guard would have admitted. Reserving
        // against the cap rather than the eventual size is what
        // makes that true: a stream's size is not knowable up front,
        // and reserving the size after the bytes are on disk turns
        // the cap into a report rather than a limit.
        if guard.snapshot().count >= guard.max_count() {
            return Err("budget_count".to_string());
        }
        let remaining = guard.max_bytes().saturating_sub(guard.snapshot().bytes);
        let per_artifact = self.store.max_artifact_bytes().unwrap_or(u64::MAX);
        let limit = remaining.min(per_artifact);
        if limit == 0 {
            return Err("budget_bytes".to_string());
        }

        match self.store.put_stream_truncating(source, limit).await {
            Ok((record, info)) => {
                // Accounted after the write, against the bytes
                // actually retained. A concurrent retry can have
                // spent the budget in between, in which case this
                // refuses and the caller records the refusal — which
                // is the guard working, not a bug in the accounting.
                guard.try_record(record.size).map_err(|e| match e.kind {
                    ExecutorErrorKind::ArtifactBudgetExceeded { cap } => format!("budget_{cap}"),
                    _ => "budget_unknown".to_string(),
                })?;
                Ok(Spilled {
                    digest: record.digest,
                    bytes: record.size,
                    // Bytes the stream produced beyond what was kept.
                    // Carried so the record can distinguish "this
                    // build printed 40 MiB and we kept the first 8"
                    // from "this build printed 40 MiB and we kept
                    // all of it".
                    dropped: info
                        .observed_before_truncate
                        .saturating_sub(info.stored_bytes),
                })
            }
            Err(e) => Err(format!("io:{e}")),
        }
    }
}

/// A retained artifact, reduced to what the record names.
struct Spilled {
    digest: ironmaint_artifacts::hash::Sha256Hex,
    bytes: u64,
    dropped: u64,
}

/// Read up to `max_bytes` from `reader` into a returned buffer,
/// forwarding **every** byte to `sink` on the way past.
///
/// The two are separate concerns and both are needed. The returned
/// buffer is the ergonomic copy on the `ExecutionRecord`; the sink
/// is the durable one. Draining-and-discarding past the cap — which
/// is what this did before the fix — is defensible for the first
/// and indefensible for the second: it satisfies "do not exhaust
/// memory" by destroying the output, and a build log that
/// overflowed the cap disappeared with no artifact, no error, and
/// no way to tell that anything had been lost.
///
/// `sink` is bounded, so a child emitting gigabytes is slowed by
/// backpressure rather than buffered. The write half is dropped
/// when this function returns, which closes the sink's read side
/// and lets the spill task finish.
async fn bounded_read<R, W>(reader: R, max_bytes: u64, sink: W) -> (Vec<u8>, bool)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = reader;
    let mut sink = sink;
    let mut kept: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut chunk = vec![0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                // Forward first. If this fails the sink is gone
                // and there is nowhere for these bytes to go; the
                // in-memory prefix is still worth returning, so the
                // error is swallowed and the spill task's own
                // result carries the failure.
                let _ = sink.write_all(&chunk[..n]).await;
                if truncated {
                    // Already over the cap; the sink has it.
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
    // Close the pipe so the spill task sees EOF and stops.
    //
    // By **dropping** the write half, not by calling `shutdown` on
    // it. `shutdown` on a `DuplexStream` tears down both halves and
    // discards whatever is still buffered, so the spill task read an
    // empty artifact for a tool that had printed 256 KiB — the
    // failure looked exactly like a tool that had printed nothing,
    // which is the one thing a spill must never be able to
    // misrepresent. The first draft of this test asserted
    // `artifact.bytes == 32 KiB` and got `0`, and the bug was here
    // rather than in the writer.
    drop(sink);
    (kept, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect everything the reader forwards, so the tests can
    /// assert on both halves of the split: the in-memory prefix and
    /// the forwarded stream.
    async fn read_with_sink<R: AsyncRead + Unpin>(
        reader: R,
        max_bytes: u64,
    ) -> (Vec<u8>, bool, Vec<u8>) {
        let (mut sink_read, mut sink_write) = tokio::io::duplex(1024 * 1024);
        let forwarder = tokio::spawn(async move {
            let mut all = Vec::new();
            let _ = sink_read.read_to_end(&mut all).await;
            all
        });
        let (kept, truncated) = bounded_read(reader, max_bytes, &mut sink_write).await;
        drop(sink_write);
        let forwarded = forwarder.await.unwrap_or_default();
        (kept, truncated, forwarded)
    }

    #[tokio::test]
    async fn bounded_read_under_limit_returns_full() {
        let bytes: Vec<u8> = (0..100).collect();
        let (kept, truncated, forwarded) = read_with_sink(bytes.as_slice(), 1024).await;
        assert_eq!(kept.len(), 100);
        assert!(!truncated);
        assert_eq!(forwarded, bytes, "the sink gets what the record got");
    }

    #[tokio::test]
    async fn bounded_read_at_exact_limit_not_truncated() {
        let bytes: Vec<u8> = vec![0u8; 100];
        let (kept, truncated, _) = read_with_sink(bytes.as_slice(), 100).await;
        assert_eq!(kept.len(), 100);
        assert!(!truncated);
    }

    /// The behaviour D-08 is about, stated as a unit test: past
    /// the cap the reader still forwards. Before the fix it drained
    /// and discarded, which is why an oversized build log left no
    /// trace.
    #[tokio::test]
    async fn bounded_read_forwards_past_the_cap_it_truncates_at() {
        let bytes: Vec<u8> = (0..200).collect();
        let (kept, truncated, forwarded) = read_with_sink(bytes.as_slice(), 100).await;
        assert_eq!(kept.len(), 100);
        assert!(truncated);
        assert_eq!(kept, (0..100).collect::<Vec<u8>>());
        assert_eq!(
            forwarded.len(),
            200,
            "the sink must receive the whole stream, not the cap: an \
             artifact that stops where the record's copy stops is not \
             a spill, it is the same truncation written twice"
        );
        assert_eq!(forwarded, bytes);
    }

    #[tokio::test]
    async fn bounded_read_empty_input() {
        let (kept, truncated, forwarded) = read_with_sink([].as_slice(), 1024).await;
        assert!(kept.is_empty());
        assert!(!truncated);
        assert!(forwarded.is_empty());
    }
}
