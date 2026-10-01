//! 0B.10 C2 — exceptional states and resume (0A §21).
//!
//! §21 is one sentence of requirement and one sentence of
//! prohibition:
//!
//! > Returning from `HumanReviewRequired` or `InfrastructureBlocked`
//! > requires a recorded event containing the resume state. Do not
//! > infer the previous state from history at runtime. Record it
//! > explicitly.
//!
//! These tests pin the *positive* half (the record exists, carries
//! the right state, and the engine honours it) and, more importantly,
//! the prohibition: a job with no record cannot be resumed, because
//! the alternative — reconstructing the prior state from the event
//! log — is exactly what the spec forbids. A test that only proved
//! resume works would pass against an implementation that guessed.

// `panic` is allowed here for the same reason it is in the MCP
// dispatcher tests: a `let … else` on a `QueryResult` variant
// match has no other way to fail loudly, and a silently-empty
// `actions` would make the `next_actions` assertions below pass
// for the wrong reason.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_policy::{
    Applicability, Obligation, ObligationStatus, ObligationStrength, PolicyReference,
};
use ironmaint_runtime::{
    Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeErrorKind, RuntimeService,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, EventStore, ObligationStore, ProjectionStore};
use time::OffsetDateTime;
use url::Url;

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("foo").unwrap(),
    )
}

fn fixed_clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock::new(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    ))
}

fn service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

async fn make_job(svc: &RuntimeService<MockStore, NullExecutor>) -> JobId {
    let r = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let rest = r.side_effects[0]
        .strip_prefix("job:")
        .expect("prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("uuid"))
}

async fn seed_candidate(
    store: &MockStore,
    job_id: JobId,
) -> (ironmaint_core::CandidateId, CandidateFingerprint) {
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap());
    let candidate = SourceCandidate::new(job_id, revision, repository, commit, tree, now);
    let fingerprint = candidate.fingerprint().clone();
    let id = store
        .put_source_candidate(&candidate)
        .await
        .expect("put_source_candidate");
    (id, fingerprint)
}

/// Seed a mandatory+applicable obligation for the job's active
/// candidate, in whatever state the caller asks for.
async fn seed_obligation(
    store: &MockStore,
    job_id: JobId,
    fingerprint: &CandidateFingerprint,
    status: ObligationStatus,
) -> ironmaint_core::ObligationId {
    let obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "policy complete",
    )
    .unwrap()
    .with_status(status);
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put")
}

/// A job in `EventDetected` with a candidate and a mandatory
/// obligation, the shape §101 step 18 operates on.
struct Scenario {
    store: Arc<MockStore>,
    svc: RuntimeService<MockStore, NullExecutor>,
    job_id: JobId,
    obligation_id: ironmaint_core::ObligationId,
}

async fn scenario() -> Scenario {
    let store = Arc::new(MockStore::new());
    let svc = service(Arc::clone(&store));
    let job_id = make_job(&svc).await;
    let (_, fingerprint) = seed_candidate(&store, job_id).await;
    let obligation_id =
        seed_obligation(&store, job_id, &fingerprint, ObligationStatus::NotEvaluated).await;
    Scenario {
        store,
        svc,
        job_id,
        obligation_id,
    }
}

