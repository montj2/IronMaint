//! Runtime service smoke tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, JobId, JobProjection, JobState,
    MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName,
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
        PackageName::new("foo").unwrap(),
    )
}

fn projection(state: JobState) -> JobProjection {
    let now = OffsetDateTime::now_utc();
    JobProjection {
        job: MaintenanceJob::new(JobId::new(), package(), MaintenanceEventId::new(), now),
        state,
        active_candidate: None,
        version: 0,
        updated_at: now,
    }
}

#[tokio::test]
async fn list_next_actions_for_event_detected_allows_capture() {
    let store = Arc::new(MockStore::new());
    let job_id = JobId::new();
    let mut p = projection(JobState::EventDetected);
    p.job.id = job_id;
    store.put_projection(&p, 0).await.expect("put projection");
    let svc = RuntimeService::new(
        store,
        Arc::new(SystemClock),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    );
    let result = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("query");
    match result {
        ironmaint_runtime::service::QueryResult::NextActions(actions) => {
            assert_eq!(actions.job_id, job_id);
            assert!(!actions.allowed.is_empty());
        }
        _ => unreachable!("expected NextActions"),
    }
}

#[tokio::test]
async fn list_next_actions_for_published_is_empty() {
    let store = Arc::new(MockStore::new());
    let job_id = JobId::new();
    let mut p = projection(JobState::Published);
    p.job.id = job_id;
    store.put_projection(&p, 0).await.expect("put projection");
    let svc = RuntimeService::new(
        store,
        Arc::new(SystemClock),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    );
    let result = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("query");
    match result {
        ironmaint_runtime::service::QueryResult::NextActions(actions) => {
            assert!(actions.allowed.is_empty());
        }
        _ => unreachable!("expected NextActions"),
    }
}

#[tokio::test]
async fn empty_job_next_actions_has_terminal_blocker() {
    let actions = JobNextActions::empty(JobId::new());
    assert!(actions.allowed.is_empty());
    assert!(!actions.blockers.is_empty());
}
