//! End-to-end tests for the three `RuntimeService::handle_command`
//! handlers wired in 0B.18: `RecordCheckEvidence`,
//! `RecordObligationOutcome`, and `RunCheck`.

// `panic` is allowed here for the same reason it is in the MCP
// dispatcher tests: a `let … else` on a `QueryResult` variant match
// has no other way to fail loudly, and a silently-empty `actions`
// would make the `next_actions` assertions below pass for the wrong
// reason.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::CheckId;
use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{EvidenceKind, EvidenceProducer, EvidenceStatus};
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRecord, ExecutionRequest, Executor, ExecutorError,
    NullExecutor, RetryClass, ToolDefinitionRecord, ToolRegistry,
};
use ironmaint_policy::{
    Applicability, Obligation, ObligationOutcome, ObligationStatus, ObligationStrength,
    PolicyReference,
};
use ironmaint_runtime::{
    ActionBlocker, Clock, FixedClock, HumanAction, OrchestratorRef, QueryResult, RuntimeCommand,
    RuntimeErrorKind, RuntimeQuery, RuntimeService,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{
    CandidateStore, CheckStore, EvidenceStore, ObligationStore, ProjectionStore,
};
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

/// Test executor that returns a fixed `ExecutionRecord`.
struct StaticExecutor {
    record: ExecutionRecord,
}

impl Executor for StaticExecutor {
    async fn execute(&self, _request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        Ok(self.record.clone())
    }
}

/// Tool registration that succeeds (exit_code 0) for "synthetic.test.pass"
/// and fails (exit_code 1) for "synthetic.test.fail".
fn static_tool(key_str: &str) -> ToolDefinitionRecord {
    ToolDefinitionRecord::new(
        ToolCapabilityKey::new(key_str).unwrap(),
        PathBuf::from("/usr/bin/ironmaint-fixture"),
        vec![OsString::from("--validate")],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    )
}

/// Build a service backed by `NullExecutor` + an empty registry.
fn null_service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

/// Build a service backed by `StaticExecutor` + a registry
/// containing `synthetic.test.pass` (exit_code 0).
fn static_service(
    store: Arc<MockStore>,
    record: ExecutionRecord,
) -> (RuntimeService<MockStore, StaticExecutor>, Arc<ToolRegistry>) {
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(static_tool("synthetic.test.pass")))
        .expect("register");
    let executor = StaticExecutor { record };
    let registry_arc = Arc::new(registry);
    let svc = RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(executor),
        registry_arc.clone(),
    );
    (svc, registry_arc)
}

async fn make_job<E: Executor + ?Sized>(
    store: &MockStore,
    svc: &RuntimeService<MockStore, E>,
) -> JobId {
    let r = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    parse_job_id(&r.side_effects[0], store).await
}

/// Parse a JobId out of a `job:<uuid> created by ...` side-effect
/// string. The MockStore assigns JobIds but the runtime constructs
/// them internally — so we read the projection back to discover
/// the actual id.
async fn parse_job_id(side_effect: &str, _store: &MockStore) -> JobId {
    let _ = side_effect;
    // The runtime mints a fresh JobId internally and persists the
    // projection. We can't list projections, but the side-effect
    // string carries the id we need to parse.
    let rest = side_effect
        .strip_prefix("job:")
        .expect("side-effect prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("valid uuid"))
}

async fn seed_candidate(
    store: &MockStore,
    job_id: JobId,
) -> (ironmaint_core::CandidateId, CandidateFingerprint) {
    seed_candidate_at_commit(store, job_id, "1").await
}

/// As `seed_candidate`, but the commit OID is derived from `digit`,
/// so two calls produce two *distinct* candidates. The fingerprint
/// covers the commit, so a fixed OID would make "a superseded
/// candidate" unrepresentable — the row would be a second name for
/// the first candidate, not a different one.
async fn seed_candidate_at_commit(
    store: &MockStore,
    job_id: JobId,
    digit: &str,
) -> (ironmaint_core::CandidateId, CandidateFingerprint) {
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, digit.repeat(40)).expect("40 hex digits");
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(package(), PackageVersion::new("1.0.0").unwrap());
    let candidate = SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now);
    let fingerprint = candidate.fingerprint().clone();
    let id = store
        .put_source_candidate(&candidate)
        .await
        .expect("put_source_candidate");
    (id, fingerprint)
}

