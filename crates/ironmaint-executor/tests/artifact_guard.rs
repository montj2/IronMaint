//! Tests for `JobArtifactGuard` (PHASE-0B.md §15).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::JobId;
use ironmaint_executor::limits::LimitsConfig;
use ironmaint_executor::{
    ExecutorErrorKind, JobArtifactGuard, factory_from_config, permissive_guard,
};

#[test]
fn guard_caps_count() {
    let g = JobArtifactGuard::new(JobId::new(), 3, u64::MAX);
    g.try_record(10).expect("1");
    g.try_record(20).expect("2");
    g.try_record(30).expect("3");
    let err = g.try_record(40).expect_err("4th should fail");
    assert_eq!(
        err.kind,
        ExecutorErrorKind::ArtifactBudgetExceeded { cap: "count" }
    );
    let s = g.snapshot();
    // On failure, count must NOT advance.
    assert_eq!(s.count, 3);
    assert_eq!(s.bytes, 60);
}

#[test]
fn guard_caps_bytes() {
    let g = JobArtifactGuard::new(JobId::new(), u32::MAX, 100);
    g.try_record(40).expect("1");
    g.try_record(40).expect("2");
    let err = g.try_record(40).expect_err("3rd overflow");
    assert_eq!(
        err.kind,
        ExecutorErrorKind::ArtifactBudgetExceeded { cap: "bytes" }
    );
    let s = g.snapshot();
    // On failure, bytes must NOT advance (and count must roll back).
    assert_eq!(s.bytes, 80);
    assert_eq!(s.count, 2);
}

#[test]
fn guard_isolated_per_job() {
    let a = permissive_guard(JobId::new());
    let b = permissive_guard(JobId::new());
    assert_ne!(a.job_id(), b.job_id());
    // Permissive caps accept arbitrary writes, but each guard
    // tracks its own job. We can't easily exercise the per-job
    // cap isolation without using a non-permissive factory —
    // assert instead that two factories from the same config
    // produce independent guards.
    let cfg = LimitsConfig {
        per_job_max_artifacts: 2,
        per_job_max_bytes: 100,
        per_artifact_max_bytes: 256 * 1024 * 1024,
    };
    let factory = factory_from_config(cfg.clone());
    let job_a = JobId::new();
    let job_b = JobId::new();
    let ga = factory(job_a);
    let gb = factory(job_b);
    ga.try_record(50).unwrap();
    ga.try_record(50).unwrap();
    // ga is full; gb is untouched.
    assert!(ga.try_record(1).is_err());
    assert_eq!(gb.snapshot().count, 0);
    assert_eq!(gb.snapshot().bytes, 0);
}

#[test]
fn guard_snapshot_reflects_writes() {
    let g = JobArtifactGuard::new(JobId::new(), 10, 1024);
    let s0 = g.snapshot();
    assert_eq!(
        s0,
        ironmaint_executor::JobArtifactUsage { count: 0, bytes: 0 }
    );
    g.try_record(100).unwrap();
    g.try_record(200).unwrap();
    let s = g.snapshot();
    assert_eq!(s.count, 2);
    assert_eq!(s.bytes, 300);
}

#[test]
fn permissive_guard_accepts_arbitrary_writes() {
    let g = permissive_guard(JobId::new());
    for i in 0..1000 {
        g.try_record(i).expect("permissive should not fail");
    }
    let s = g.snapshot();
    assert_eq!(s.count, 1000);
    assert_eq!(s.bytes, (0..1000).sum::<u64>());
}

#[test]
fn factory_from_config_propagates_caps() {
    let cfg = LimitsConfig {
        per_job_max_artifacts: 7,
        per_job_max_bytes: 700,
        per_artifact_max_bytes: 256 * 1024 * 1024,
    };
    let f = factory_from_config(cfg);
    let g = f(JobId::new());
    assert_eq!(g.max_count(), 7);
    assert_eq!(g.max_bytes(), 700);
}

#[test]
fn job_id_is_preserved() {
    let jid = JobId::new();
    let g = JobArtifactGuard::new(jid, 4, 1024);
    assert_eq!(g.job_id(), jid);
}
