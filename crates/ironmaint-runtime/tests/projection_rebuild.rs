//! §102 item 3 — "Job projections can be rebuilt from events."
//!
//! D-14 was that the claim was false for every job the runtime had
//! ever created. `handle_create_job` seeded the projection with
//! `put_projection` and then appended a bare `JobEvent::Domain`
//! reference, and `rebuild_projection` refused any log not beginning
//! with a `Transitioned`. The projection's initial contents existed
//! only in that write and in no event, so the log could not say what
//! the job looked like at birth.
//!
//! Every pre-existing rebuild test hand-assembled its log starting
//! from a `Transitioned`, which is why this went unnoticed. These
//! tests drive the **real** creation path against a **real** SQLite
//! store, so the log under test is the one the runtime actually
//! writes. A regression in what `handle_create_job` emits fails here
//! rather than in production, where the symptom is
//! `ironmaintctl rebuild-projections` reporting every job as an
//! error and rebuilding nothing.

// `panic` is allowed for the same reason as in the other runtime
// test files: a `let … else` on a `QueryResult`/`ReconcileOutcome`
// variant match has no other way to fail loudly, and a
// silently-defaulted value would make the assertions below pass for
// the wrong reason.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, JobId, JobProjection, JobState,
    PackageIdentity, PackageName,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{
    FixedClock, OrchestratorRef, RuntimeCommand, RuntimeQuery, RuntimeService,
};
use ironmaint_store::{EventStore, ProjectionStore};
use ironmaint_store_sqlite::SqliteStore;
use time::OffsetDateTime;

fn service(store: Arc<SqliteStore>) -> RuntimeService<SqliteStore, NullExecutor> {
    RuntimeService::new(
        store,
        Arc::new(FixedClock::new(
            OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        )),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("sid").unwrap(),
        ),
        PackageName::new("synthetic-pkg").unwrap(),
    )
}

async fn create_job(svc: &RuntimeService<SqliteStore, NullExecutor>) -> JobId {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create job");
    // The side effect names the job; there is no other way to learn
    // the generated id from a command result.
    let side = &result.side_effects[0];
    let id_part = side
        .split_whitespace()
        .next()
        .expect("a side effect starting with `job:`")
        .trim_start_matches("job:");
    id_part
        .parse::<JobId>()
        .expect("a UUID job id in the side effect")
}

/// Seed a projection row without any events — the shape of a job whose
/// log has been lost or truncated. Needed because the `events` table
/// carries a foreign key to the job, so an event cannot be appended
/// for a job the database has never heard of.
async fn seed_projection_row(store: &SqliteStore, job_id: JobId, state: JobState) {
    let at = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let projection = JobProjection {
        job: ironmaint_core::MaintenanceJob::new(
            job_id,
            package(),
            ironmaint_core::MaintenanceEventId::new(),
            at,
        ),
        state,
        active_candidate: None,
        version: 0,
        updated_at: at,
    };
    store
        .put_projection(&projection, 0)
        .await
        .expect("seed projection row");
}

/// The bug. A job that has never transitioned still has a complete,
/// replayable log, because `JobCreated` carries its birth projection.
#[tokio::test]
async fn a_never_transitioned_job_rebuilds_from_its_log() {
    let store = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let svc = service(store.clone());
    let job_id = create_job(&svc).await;

    let stored = store.get_projection(job_id).await.expect("stored");
    let rebuilt = store
        .rebuild_projection(job_id)
        .await
        .expect("a fresh job's log must rebuild");

    assert_eq!(rebuilt.state, stored.state, "state survives the round trip");
    assert_eq!(rebuilt.version, stored.version, "version survives");
    assert_eq!(rebuilt.job.id, stored.job.id, "the same job");
    assert_eq!(
        rebuilt.active_candidate, stored.active_candidate,
        "a fresh job has no active candidate"
    );
    assert_eq!(
        rebuilt.updated_at, stored.updated_at,
        "the creation projection's own stamp, not the envelope's"
    );
}

/// And it still does after the job has moved, so replay is not merely
/// reading back the seed.
#[tokio::test]
async fn a_transitioned_job_rebuilds_to_where_it_actually_is() {
    let store = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let svc = service(store.clone());
    let job_id = create_job(&svc).await;

    svc.handle_command(RuntimeCommand::RequestApproval {
        job_id,
        category: ironmaint_policy::ApprovalCategory::HumanReview,
    })
    .await
    .expect_err("approval is refused in 0B — the call must not panic");

    let stored = store.get_projection(job_id).await.expect("stored");
    let rebuilt = store
        .rebuild_projection(job_id)
        .await
        .expect("rebuild after a command that writes an event");

    assert_eq!(rebuilt.state, JobState::EventDetected);
    assert_eq!(rebuilt.version, stored.version);
    assert_eq!(rebuilt.state, stored.state);
}

