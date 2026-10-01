//! The approval refusal is *observable* (0B.9 C4).
//!
//! `RuntimeCommand::RequestApproval` has no handler in 0B: there
//! is no approval principal, no out-of-band delivery channel, and
//! no durable `ApprovalStore`. Before this sub-phase the command
//! did not exist at all, so the boundary was correct only by
//! absence — nothing exercised it, and `next_actions` advertised
//! an action that no code path could service.
//!
//! These tests pin the four properties that make the refusal a
//! real answer rather than a missing arm:
//!
//! 1. it is typed (`Unsupported`, not `Other`/`InvalidInput`);
//! 2. it explains the boundary in the message, so an agent can
//!    report *why* it cannot proceed;
//! 3. it mutates nothing — no projection bump, no event, no
//!    approval record;
//! 4. the job still parks at `ReadyForApproval`, with
//!    `reconcile` reporting `NeedsActorDecision`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    JobProjection, JobState, MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName,
    PackageRevision, PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_policy::ApprovalCategory;
use ironmaint_runtime::{
    ActionBlocker, Clock, FixedClock, HumanAction, QueryResult, ReconcileOutcome, RuntimeCommand,
    RuntimeErrorKind, RuntimeQuery, RuntimeService,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EventStore, ProjectionStore};
use time::OffsetDateTime;

const CATEGORIES: [ApprovalCategory; 6] = [
    ApprovalCategory::HumanReview,
    ApprovalCategory::PolicyException,
    ApprovalCategory::ExternalMutation,
    ApprovalCategory::Publication,
    ApprovalCategory::Signing,
    ApprovalCategory::SecuritySensitive,
];

fn fixed_clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock::new(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    ))
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("foo").unwrap(),
    )
}

fn build_service(store: &Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        Arc::clone(store),
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

/// A job parked at `ReadyForApproval` with a real active
/// candidate — the exact point where an orchestrator is supposed
/// to ask a human.
async fn seed_at_ready_for_approval(store: &Arc<MockStore>) -> JobId {
    let job_id = JobId::new();
    let projection = JobProjection {
        job: MaintenanceJob::new(
            job_id,
            package(),
            MaintenanceEventId::new(),
            OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        ),
        state: JobState::ReadyForApproval,
        active_candidate: None,
        version: 0,
        updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    };
    store.put_projection(&projection, 0).await.expect("seed");

    let candidate = SourceCandidate::new(
        job_id,
        PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap()),
        RepositoryRef::new(
            VcsKind::Git,
            url::Url::parse("https://example.invalid/repo.git").unwrap(),
        )
        .unwrap(),
        GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap(),
        GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap(),
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    let candidate_id = store.put_source_candidate(&candidate).await.expect("put");
    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.active_candidate = Some(candidate_id);
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("attach candidate");
    job_id
}

// ---------------------------------------------------------------------------
// 1 + 2: the refusal is typed and self-explaining
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_approval_is_refused_with_a_typed_unsupported_error() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);
    let job_id = seed_at_ready_for_approval(&store).await;

    let err = svc
        .handle_command(RuntimeCommand::RequestApproval {
            job_id,
            category: ApprovalCategory::HumanReview,
        })
        .await
        .expect_err("RequestApproval must be refused");
    assert_eq!(
        err.kind,
        RuntimeErrorKind::Unsupported,
        "a structural refusal is not an input error and not a bug: {err:?}"
    );
}

#[tokio::test]
async fn the_refusal_names_the_category_and_the_boundary() {
    // The message is the only thing an orchestrator can show a
    // human. "unsupported" alone tells nobody whether to file a bug
    // or hand the job over, so the refusal has to say which
    // approval was wanted and that a person is the missing party.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);
    let job_id = seed_at_ready_for_approval(&store).await;

    for category in CATEGORIES {
        let err = svc
            .handle_command(RuntimeCommand::RequestApproval {
                job_id,
                category: category.clone(),
            })
            .await
            .expect_err("refused");
        let message = err.message.to_lowercase();
        assert!(
            message.contains("human") || message.contains("principal"),
            "{category}: the refusal must name the missing party: {err}"
        );
        assert!(
            message.contains(&category.name().to_lowercase()),
            "{category}: the refusal must echo the requested category: {err}"
        );
        assert!(
            message.contains("readyforapproval") || message.contains("ready_for_approval"),
            "{category}: the refusal must tell the caller where the job is parked: {err}"
        );
    }
}