async fn events(s: &Scenario) -> Vec<ironmaint_state::JobEvent> {
    s.store
        .list_events_for_job(s.job_id, 1, None)
        .await
        .expect("list")
        .into_iter()
        .map(|e| e.event)
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Entering
// ─────────────────────────────────────────────────────────────────────────────

/// A failed mandatory obligation does **not** move the job.
///
/// This is the 0B.10 C2 decision, and it is the one most likely to
/// be silently reversed by a later reader who assumes §21's
/// "may" means "should". 0B §83 tells the agent to stop when the
/// runtime reports `HumanReviewRequired`; a transition that fired
/// on every policy failure would strand exactly the jobs §101 says
/// the agent should repair and re-run (its step 18 is a failure
/// that step 24 recovers from).
#[tokio::test]
async fn a_failed_mandatory_obligation_does_not_move_the_job() {
    let s = scenario().await;
    let before = s.store.get_projection(s.job_id).await.expect("projection");

    s.store
        .update_obligation(s.obligation_id, &{
            let mut o = s.store.get_obligation(s.obligation_id).await.expect("get");
            o = o.with_status(ObligationStatus::Fail);
            o
        })
        .await
        .expect("update");

    let after = s.store.get_projection(s.job_id).await.expect("projection");
    assert_eq!(
        before.state, after.state,
        "recording a failed obligation must not change workflow state"
    );
    assert_ne!(after.state, JobState::HumanReviewRequired);
}

/// `EnterHumanReview` moves the job and records where it came from.
#[tokio::test]
async fn enter_human_review_records_the_state_it_came_from() {
    let s = scenario().await;
    let before = s.store.get_projection(s.job_id).await.expect("projection");
    assert!(!before.state.is_exceptional());

    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "policy obligation cannot be satisfied automatically".to_string(),
        })
        .await
        .expect("enter human review");

    let after = s.store.get_projection(s.job_id).await.expect("projection");
    assert_eq!(after.state, JobState::HumanReviewRequired);

    // The record is in the log, and it names the *prior* state.
    let recorded: Vec<_> = events(&s)
        .await
        .into_iter()
        .filter_map(|e| match e {
            ironmaint_state::JobEvent::ResumeRecorded(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(recorded.len(), 1, "exactly one resume record");
    assert_eq!(recorded[0].from, JobState::HumanReviewRequired);
    assert_eq!(
        recorded[0].to, before.state,
        "resume target must be the state the job was in *before* entry, not the state it entered"
    );
}

/// Entering twice is refused rather than silently overwriting the
/// recorded resume target with `HumanReviewRequired` itself.
///
/// The engine permits `HumanReviewRequired → HumanReviewRequired`
/// — §21 says "any nonterminal state" — so this hole is only
/// closeable in the runtime. Left open, it produces a job that has
/// recorded "resume to HumanReviewRequired" and can therefore
/// never leave.
#[tokio::test]
async fn re_entering_human_review_is_refused() {
    let s = scenario().await;
    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "first".to_string(),
        })
        .await
        .expect("first entry");

    let err = s
        .svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "second".to_string(),
        })
        .await
        .expect_err("second entry must be refused");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);

    let recorded: Vec<_> = events(&s)
        .await
        .into_iter()
        .filter_map(|e| match e {
            ironmaint_state::JobEvent::ResumeRecorded(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(recorded.len(), 1, "the original record survives");
}

// ─────────────────────────────────────────────────────────────────────────────
// Resuming
// ─────────────────────────────────────────────────────────────────────────────

/// The full loop: enter, resume, and land back where it started.
#[tokio::test]
async fn resume_returns_the_job_to_the_recorded_state() {
    let s = scenario().await;
    let original = s
        .store
        .get_projection(s.job_id)
        .await
        .expect("projection")
        .state;

    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "needs a human".to_string(),
        })
        .await
        .expect("enter");
    assert_eq!(
        s.store.get_projection(s.job_id).await.expect("p").state,
        JobState::HumanReviewRequired
    );

    s.svc
        .handle_command(RuntimeCommand::ResumeJob { job_id: s.job_id })
        .await
        .expect("resume");

    let after = s.store.get_projection(s.job_id).await.expect("projection");
    assert_eq!(after.state, original, "resumed to the recorded state");
}

