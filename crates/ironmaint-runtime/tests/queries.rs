//! `RuntimeQuery` coverage for the two query variants added in
//! 0B.9 — `GetCheckOutcome` and `GetOperation` — plus the
//! `ListNextActions` enrichment that makes `check.run` reachable.
//!
//! Both variants exist so the MCP layer can read durable state
//! *through the runtime* (PHASE-0B.md §98.6) rather than around it.
//! Before them, `check.run` and `operation.get` had no way to
//! report a real result: one returned a fixed all-zero record, the
//! other a sentinel `proposed` placeholder for every id.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, CheckId, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{EvidenceKind, EvidenceStatus, GateStatus};
use ironmaint_executor::{
    ExecutionClass, ExecutionLimits, ExecutionRecord, ExecutionRequest, Executor, ExecutorError,
    RetryClass, ToolRegistry,
};
use ironmaint_policy::{AuthorizationState, PrivilegedOperation, PrivilegedOperationKind};
use ironmaint_runtime::{
    Clock, FixedClock, QueryResult, RuntimeCommand, RuntimeErrorKind, RuntimeQuery, RuntimeService,
    orchestrator::OrchestratorRef,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, CheckStore, OperationStore, ProjectionStore};
use time::OffsetDateTime;
use url::Url;

const TOOL: &str = "synthetic.test.validate";

// ---------------------------------------------------------------------------
// A minimal executor. The tests care about what the runtime does with
// a verdict, not about the verdict itself, so one recording shape and
// one failing shape is enough.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Behaviour {
    Pass,
    Fail,
}

#[derive(Clone)]
struct ScriptedExecutor {
    behaviour: Behaviour,
}

impl Executor for ScriptedExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        Ok(ExecutionRecord {
            tool_key: request.tool_key.clone(),
            retry_class: RetryClass::Safe,
            started_at: now,
            finished_at: now,
            exit_code: match self.behaviour {
                Behaviour::Pass => 0,
                Behaviour::Fail => 1,
            },
            stdout: String::new(),
            stderr: String::new(),
            retries_exhausted: false,
            truncated: false,
        })
    }
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").expect("family"),
            DistributionRelease::new("unstable").expect("release"),
        ),
        PackageName::new("fixture-pkg").expect("name"),
    )
}

fn build_service(
    store: &Arc<MockStore>,
    behaviour: Behaviour,
) -> (
    RuntimeService<MockStore, ScriptedExecutor>,
    Arc<ToolRegistry>,
) {
    let key = ToolCapabilityKey::new(TOOL).expect("valid capability");
    let mut registry = ToolRegistry::new();
    registry
        .register(Box::new(ironmaint_executor::ToolDefinitionRecord::new(
            key,
            std::path::Path::new("/nonexistent/ironmaint-fixture"),
            vec![std::ffi::OsString::from(TOOL)],
            ExecutionClass::Check,
            ExecutionLimits::default(),
        )))
        .expect("register");
    let registry = Arc::new(registry);
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    ));
    let service = RuntimeService::new(
        Arc::clone(store),
        clock,
        Arc::new(ScriptedExecutor { behaviour }),
        Arc::clone(&registry),
    );
    (service, registry)
}

fn parse_job_id(side_effect: &str) -> JobId {
    let rest = side_effect
        .strip_prefix("job:")
        .expect("prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("job uuid"))
}

/// Create a job, attach a real candidate, make it active, and move
/// the projection into a post-capture state so `RunCheck` is legal.
async fn seed_job_with_candidate(
    store: &MockStore,
    svc: &RuntimeService<MockStore, ScriptedExecutor>,
) -> (JobId, CandidateFingerprint) {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let job_id = parse_job_id(&result.side_effects[0]);

    let candidate = SourceCandidate::new(
        job_id,
        PackageRevision::new(package(), PackageVersion::new("1.0.0").expect("version")),
        RepositoryRef::new(
            VcsKind::Git,
            Url::parse("https://example.invalid/foo.git").unwrap(),
        )
        .expect("repository"),
        GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).expect("commit"),
        GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).expect("tree"),
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    let fingerprint = candidate.fingerprint().clone();
    store
        .put_source_candidate(&candidate)
        .await
        .expect("put candidate");

    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fingerprint.clone(),
    })
    .await
    .expect("set active");

    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.state = JobState::SourceRevision;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("seed state");
    (job_id, fingerprint)
}