#[tokio::test]
async fn the_refusal_is_identical_for_a_job_that_does_not_exist() {
    // If the refusal consulted the store, a nonexistent job would
    // produce `InvalidInput` instead. Uniform refusal is evidence
    // that the handler really is pure — and it is also the right
    // behaviour: a request cannot be honoured for any job, so
    // there is no reason to leak whether one exists.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);

    let err = svc
        .handle_command(RuntimeCommand::RequestApproval {
            job_id: JobId::new(),
            category: ApprovalCategory::Publication,
        })
        .await
        .expect_err("refused");
    assert_eq!(err.kind, RuntimeErrorKind::Unsupported);
}

// ---------------------------------------------------------------------------
// 3: the refusal mutates nothing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_refused_request_leaves_the_projection_byte_identical() {
    // A refusal that still bumped the projection version or
    // rewrote `updated_at` would be a write disguised as a
    // no-op: it would make the job look touched by a decision
    // that never happened.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);
    let job_id = seed_at_ready_for_approval(&store).await;

    let before = store.get_projection(job_id).await.expect("before");
    let _ = svc
        .handle_command(RuntimeCommand::RequestApproval {
            job_id,
            category: ApprovalCategory::Signing,
        })
        .await;
    let after = store.get_projection(job_id).await.expect("after");

    assert_eq!(after.version, before.version, "version must not move");
    assert_eq!(after.state, before.state, "state must not move");
    assert_eq!(after.updated_at, before.updated_at, "mtime must not move");
    assert_eq!(
        after.active_candidate, before.active_candidate,
        "the active candidate must not move"
    );
}

#[tokio::test]
async fn a_refused_request_appends_no_event() {
    // No `JobEvent::ApprovalRequested`, no `ApprovalRecorded`, no
    // placeholder. The event log is the audit trail; a refused
    // request belongs in the caller's error handling, not in the
    // job's history as though it were a fact about the job.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);
    let job_id = seed_at_ready_for_approval(&store).await;

    let before = store
        .list_events_for_job(job_id, 0, None)
        .await
        .expect("events before");
    let next_sequence_before = store.next_sequence(job_id).await.expect("seq before");

    let _ = svc
        .handle_command(RuntimeCommand::RequestApproval {
            job_id,
            category: ApprovalCategory::ExternalMutation,
        })
        .await;

    let after = store
        .list_events_for_job(job_id, 0, None)
        .await
        .expect("events after");
    assert_eq!(
        after.len(),
        before.len(),
        "a refusal must not append an event: {after:?}"
    );
    assert_eq!(
        store.next_sequence(job_id).await.expect("seq after"),
        next_sequence_before,
        "the sequence counter must not advance"
    );
}

#[tokio::test]
async fn repeated_refusals_are_identical_and_never_accumulate() {
    // An orchestrator may retry after the refusal. Retrying must be
    // free: no state to unwind, no rows to clean up, and the same
    // answer every time.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);
    let job_id = seed_at_ready_for_approval(&store).await;

    let mut messages = Vec::new();
    for _ in 0..5 {
        let err = svc
            .handle_command(RuntimeCommand::RequestApproval {
                job_id,
                category: ApprovalCategory::PolicyException,
            })
            .await
            .expect_err("refused");
        messages.push(err.message);
    }
    assert!(
        messages.windows(2).all(|w| w[0] == w[1]),
        "every refusal must be the same: {messages:?}"
    );
    assert!(
        store
            .list_events_for_job(job_id, 0, None)
            .await
            .expect("events")
            .is_empty(),
        "five refusals must leave no trace"
    );
}