/// The prohibition, as a test: with the record removed, resume must
/// fail rather than reconstruct the prior state from history.
///
/// The log still contains every `Transitioned` event, so a
/// "helpful" implementation could walk backwards and infer
/// `EventDetected` perfectly well. That is the behaviour 0A §21
/// prohibits, and the only way to keep it from creeping back in is
/// to assert on its absence.
#[tokio::test]
async fn resume_without_a_record_is_refused_rather_than_inferred() {
    let s = scenario().await;
    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "needs a human".to_string(),
        })
        .await
        .expect("enter");

    // Simulate a log that never carried the record — the shape a
    // job entered by some future path (or a pre-0B.10 database)
    // would have. The transition history is intact and would
    // support a confident guess.
    let all = s
        .store
        .list_events_for_job(s.job_id, 1, None)
        .await
        .expect("list");
    let stripped: Vec<_> = all
        .iter()
        .filter(|e| !matches!(e.event, ironmaint_state::JobEvent::ResumeRecorded(_)))
        .collect();
    let fresh = Arc::new(MockStore::new());
    for env in stripped {
        fresh.append_event(env).await.expect("replay stripped log");
    }
    // The projection is stored separately from the log, so it has to
    // be seeded too — otherwise the refusal would come back as a
    // `Store` not-found and the test would pass for the wrong
    // reason.
    let live = s.store.get_projection(s.job_id).await.expect("p");
    fresh
        .put_projection(&live, 0)
        .await
        .expect("seed projection");
    let svc = service(Arc::clone(&fresh));

    let err = svc
        .handle_command(RuntimeCommand::ResumeJob { job_id: s.job_id })
        .await
        .expect_err("must refuse without a record");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
    assert!(
        err.message.contains("not inferred"),
        "the error should say why it refused, got: {}",
        err.message
    );
    assert_eq!(
        fresh.get_projection(s.job_id).await.expect("p").state,
        JobState::HumanReviewRequired,
        "a refused resume must not move the job"
    );
}

/// Resuming a job that is not in an exceptional state is refused.
#[tokio::test]
async fn a_non_exceptional_job_cannot_be_resumed() {
    let s = scenario().await;
    let err = s
        .svc
        .handle_command(RuntimeCommand::ResumeJob { job_id: s.job_id })
        .await
        .expect_err("nothing to resume");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
    assert!(
        err.message.contains("not an exceptional state"),
        "got: {}",
        err.message
    );
}

