//! `JobArtifactGuard` — per-job byte/count caps (PHASE-0B.md §15).
//!
//! Every tool run attaches zero or more artifacts to the evidence
//! ledger. The guard enforces that the *aggregate* per-job count
//! and bytes stay within [`LimitsConfig`] caps. The guard lives in
//! the executor crate (not in `ironmaint-artifacts`) because the
//! store has no path to a `JobId` — pushing the cap in would force
//! a `job_id` parameter on every `put_*` call or a job-aware
//! facade.
//!
//! Counters use atomics with compare-exchange loops so concurrent
//! writers from concurrent retries don't overshoot caps. When the
//! byte cap fails, the count roll-back is best-effort: the count
//! bump is preserved by design (the artifact attempt happened; we
//! just refused to retain its bytes), so a later test that drains
//! the cap can still observe the attempt.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use ironmaint_core::JobId;
use uuid::Uuid;

use crate::error::{ExecutorError, ExecutorErrorKind};
use crate::limits::LimitsConfig;

/// Snapshot of a guard's current usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobArtifactUsage {
    pub count: u32,
    pub bytes: u64,
}

/// Per-job counter for artifact count + aggregate bytes.
///
/// Constructed by the runtime at job creation. The factory pattern
/// ([`JobArtifactGuardFactory`]) lets the runtime own how caps are
/// chosen per-job; the executor just consults it.
pub struct JobArtifactGuard {
    job_id: JobId,
    run_id: Uuid,
    count: AtomicU32,
    bytes: AtomicU64,
    max_count: u32,
    max_bytes: u64,
}

impl std::fmt::Debug for JobArtifactGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobArtifactGuard")
            .field("job_id", &self.job_id)
            .field("run_id", &self.run_id)
            .field("count", &self.count.load(Ordering::SeqCst))
            .field("bytes", &self.bytes.load(Ordering::SeqCst))
            .field("max_count", &self.max_count)
            .field("max_bytes", &self.max_bytes)
            .finish()
    }
}

impl JobArtifactGuard {
    /// Construct a guard with explicit caps.
    #[must_use]
    pub fn new(job_id: JobId, max_count: u32, max_bytes: u64) -> Self {
        Self {
            job_id,
            run_id: Uuid::now_v7(),
            count: AtomicU32::new(0),
            bytes: AtomicU64::new(0),
            max_count,
            max_bytes,
        }
    }

    /// Construct a guard from a [`LimitsConfig`].
    #[must_use]
    pub fn from_config(job_id: JobId, config: &LimitsConfig) -> Self {
        Self::new(
            job_id,
            config.per_job_max_artifacts,
            config.per_job_max_bytes,
        )
    }

    /// Construct a guard with caps set to the platform maximums.
    /// Intended for tests and dev builds; production uses
    /// [`Self::from_config`].
    #[must_use]
    pub fn permissive(job_id: JobId) -> Self {
        Self::new(job_id, u32::MAX, u64::MAX)
    }

    #[must_use]
    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    #[must_use]
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    /// Record a write of `n` bytes. On success both counters
    /// increment atomically (count strictly, bytes by `n`); on
    /// failure neither counter is advanced.
    ///
    /// # Errors
    /// - `ArtifactBudgetExceeded { cap: "count" }` when the count
    ///   cap would be exceeded.
    /// - `ArtifactBudgetExceeded { cap: "bytes" }` when the bytes
    ///   cap would be exceeded.
    pub fn try_record(&self, n: u64) -> Result<(), ExecutorError> {
        self.try_bump_count()?;
        if let Err(e) = self.try_bump_bytes(n) {
            self.count.fetch_sub(1, Ordering::SeqCst);
            return Err(e);
        }
        Ok(())
    }

    fn try_bump_count(&self) -> Result<(), ExecutorError> {
        loop {
            let cur = self.count.load(Ordering::SeqCst);
            if cur >= self.max_count {
                return Err(ExecutorError::new(
                    ExecutorErrorKind::ArtifactBudgetExceeded { cap: "count" },
                    format!("job {} reached max_count {}", self.job_id, self.max_count),
                ));
            }
            if self
                .count
                .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    fn try_bump_bytes(&self, n: u64) -> Result<(), ExecutorError> {
        loop {
            let cur = self.bytes.load(Ordering::SeqCst);
            let new = cur.saturating_add(n);
            if new > self.max_bytes {
                return Err(ExecutorError::new(
                    ExecutorErrorKind::ArtifactBudgetExceeded { cap: "bytes" },
                    format!(
                        "job {} reached max_bytes {} (adding {} of {})",
                        self.job_id, self.max_bytes, n, cur
                    ),
                ));
            }
            if self
                .bytes
                .compare_exchange(cur, new, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    /// Snapshot current usage.
    #[must_use]
    pub fn snapshot(&self) -> JobArtifactUsage {
        JobArtifactUsage {
            count: self.count.load(Ordering::SeqCst),
            bytes: self.bytes.load(Ordering::SeqCst),
        }
    }

    /// Configured count cap.
    #[must_use]
    pub fn max_count(&self) -> u32 {
        self.max_count
    }

    /// Configured bytes cap.
    #[must_use]
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
}

/// Factory the executor holds for constructing per-job guards
/// (PHASE-0B.md §15: caps outlive transient executor errors and
/// are reused across retries).
pub type JobArtifactGuardFactory = Arc<dyn Fn(JobId) -> Arc<JobArtifactGuard> + Send + Sync>;

/// Factory default: every job gets a permissive guard
/// (caps at the platform maximums). Production wiring replaces
/// this with a config-driven factory.
#[must_use]
pub fn permissive_guard(job_id: JobId) -> Arc<JobArtifactGuard> {
    Arc::new(JobArtifactGuard::permissive(job_id))
}

/// Wrap a guard factory from a [`LimitsConfig`].
#[must_use]
pub fn factory_from_config(config: LimitsConfig) -> JobArtifactGuardFactory {
    Arc::new(move |job_id| Arc::new(JobArtifactGuard::from_config(job_id, &config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_guard_has_zero_usage() {
        let g = JobArtifactGuard::new(JobId::new(), 4, 1024);
        let s = g.snapshot();
        assert_eq!(s.count, 0);
        assert_eq!(s.bytes, 0);
    }

    #[test]
    fn permissive_accepts_arbitrary_writes() {
        let g = JobArtifactGuard::permissive(JobId::new());
        for i in 0..1000 {
            g.try_record(i).expect("permissive should not fail");
        }
        let s = g.snapshot();
        assert_eq!(s.count, 1000);
        assert_eq!(s.bytes, (0..1000).sum::<u64>());
    }

    #[test]
    fn from_config_uses_caps() {
        let cfg = LimitsConfig {
            per_job_max_artifacts: 5,
            per_job_max_bytes: 100,
            per_artifact_max_bytes: 50,
        };
        let g = JobArtifactGuard::from_config(JobId::new(), &cfg);
        assert_eq!(g.max_count(), 5);
        assert_eq!(g.max_bytes(), 100);
    }
}