// ---------------------------------------------------------------------------
// 4: the job still parks, and the advertised action is the one refused
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reconcile_still_parks_at_ready_for_approval() {
    // The refusal must not become a way to advance the job. With
    // no approvals on file, `ReadyForApproval -> Approved` is
    // blocked on `MissingApproval`, and that is exactly what
    // `reconcile` should keep reporting.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);
    let job_id = seed_at_ready_for_approval(&store).await;

    for _ in 0..3 {
        let _ = svc
            .handle_command(RuntimeCommand::RequestApproval {
                job_id,
                category: ApprovalCategory::HumanReview,
            })
            .await;
        let outcome = svc.reconcile(job_id).await.expect("reconcile");
        assert!(
            matches!(outcome, ReconcileOutcome::NeedsActorDecision { .. }),
            "refusal must not let the job advance, got {outcome:?}"
        );
        let projection = store.get_projection(job_id).await.expect("projection");
        assert_eq!(projection.state, JobState::ReadyForApproval);
    }
}

#[tokio::test]
async fn next_actions_advertises_exactly_the_action_that_is_refused() {
    // This is the pairing that makes the workflow honest. If
    // `next_actions` said nothing, the agent would not know an
    // approval is the thing it needs — which is the exit checkpoint
    // in SKILL.md. So: report the move, on the field that says "this
    // is a person's to make", and answer a command for it with the
    // refusal that says a human is required.
    //
    // The C3 split changed the field, not the pairing. `RequestApproval`
    // used to be an `AllowedAction`, which claimed the caller "can take
    // it right now" — and then refused. It is now a `HumanAction`, which
    // says the opposite thing accurately.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);
    let job_id = seed_at_ready_for_approval(&store).await;

    let QueryResult::NextActions(actions) = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions")
    else {
        panic!("wrong QueryResult variant");
    };

    assert_eq!(
        actions.requires_human,
        Some(HumanAction::ApproveRelease),
        "ReadyForApproval parks on exactly the approval"
    );
    assert!(
        actions.allowed.is_empty(),
        "no move here is the agent's to take: {:?}",
        actions.allowed
    );
    assert!(
        !actions
            .blockers
            .iter()
            .any(|b| matches!(b, ActionBlocker::GatePending { .. })),
        "the gate is satisfied by the time a job parks here: {:?}",
        actions.blockers
    );

    // And the advertised action is exactly the one that refuses.
    let err = svc
        .handle_command(RuntimeCommand::RequestApproval {
            job_id,
            category: ApprovalCategory::HumanReview,
        })
        .await
        .expect_err("refused");
    assert_eq!(err.kind, RuntimeErrorKind::Unsupported);
}

#[tokio::test]
async fn the_refusal_does_not_depend_on_candidate_presence() {
    // A job with no active candidate and one with a full candidate
    // get the same answer, so the refusal cannot be mistaken for a
    // precondition failure the caller could fix by capturing
    // something first.
    let store = Arc::new(MockStore::new());
    let svc = build_service(&store);

    let bare = JobId::new();
    store
        .put_projection(
            &JobProjection {
                job: MaintenanceJob::new(
                    bare,
                    package(),
                    MaintenanceEventId::new(),
                    OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                ),
                state: JobState::ReadyForApproval,
                active_candidate: None,
                version: 0,
                updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            },
            0,
        )
        .await
        .expect("seed bare");

    let with_candidate = seed_at_ready_for_approval(&store).await;
    for job_id in [bare, with_candidate] {
        let err = svc
            .handle_command(RuntimeCommand::RequestApproval {
                job_id,
                category: ApprovalCategory::SecuritySensitive,
            })
            .await
            .expect_err("refused");
        assert_eq!(err.kind, RuntimeErrorKind::Unsupported, "job {job_id}");
    }

    // The active candidate the job carried is irrelevant to the
    // outcome; assert it really was set so the test is not
    // vacuous.
    let projection = store
        .get_projection(with_candidate)
        .await
        .expect("projection");
    assert!(projection.active_candidate.is_some());
    assert!(
        store
            .get_projection(bare)
            .await
            .expect("bare projection")
            .active_candidate
            .is_none()
    );
}