/// Seed the candidate and advance the projection to a
/// post-capture state (`SourceRevision`) so `RunCheck`'s
/// pre-condition lets the call through. Used by tests that
/// drive `RunCheck { check_id }` directly without the engine
/// wired in.
///
/// `SetActiveCandidate` enforces its own pre-condition (must
/// be in `EventDetected`..`CandidateAssembly`), so the
/// state-bump to `SourceRevision` happens *after* the active
/// candidate is attached.
async fn seed_candidate_post_capture(
    store: &MockStore,
    job_id: JobId,
) -> (ironmaint_core::CandidateId, CandidateFingerprint) {
    let (id, fingerprint) = seed_candidate(store, job_id).await;
    store
        .set_active_source_candidate(job_id, id)
        .await
        .expect("set active");
    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.active_candidate = Some(id);
    projection.state = JobState::SourceRevision;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("seed post-capture");
    (id, fingerprint)
}

#[tokio::test]
async fn record_check_evidence_persists_evidence_record() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;

    // Activate the candidate so the projection has a fingerprint.
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let result = svc
        .handle_command(RuntimeCommand::RecordCheckEvidence {
            job_id,
            tool_key: "synthetic.test.pass".to_string(),
            evidence_kind: EvidenceKind::Build,
            status: EvidenceStatus::Pass,
            producer: "test-runner".to_string(),
        })
        .await
        .expect("record evidence");

    // create=1,2 (JobCreated + Domain), set_active=3, evidence=4
    assert_eq!(result.new_sequence, 4);
    assert!(result.side_effects.is_empty());

    // Evidence list now contains the new evidence, bound to our
    // candidate's fingerprint.
    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1);
    let ev = &evidences[0];
    assert_eq!(ev.kind, EvidenceKind::Build);
    assert_eq!(ev.status, EvidenceStatus::Pass);
    assert_eq!(ev.candidate, fingerprint);
    assert_eq!(ev.producer, EvidenceProducer::new("test-runner"));
}

#[tokio::test]
async fn record_check_evidence_rejects_without_active_candidate() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;

    let err = svc
        .handle_command(RuntimeCommand::RecordCheckEvidence {
            job_id,
            tool_key: "synthetic.test.pass".to_string(),
            evidence_kind: EvidenceKind::Build,
            status: EvidenceStatus::Pass,
            producer: "test-runner".to_string(),
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn recording_an_outcome_updates_obligation_status() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    // Seed an obligation directly into the store.
    let mut obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ironmaint_policy::ObligationStrength::Mandatory,
        ironmaint_policy::Applicability::Applicable,
        "lintian-clean",
    )
    .unwrap();
    obligation = obligation.with_status(ObligationStatus::NotEvaluated);
    let obligation_id = store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let result = svc
        .handle_command(RuntimeCommand::RecordObligationOutcome {
            job_id,
            obligation_ref: "lintian-clean".to_string(),
            outcome: ObligationOutcome::Pass,
        })
        .await
        .expect("record obligation outcome");
    assert!(!result.side_effects.is_empty());

    let updated = store.get_obligation(obligation_id).await.expect("get");
    assert_eq!(updated.status, ObligationStatus::Pass);
}