/// Two excursions work, and the second record wins.
///
/// Taking the *first* matching record would strand a job that is
/// reviewed, resumed, reviewed again from a different state.
#[tokio::test]
async fn a_second_excursion_records_and_uses_the_newer_target() {
    let s = scenario().await;
    let start = s.store.get_projection(s.job_id).await.expect("p").state;

    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "first".to_string(),
        })
        .await
        .expect("enter 1");
    s.svc
        .handle_command(RuntimeCommand::ResumeJob { job_id: s.job_id })
        .await
        .expect("resume 1");
    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "second".to_string(),
        })
        .await
        .expect("enter 2");
    s.svc
        .handle_command(RuntimeCommand::ResumeJob { job_id: s.job_id })
        .await
        .expect("resume 2");

    let after = s.store.get_projection(s.job_id).await.expect("p");
    assert_eq!(after.state, start);
    let records: Vec<_> = events(&s)
        .await
        .into_iter()
        .filter_map(|e| match e {
            ironmaint_state::JobEvent::ResumeRecorded(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 2, "one record per entry");
    assert!(
        records.iter().all(|r| r.to == start),
        "both excursions returned to the same state here: {records:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Projection and discovery
// ─────────────────────────────────────────────────────────────────────────────

/// Replaying a log that carries a resume record reproduces the
/// state the record was written alongside — the record itself moves
/// nothing.
///
/// Built from a hand-assembled log rather than a real job's, because
/// `rebuild_projection` requires the first event to be a
/// `Transitioned` and `CreateJob` always seeds a `Domain` event
/// first. (That is a real defect in the §36 rebuild escape hatch —
/// tracked as D-11 in `doc/DEBT.md` — but fixing it is not this
/// commit's business.) Assembling the log directly isolates the one
/// property under test: `ResumeRecorded` is a pass-through.
#[tokio::test]
async fn a_resume_record_does_not_disturb_projection_replay() {
    use ironmaint_core::{JobProjection, MaintenanceEventId};
    use ironmaint_store::envelope::EventEnvelope;

    let s = scenario().await;
    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "needs a human".to_string(),
        })
        .await
        .expect("enter");

    let log = events(&s).await;
    let transitioned = log
        .iter()
        .find_map(|e| match e {
            ironmaint_state::JobEvent::Transitioned(t) => Some(t.clone()),
            _ => None,
        })
        .expect("the entry transition");
    let record = log
        .iter()
        .find_map(|e| match e {
            ironmaint_state::JobEvent::ResumeRecorded(r) => Some(r.clone()),
            _ => None,
        })
        .expect("the record");

    let fresh = Arc::new(MockStore::new());
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    fresh
        .append_event(&EventEnvelope::new(
            MaintenanceEventId::new(),
            s.job_id,
            1,
            now,
            ironmaint_state::JobEvent::Transitioned(transitioned.clone()),
        ))
        .await
        .expect("seed transition");
    fresh
        .append_event(&EventEnvelope::new(
            MaintenanceEventId::new(),
            s.job_id,
            2,
            now,
            ironmaint_state::JobEvent::ResumeRecorded(record),
        ))
        .await
        .expect("seed record");

    let rebuilt: JobProjection = fresh.rebuild_projection(s.job_id).await.expect("rebuild");
    assert_eq!(
        rebuilt.state, transitioned.projection_after.state,
        "the record must not move the job; only the transition does"
    );
    assert_eq!(
        rebuilt.state,
        JobState::HumanReviewRequired,
        "sanity: the transition we seeded is the entry itself"
    );
    assert_eq!(rebuilt.version, transitioned.projection_after.version);
}

/// `next_actions` advertises `resume_job` only when the job can
/// actually be resumed.
///
/// The `allowed` field documents itself as what the caller "can take
/// *right now*". A job in an exceptional state with no record
/// cannot be resumed, so advertising the action there would point
/// an agent at a call guaranteed to fail.
#[tokio::test]
async fn next_actions_offers_resume_only_when_a_record_exists() {
    let s = scenario().await;

    let before = s
        .svc
        .handle_query(ironmaint_runtime::RuntimeQuery::ListNextActions { job_id: s.job_id })
        .await
        .expect("next actions");
    let ironmaint_runtime::QueryResult::NextActions(before) = before else {
        panic!("wrong variant");
    };
    assert!(
        !before
            .allowed
            .contains(&ironmaint_runtime::AllowedAction::ResumeJob),
        "a healthy job must not advertise resume"
    );

    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "needs a human".to_string(),
        })
        .await
        .expect("enter");

    let after = s
        .svc
        .handle_query(ironmaint_runtime::RuntimeQuery::ListNextActions { job_id: s.job_id })
        .await
        .expect("next actions");
    let ironmaint_runtime::QueryResult::NextActions(after) = after else {
        panic!("wrong variant");
    };
    assert!(
        after
            .allowed
            .contains(&ironmaint_runtime::AllowedAction::ResumeJob),
        "an exceptional job with a record must advertise resume, got {:?}",
        after.allowed
    );
}

/// Capturing a patched candidate while the job awaits review says
/// so, rather than reporting an inert capture.
#[tokio::test]
async fn capture_while_awaiting_review_explains_why_it_did_not_activate() {
    let s = scenario().await;
    s.svc
        .handle_command(RuntimeCommand::EnterHumanReview {
            job_id: s.job_id,
            reason: "needs a human".to_string(),
        })
        .await
        .expect("enter");

    // A second, genuinely different candidate — the agent's patch.
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "3".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "4".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let revision = PackageRevision::new(package(), PackageVersion::new("1.0.1").unwrap());
    let candidate = SourceCandidate::new(s.job_id, revision, repository, commit, tree, now);

    let result = s
        .svc
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id: s.job_id,
            candidate,
        })
        .await
        .expect("capture should still persist the candidate");

    let joined = result.side_effects.join(" | ");
    assert!(
        joined.contains("did not activate it"),
        "capture must report that activation was refused: {joined}"
    );
    assert!(
        joined.contains("HumanReviewRequired") && joined.contains("resume"),
        "the note must name the state and the remedy: {joined}"
    );
}