/// The refusal the store still owes. A log whose first event is a
/// bare `Domain` reference genuinely carries no projection, and
/// reporting that plainly is better than inventing one — the error is
/// the operator's evidence that the log is not replayable.
#[tokio::test]
async fn a_log_that_carries_no_projection_is_refused_with_a_reason() {
    use ironmaint_core::MaintenanceEventId;
    use ironmaint_state::JobEvent;
    use ironmaint_store::EventEnvelope;

    let store = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let job_id = JobId::new();
    seed_projection_row(&store, job_id, JobState::EventDetected).await;
    store
        .append_event(&EventEnvelope::new(
            MaintenanceEventId::new(),
            job_id,
            1,
            OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            JobEvent::Domain(ironmaint_core::DomainEventId::new()),
        ))
        .await
        .expect("append");

    let err = store
        .rebuild_projection(job_id)
        .await
        .expect_err("a bare Domain reference cannot seed a replay");
    assert_eq!(*err.kind(), ironmaint_store::StoreErrorKind::Corrupt);
    let msg = err.to_string();
    assert!(
        msg.contains("Domain reference") && msg.contains("seed"),
        "the error must say what is wrong and what was expected: {msg}"
    );
}

/// A log with no events at all is a different failure — nothing to
/// replay — and must not be reported as corruption.
#[tokio::test]
async fn a_job_with_no_events_is_not_found_rather_than_corrupt() {
    let store = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let err = store
        .rebuild_projection(JobId::new())
        .await
        .expect_err("no events means no projection");
    assert_eq!(*err.kind(), ironmaint_store::StoreErrorKind::NotFound);
}

/// `JobCreated` carries the whole `JobProjection`, so it has to
/// survive SQLite's JSON encoding with the same fidelity `Transitioned`
/// already has. A lossily-encoded seed would produce a projection
/// that *looks* rebuilt but names the wrong job.
#[tokio::test]
async fn the_job_created_event_round_trips_through_sqlite() {
    use ironmaint_state::JobEvent;

    let store = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let svc = service(store.clone());
    let job_id = create_job(&svc).await;
    let stored = store.get_projection(job_id).await.expect("stored");

    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("events");
    let first = events.first().expect("CreateJob writes events");
    let JobEvent::JobCreated(seed) = &first.event else {
        panic!(
            "the first event must be the replay seed, got {:?}",
            first.event
        );
    };
    assert_eq!(seed.job.id, job_id);
    assert_eq!(seed.state, JobState::EventDetected);
    assert_eq!(seed.version, 0);
    assert_eq!(seed, &stored, "the seed is the projection as written");

    // And the payload decodes back to the same value on a second
    // read, which is the property the rebuild path depends on.
    let reread = store
        .list_events_for_job(job_id, 1, Some(1))
        .await
        .expect("reread");
    assert_eq!(reread[0].event, first.event);

    // The audit reference follows the seed, not the other way round.
    assert!(
        matches!(events[1].event, JobEvent::Domain(_)),
        "CreateJob's second event is the Domain reference, got {:?}",
        events[1].event
    );
}

/// A log written before 0B.10 has no `JobCreated` and still begins
/// with a `Transitioned`. That path must keep working — this is
/// existing data, not a bug to be cleaned up.
#[tokio::test]
async fn a_legacy_log_seeded_by_a_transition_still_rebuilds() {
    use ironmaint_core::{MaintenanceEventId, MaintenanceJob};
    use ironmaint_state::{JobEvent, StateTransitioned, Transition};
    use ironmaint_store::EventEnvelope;
    use time::macros::datetime;

    let store = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let job_id = JobId::new();
    seed_projection_row(&store, job_id, JobState::EventDetected).await;
    let at = datetime!(2026-01-01 00:00:00 UTC);
    let job = MaintenanceJob::new(job_id, package(), MaintenanceEventId::new(), at);
    let after = JobProjection {
        job,
        state: JobState::Intake,
        active_candidate: None,
        version: 1,
        updated_at: at,
    };
    let transitioned = StateTransitioned::new(
        Transition {
            from: JobState::EventDetected,
            to: JobState::Intake,
            rule_index: 0,
        },
        after,
        at,
    );
    store
        .append_event(&EventEnvelope::new(
            MaintenanceEventId::new(),
            job_id,
            1,
            at,
            JobEvent::Transitioned(transitioned),
        ))
        .await
        .expect("append");

    let rebuilt = store
        .rebuild_projection(job_id)
        .await
        .expect("a pre-0B.10 log must still rebuild");
    assert_eq!(rebuilt.state, JobState::Intake);
    assert_eq!(rebuilt.version, 1);
}

/// The query side of the same claim: `job.get` reads the projection
/// row, and the row and the log must agree. If they disagreed, an
/// operator comparing them would have no way to tell which is right.
#[tokio::test]
async fn the_stored_row_and_the_rebuilt_projection_agree() {
    let store = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let svc = service(store.clone());
    let job_id = create_job(&svc).await;

    let ironmaint_runtime::QueryResult::Projection(queried) = svc
        .handle_query(RuntimeQuery::GetProjection { job_id })
        .await
        .expect("projection query")
    else {
        panic!("wrong QueryResult variant");
    };
    let stored: JobProjection = serde_json::from_value(queried).expect("projection decodes");
    let rebuilt = store.rebuild_projection(job_id).await.expect("rebuild");

    assert_eq!(
        rebuilt, stored,
        "the row and the log must agree, or the escape hatch cannot be trusted"
    );
}