#[tokio::test]
async fn recording_an_outcome_rejects_unknown_ref() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint,
    })
    .await
    .expect("set active");

    let err = svc
        .handle_command(RuntimeCommand::RecordObligationOutcome {
            job_id,
            obligation_ref: "no-such-obligation".to_string(),
            outcome: ObligationOutcome::Pass,
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn run_check_invokes_executor_and_records_evidence() {
    let store = Arc::new(MockStore::new());
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let record = ExecutionRecord {
        tool_key: ToolCapabilityKey::new("synthetic.test.pass").unwrap(),
        retry_class: RetryClass::Safe,
        started_at: now,
        finished_at: now + time::Duration::milliseconds(10),
        exit_code: 0,
        stdout: "OK".to_string(),
        stderr: String::new(),
        retries_exhausted: false,
        truncated: false,

        // Constructed rather than executed: nothing spilled.
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    };
    let (svc, _registry) = static_service(store.clone(), record);

    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate_post_capture(&store, job_id).await;

    // Materialize a check for the synthetic tool so RunCheck has
    // a valid contract to resolve.
    let key = ToolCapabilityKey::new("synthetic.test.pass").unwrap();
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: fingerprint,
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let check_id = ids[0];

    let result = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id,
            retry_class: RetryClass::Safe,
        })
        .await
        .expect("run check");

    // create=1,2 (JobCreated + Domain), run_check=3
    assert_eq!(result.new_sequence, 3);
    assert!(!result.side_effects.is_empty());

    // Evidence was recorded.
    let evidences = store.list_evidence_for_job(job_id).await.expect("list");
    assert_eq!(evidences.len(), 1);
}

#[tokio::test]
async fn run_check_rejects_unknown_tool() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());

    let job_id = make_job(&store, &svc).await;
    let (_cid, _fingerprint) = seed_candidate_post_capture(&store, job_id).await;

    // No MaterializeChecks call: any freshly-minted CheckId is
    // unknown to the store, so RunCheck must reject with
    // InvalidInput.
    let err = svc
        .handle_command(RuntimeCommand::RunCheck {
            job_id,
            check_id: CheckId::new(),
            retry_class: RetryClass::Safe,
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

/// 0A §35: an approved exception is backed by an explicit approval
/// record, and a policy evaluation is not authority to withdraw
/// authority. Recording a fresh verdict over one would silently revoke
/// a decision nobody made.
#[tokio::test]
async fn recording_an_outcome_will_not_overwrite_an_approved_exception() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "exempt-by-policy",
    )
    .unwrap()
    .with_status(ObligationStatus::ExceptionApproved);
    let obligation_id = store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let err = svc
        .handle_command(RuntimeCommand::RecordObligationOutcome {
            job_id,
            obligation_ref: "exempt-by-policy".to_string(),
            outcome: ObligationOutcome::Fail,
        })
        .await
        .expect_err("an approved exception must not be overwritten");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
    assert!(
        format!("{err}").contains("exception"),
        "the error must name the reason, got: {err}"
    );

    let after = store.get_obligation(obligation_id).await.expect("get");
    assert_eq!(
        after.status,
        ObligationStatus::ExceptionApproved,
        "the refusal must leave the obligation untouched"
    );
}

/// Recording a verdict is a fact about the obligation, not a
/// transition. The job parks exactly where it was; the engine decides
/// whether it may move on the next attempt.
#[tokio::test]
async fn recording_a_failing_outcome_does_not_advance_the_job() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let before = store.get_projection(job_id).await.expect("projection");
    let obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "changelog-present",
    )
    .unwrap();
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "changelog-present".to_string(),
        outcome: ObligationOutcome::Fail,
    })
    .await
    .expect("record");

    let after = store.get_projection(job_id).await.expect("projection");
    assert_eq!(
        before.state, after.state,
        "an obligation verdict must not move the job"
    );
    assert_eq!(
        before.version, after.version,
        "an obligation verdict must not bump the projection version"
    );
}