async fn materialize_check(
    svc: &RuntimeService<MockStore, ScriptedExecutor>,
    store: &Arc<MockStore>,
    job_id: JobId,
    fingerprint: CandidateFingerprint,
) -> CheckId {
    let key = ToolCapabilityKey::new(TOOL).expect("capability");
    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate: fingerprint,
        planned: vec![(key, EvidenceKind::Build, true)],
    })
    .await
    .expect("materialize");
    store
        .list_checks_for_job(job_id)
        .await
        .expect("list checks")
        .into_iter()
        .next()
        .expect("at least one check")
}

async fn run_check(
    svc: &RuntimeService<MockStore, ScriptedExecutor>,
    job_id: JobId,
    check_id: CheckId,
) {
    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id,
        retry_class: RetryClass::Safe,
    })
    .await
    .expect("run check");
}

async fn outcome_of(
    svc: &RuntimeService<MockStore, ScriptedExecutor>,
    check_id: CheckId,
) -> ironmaint_runtime::outcome::CheckOutcome {
    match svc
        .handle_query(RuntimeQuery::GetCheckOutcome { check_id })
        .await
        .expect("get outcome")
    {
        QueryResult::CheckOutcome(o) => o,
        other => panic!("wrong QueryResult variant: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// GetCheckOutcome
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_outcome_reports_pass_after_a_passing_check() {
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check(&svc, &store, job_id, fingerprint.clone()).await;

    run_check(&svc, job_id, check_id).await;
    let outcome = outcome_of(&svc, check_id).await;

    assert_eq!(outcome.job_id, job_id);
    assert_eq!(outcome.check_id, check_id);
    assert_eq!(outcome.candidate, fingerprint);
    assert_eq!(outcome.tool_key, TOOL);
    assert_eq!(outcome.evidence_status, Some(EvidenceStatus::Pass));
    assert_eq!(outcome.gate_status, GateStatus::Pass);
    assert!(outcome.evidence_id.is_some(), "evidence must be recorded");
}

#[tokio::test]
async fn check_outcome_reports_fail_after_a_failing_check() {
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Fail);
    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check(&svc, &store, job_id, fingerprint).await;

    run_check(&svc, job_id, check_id).await;
    let outcome = outcome_of(&svc, check_id).await;

    assert_eq!(outcome.evidence_status, Some(EvidenceStatus::Fail));
    assert_eq!(outcome.gate_status, GateStatus::Fail);
}

#[tokio::test]
async fn check_outcome_for_an_unrun_check_is_not_evaluated_not_failed() {
    // "Not run yet" and "ran and failed" must be distinguishable, or
    // an orchestrator reading a pending gate concludes the package
    // is broken when nothing has executed. `EvidenceStatus` has no
    // "not evaluated" member — evidence is always the result of
    // something that ran — so the absence is carried by
    // `evidence_id`/`evidence_status` being `None`.
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check(&svc, &store, job_id, fingerprint).await;

    let outcome = outcome_of(&svc, check_id).await;
    assert!(outcome.evidence_id.is_none(), "no evidence expected");
    assert!(outcome.evidence_status.is_none());
    assert_eq!(outcome.gate_status, GateStatus::NotEvaluated);
    assert_eq!(outcome.tool_key, TOOL, "the tool is still named");
}

#[tokio::test]
async fn check_outcome_rejects_an_unknown_check_id() {
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let err = svc
        .handle_query(RuntimeQuery::GetCheckOutcome {
            check_id: CheckId::new(),
        })
        .await
        .expect_err("unknown check must error");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

// ---------------------------------------------------------------------------
// GetOperation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_operation_returns_the_stored_record() {
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;

    let operation = PrivilegedOperation::proposed(
        PrivilegedOperationKind::CanonicalRepositoryPush,
        fingerprint,
    );
    let operation_id = store
        .put_operation(&operation, job_id)
        .await
        .expect("put operation");

    let got = svc
        .handle_query(RuntimeQuery::GetOperation { operation_id })
        .await
        .expect("get operation");
    let QueryResult::Operation(op) = got else {
        panic!("wrong QueryResult variant");
    };
    assert_eq!(op.id, operation_id);
    assert_eq!(op.kind, PrivilegedOperationKind::CanonicalRepositoryPush);
    assert_eq!(op.authorization, AuthorizationState::Proposed);
}

#[tokio::test]
async fn get_operation_rejects_an_unknown_id() {
    // The 0B.6 MCP stub returned a sentinel `proposed` record for
    // *every* id, so this could not fail and a caller could not tell
    // a real operation from the placeholder.
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let err = svc
        .handle_query(RuntimeQuery::GetOperation {
            operation_id: ironmaint_core::OperationId::new(),
        })
        .await
        .expect_err("unknown operation must error");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
    assert!(err.message.contains("unknown operation"), "{}", err.message);
}

// ---------------------------------------------------------------------------
// ListNextActions enrichment
// ---------------------------------------------------------------------------

#[tokio::test]
async fn next_actions_surfaces_the_check_id_for_a_pending_gate() {
    // Without this, `next_actions` could only say "a gate is
    // pending" and no MCP tool hands an agent a `check_id`, so
    // `check.run` was unreachable from the tool surface.
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    let check_id = materialize_check(&svc, &store, job_id, fingerprint).await;

    let got = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions");
    let QueryResult::NextActions(actions) = got else {
        panic!("wrong QueryResult variant");
    };

    assert!(
        actions
            .allowed
            .contains(&ironmaint_runtime::next_actions::AllowedAction::RunCheck { check_id }),
        "the pending check must be runnable: {:?}",
        actions.allowed
    );
    // And the blocker must name the real tool rather than a
    // hardcoded fixture string.
    let named = actions.blockers.iter().any(|b| {
        matches!(
            b,
            ironmaint_runtime::next_actions::ActionBlocker::GatePending { tool_key: Some(k) }
                if k == TOOL
        )
    });
    assert!(
        named,
        "pending gate must name the real tool: {:?}",
        actions.blockers
    );
}

#[tokio::test]
async fn next_actions_does_not_duplicate_the_run_check_action() {
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let (job_id, fingerprint) = seed_job_with_candidate(&store, &svc).await;
    materialize_check(&svc, &store, job_id, fingerprint).await;

    let got = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions");
    let QueryResult::NextActions(actions) = got else {
        panic!("wrong QueryResult variant");
    };
    let run_checks = actions
        .allowed
        .iter()
        .filter(|a| {
            matches!(
                a,
                ironmaint_runtime::next_actions::AllowedAction::RunCheck { .. }
            )
        })
        .count();
    assert_eq!(run_checks, 1, "RunCheck must appear exactly once");
}

#[tokio::test]
async fn next_actions_leaves_the_tool_unnamed_when_nothing_is_materialised() {
    // No adapter has produced checks for this job, so there is no
    // tool to name. Inventing one — as the pure projection used to,
    // with a hardcoded `synthetic.build.validate` — would tell the
    // agent to run something that does not exist.
    let store = Arc::new(MockStore::new());
    let (svc, _registry) = build_service(&store, Behaviour::Pass);
    let (job_id, _fingerprint) = seed_job_with_candidate(&store, &svc).await;

    let got = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("next actions");
    let QueryResult::NextActions(actions) = got else {
        panic!("wrong QueryResult variant");
    };
    assert!(
        actions.allowed.is_empty(),
        "nothing to run: {:?}",
        actions.allowed
    );
    assert!(
        actions.blockers.iter().any(|b| matches!(
            b,
            ironmaint_runtime::next_actions::ActionBlocker::GatePending { tool_key: None }
        )),
        "gate must still be reported as pending, unnamed: {:?}",
        actions.blockers
    );
}
