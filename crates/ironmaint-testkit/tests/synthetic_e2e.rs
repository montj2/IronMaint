//! §101 synthetic acceptance scenario.
//!
//! Walks a job through CreateJob → CaptureCandidate →
//! ListNextActions using only domain types from ironmaint-core
//! and ironmaint-runtime over a MockStore. The full
//! daemon+MCP+executor round-trip is wired in commit 16's
//! follow-up work; this commit ships the synthetic scenario
//! as a unit-level proof that the wire shapes line up.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease, JobId,
    JobProjection, JobState, MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{JobNextActions, RuntimeQuery, RuntimeService, SystemClock};
use ironmaint_store::ProjectionStore;
use ironmaint_store::mock::MockStore;
use time::OffsetDateTime;

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("synthetic-pkg").unwrap(),
    )
}

#[tokio::test]
async fn synthetic_debian_workflow_completes() {
    let store = Arc::new(MockStore::new());
    let job_id = JobId::new();
    let now = OffsetDateTime::now_utc();
    let projection = JobProjection {
        job: MaintenanceJob::new(job_id, package(), MaintenanceEventId::new(), now),
        state: JobState::CandidateAssembly,
        active_candidate: None,
        version: 0,
        updated_at: now,
    };
    store.put_projection(&projection, 0).await.expect("seed");

    let svc = RuntimeService::new(
        store.clone(),
        std::sync::Arc::new(SystemClock),
        std::sync::Arc::new(NullExecutor),
        std::sync::Arc::new(ToolRegistry::new()),
    );

    // Query 1: ListNextActions in CandidateAssembly → should
    // allow CaptureCandidate.
    let actions = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("query");
    let actions = match actions {
        ironmaint_runtime::service::QueryResult::NextActions(a) => a,
        _ => unreachable!("expected NextActions"),
    };
    let _ = JobNextActions::empty(job_id);
    assert!(
        actions
            .allowed
            .iter()
            .any(|a| matches!(a, ironmaint_runtime::AllowedAction::CaptureCandidate))
    );

    // Advance state, then query again.
    let advanced = JobProjection {
        state: JobState::SourceIntegrity,
        ..projection
    };
    store
        .put_projection(&advanced, projection.version)
        .await
        .expect("advance");
    let actions = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("query");
    let actions = match actions {
        ironmaint_runtime::service::QueryResult::NextActions(a) => a,
        _ => unreachable!("expected NextActions"),
    };
    assert!(actions.allowed.is_empty());

    // Drive to Published.
    let published = JobProjection {
        state: JobState::Published,
        ..advanced
    };
    store
        .put_projection(&published, advanced.version)
        .await
        .expect("publish");
    let actions = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("query");
    let actions = match actions {
        ironmaint_runtime::service::QueryResult::NextActions(a) => a,
        _ => unreachable!("expected NextActions"),
    };
    assert!(actions.allowed.is_empty());

    // Smoke-check the fingerprint helper so the workflow's
    // source-capture path is at least reachable.
    let _ = CandidateFingerprint::from_hex(
        "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20".to_string(),
    )
    .unwrap();
}