/// The reporting half. `project_next_actions` is pure, so at
/// `ReleaseReview` it can only say "policy completion is a person's
/// judgement" without reading whether anything is outstanding. With a
/// writable `Fail`, an agent told only that would not know which
/// assertion failed — so `attach_obligation_state` names it.
#[tokio::test]
async fn next_actions_names_the_outstanding_obligation() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "copyright-file-present",
    )
    .unwrap();
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");
    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "copyright-file-present".to_string(),
        outcome: ObligationOutcome::Fail,
    })
    .await
    .expect("record");

    // Park the job where policy gates the next move.
    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.state = JobState::ReleaseReview;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("park at ReleaseReview");

    let QueryResult::NextActions(actions) = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions")
    else {
        panic!("wrong QueryResult variant");
    };

    assert_eq!(
        actions.requires_human,
        Some(HumanAction::SatisfyObligation {
            reference: Some("copyright-file-present".to_string()),
        }),
        "the outstanding obligation must be named, not reported as a bare 'satisfy something'"
    );
    assert!(
        actions.blockers.iter().any(
            |b| matches!(b, ActionBlocker::ObligationPending { reference }
                if reference == "copyright-file-present")
        ),
        "the failing obligation must also appear as a blocker: {:?}",
        actions.blockers
    );
}

/// The same reporting, at a state where policy does not gate the
/// next move.
///
/// `attach_obligation_state` used to return early unless the pure
/// projection had already said `SatisfyObligation`, which it only
/// does at `ReleaseReview` and `FinalValidation`. So the whole
/// obligation-reporting path was unreachable below those states, and
/// an agent at `EventDetected` whose policy evaluation had come back
/// negative was told its blockers were empty. Its only visible move
/// was to re-run the check that had just failed, which produces the
/// same evidence and the same verdict — and SKILL.md had to tell
/// agents to patch and re-capture instead, because the tool surface
/// never said so.
///
/// What changed is the *blocker*, not `requires_human`. Whether a
/// person has to act is state-dependent and the pure projection
/// decides it; whether an assertion is outstanding is not.
#[tokio::test]
async fn an_outstanding_obligation_is_reported_at_an_early_state_too() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "copyright-file-present",
    )
    .unwrap();
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");
    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "copyright-file-present".to_string(),
        outcome: ObligationOutcome::Fail,
    })
    .await
    .expect("record");

    // A fresh job: `EventDetected`, well below `ReleaseReview`.
    let QueryResult::NextActions(actions) = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions")
    else {
        panic!("wrong QueryResult variant");
    };

    assert_eq!(
        store
            .get_projection(job_id)
            .await
            .expect("projection")
            .state,
        JobState::EventDetected,
        "this test is only worth anything before policy gates the move"
    );
    assert!(
        actions.blockers.iter().any(
            |b| matches!(b, ActionBlocker::ObligationPending { reference }
                if reference == "copyright-file-present")
        ),
        "the outstanding assertion is a fact at every state, not only near \
         release: {:?}",
        actions.blockers
    );
    assert_eq!(
        actions.requires_human, None,
        "but the fix is source, and source is the agent's work — reporting it \
         as a human action would tell an agent to stop on a job §83 says it \
         should drive"
    );
}

/// `NotEvaluated` is an unasked question, not an outstanding
/// assertion.
///
/// Every obligation a capture derives starts here, so reporting
/// `NotEvaluated` would attach an `ObligationPending` blocker to
/// every job from its first capture and tell the agent to go satisfy
/// a verdict that does not exist. The question is already asked by
/// the pending `RunCheck`.
#[tokio::test]
async fn an_unevaluated_obligation_is_not_a_blocker() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    store
        .put_obligation(
            &Obligation::new(
                fingerprint.clone(),
                PolicyReference::new(ironmaint_core::AuthorityId::new()),
                ObligationStrength::Mandatory,
                Applicability::Applicable,
                "copyright-file-present",
            )
            .unwrap(),
            job_id,
        )
        .await
        .expect("put obligation");

    let QueryResult::NextActions(actions) = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions")
    else {
        panic!("wrong QueryResult variant");
    };

    assert!(
        !actions
            .blockers
            .iter()
            .any(|b| matches!(b, ActionBlocker::ObligationPending { .. })),
        "an obligation nobody has evaluated is not something to clear: {:?}",
        actions.blockers
    );
}

/// And a job with no active candidate at all must not fail.
///
/// `next_actions` on a freshly created job is the first call an agent
/// makes. Once the obligation scan stopped being gated on the
/// projection's state it also stopped being gated on a candidate
/// existing, and "no active candidate" is the normal state of a
/// brand-new job — reported as a `no_active_candidate` blocker by the
/// pure projection, not as an error.
#[tokio::test]
async fn next_actions_on_a_job_with_no_candidate_is_not_an_error() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;

    let QueryResult::NextActions(actions) = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect(
            "a job with no candidate has a next-actions list; it is an empty \
                 one",
        )
    else {
        panic!("wrong QueryResult variant");
    };
    assert_eq!(
        store
            .get_projection(job_id)
            .await
            .expect("projection")
            .state,
        JobState::EventDetected,
        "a job that has not captured anything yet is at EventDetected, and the \
         only thing `next_actions` must say about it is that a capture is the \
         next move"
    );
    assert!(
        actions
            .allowed
            .iter()
            .any(|a| matches!(a, ironmaint_runtime::AllowedAction::CaptureCandidate)),
        "got {:?}",
        actions.allowed
    );
}

/// And when every mandatory obligation passes, the report must not
/// claim there is something outstanding. The unconditional
/// `SatisfyObligation { reference: None }` the pure projection emits
/// is only honest as a fallback when the store knows of nothing.
#[tokio::test]
async fn next_actions_reports_no_obligation_pending_once_satisfied() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;
    let (_cid, fingerprint) = seed_candidate(&store, job_id).await;
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let obligation = Obligation::new(
        fingerprint.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "copyright-file-present",
    )
    .unwrap();
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");
    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: "copyright-file-present".to_string(),
        outcome: ObligationOutcome::Pass,
    })
    .await
    .expect("record");

    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.state = JobState::ReleaseReview;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("park at ReleaseReview");

    let QueryResult::NextActions(actions) = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions")
    else {
        panic!("wrong QueryResult variant");
    };

    assert!(
        !actions
            .blockers
            .iter()
            .any(|b| matches!(b, ActionBlocker::ObligationPending { .. })),
        "a satisfied obligation is not a blocker: {:?}",
        actions.blockers
    );
    assert_eq!(
        actions.requires_human,
        Some(HumanAction::SatisfyObligation { reference: None }),
        "with nothing specific outstanding the reference stays unknown"
    );
}

/// An obligation belonging to a superseded candidate is not the
/// active policy surface. Reporting it — or letting a verdict be
/// written against it — would attach the previous candidate's policy
/// to the current one, which is the exact cross-candidate leak the
/// immutable-candidate rule exists to prevent.
#[tokio::test]
async fn an_obligation_from_a_superseded_candidate_is_not_reported() {
    let store = Arc::new(MockStore::new());
    let svc = null_service(store.clone());
    let job_id = make_job(&store, &svc).await;
    let (_cid, stale_fp) = seed_candidate_at_commit(&store, job_id, "3").await;
    let (_cid2, live_fp) = seed_candidate_at_commit(&store, job_id, "4").await;
    assert_ne!(stale_fp, live_fp, "the two candidates must differ");

    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: live_fp.clone(),
    })
    .await
    .expect("set active");

    let obligation = Obligation::new(
        stale_fp.clone(),
        PolicyReference::new(ironmaint_core::AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "stale-candidate-rule",
    )
    .unwrap()
    .with_status(ObligationStatus::Fail);
    store
        .put_obligation(&obligation, job_id)
        .await
        .expect("put obligation");

    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.state = JobState::ReleaseReview;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("park at ReleaseReview");

    let QueryResult::NextActions(actions) = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions")
    else {
        panic!("wrong QueryResult variant");
    };
    assert!(
        !actions
            .blockers
            .iter()
            .any(|b| matches!(b, ActionBlocker::ObligationPending { .. })),
        "a superseded candidate's obligation must not block: {:?}",
        actions.blockers
    );

    // And a verdict cannot be written against it either.
    let err = svc
        .handle_command(RuntimeCommand::RecordObligationOutcome {
            job_id,
            obligation_ref: "stale-candidate-rule".to_string(),
            outcome: ObligationOutcome::Pass,
        })
        .await
        .expect_err("must not match an obligation bound to another candidate");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}
