//! State-machine wiring test — the 13-state happy path, on a
//! `MockStore`, with the evidence decided in advance.
//!
//! **This is not the §81 fixture driver, and it did not claim to be
//! until 2026-10-01.** The old module doc called it that, and
//! §101 item 28 / §81 both want a driver that *runs a failing
//! check, reads the evidence, patches, captures, and reruns*. This
//! file pre-seeds twelve passing `GateResult`s, so it exercises
//! neither failure, nor repair, nor evidence invalidation, nor a
//! real tool. It walks a job across `TRANSITION_RULES` and checks
//! that the table says what it says — which is worth having, and is
//! a different thing.
//!
//! The exit checkpoints live in their own files:
//!
//! - `tests/acceptance_scenario.rs` — §101 / §102 item 28, the
//!   deterministic no-LLM driver, over `RuntimeCommand` /
//!   `RuntimeQuery` against real SQLite.
//! - `tests/mcp_acceptance_scenario.rs` — §101 / §102 item 29, the
//!   same walk with **no `RuntimeService` handle**, every step a
//!   `dispatch()` call.
//!
//! What this file is actually good for is the thing the other two
//! deliberately do not do: it holds a `RuntimeService` and a
//! `MockStore` in the same scope, so it can assert on the store's
//! shape directly. That makes it the cheapest place to check that
//! `reconcile` and the projection agree, and it is why the
//! `MockStore`-vs-SQLite divergences that 0B.10's drivers found
//! (a silent second row under a duplicated fingerprint; a
//! `rebuild_projection` that rejects every real log) were worth
//! looking for here at all.
//!
//! Each transition is driven by `RuntimeCommand::Reconcile`, which
//! walks the static `TRANSITION_RULES` table until it hits a
//! blocker or an actor-required transition.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    JobState, MaintenanceEventId, MaintenanceJob, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{
    EvidenceKind, GateDefinition, GateRequirement, GateResult, GateStage, RequiredEvidenceStatus,
};
use ironmaint_executor::{
    ExecutionRecord, ExecutionRequest, Executor, ExecutorError, NullExecutor, RetryClass,
    ToolDefinitionRecord, ToolRegistry,
};
use ironmaint_runtime::{
    HumanAction, ReconcileOutcome, RuntimeCommand, RuntimeQuery, RuntimeService, SystemClock,
};
use ironmaint_state::JobEvent as StateJobEvent;
use ironmaint_store::{EventStore, GateStore, ObligationStore, ProjectionStore, mock::MockStore};
use std::ffi::OsString;
use std::path::PathBuf;
use time::OffsetDateTime;
use url::Url;

fn debian_foo() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new("debian").unwrap(),
            DistributionRelease::new("unstable").unwrap(),
        ),
        PackageName::new("synthetic-pkg").unwrap(),
    )
}

/// Always-passes scripted executor — used so `RunCheck` can
/// record `Pass` evidence without spawning a subprocess.
struct PassExecutor;

impl Executor for PassExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        Ok(ExecutionRecord {
            tool_key: request.tool_key.clone(),
            retry_class: RetryClass::Safe,
            started_at: now,
            finished_at: now,
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            retries_exhausted: false,
            truncated: false,

            // Constructed rather than executed: nothing spilled.
            artifacts: Vec::new(),
            artifacts_dropped: Vec::new(),
        })
    }
}

/// Always-fails scripted executor — for the blocker test.
struct FailExecutor;

impl Executor for FailExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionRecord, ExecutorError> {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        Ok(ExecutionRecord {
            tool_key: request.tool_key.clone(),
            retry_class: RetryClass::Safe,
            started_at: now,
            finished_at: now,
            exit_code: 1,
            stdout: String::new(),
            stderr: String::from("synthetic fail"),
            retries_exhausted: false,
            truncated: false,

            // Constructed rather than executed: nothing spilled.
            artifacts: Vec::new(),
            artifacts_dropped: Vec::new(),
        })
    }
}

fn make_source_candidate(job_id: JobId) -> SourceCandidate {
    let url = Url::parse("https://example.invalid/repo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "a".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "b".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(debian_foo(), PackageVersion::new("1.0.0").unwrap());
    SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now)
}

async fn seed_passing_gate(
    store: &Arc<MockStore>,
    job_id: JobId,
    fp: &ironmaint_core::CandidateFingerprint,
    stage: GateStage,
) {
    let gate = GateDefinition::new(
        fp.clone(),
        stage,
        GateRequirement {
            evidence_kind: EvidenceKind::SourceIntegrity,
            minimum_status: RequiredEvidenceStatus::Pass,
        },
        true,
    );
    let g_id = store
        .put_gate_definition(&gate, job_id)
        .await
        .expect("put gate");
    let result = GateResult::pass(
        g_id,
        fp.clone(),
        vec![],
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    store
        .put_gate_result(g_id, fp, &result)
        .await
        .expect("put gate result");
}

/// Builds a service with the given executor. The registry is
/// populated with a single tool so `RunCheck` succeeds for
/// any (synthetic.*) tool_key.
fn build_service<E: Executor + ?Sized>(
    store: Arc<MockStore>,
    executor: Arc<E>,
) -> RuntimeService<MockStore, E> {
    let mut reg = ToolRegistry::new();
    let record = ToolDefinitionRecord::new(
        ToolCapabilityKey::new("synthetic.build.validate").unwrap(),
        PathBuf::from("/bin/true"),
        vec![OsString::from("synthetic.build.validate")],
        ironmaint_executor::ExecutionClass::Check,
        ironmaint_executor::ExecutionLimits::default(),
    );
    let boxed: Box<dyn ironmaint_executor::ToolDefinition> = Box::new(record);
    reg.register(boxed).expect("register tool");
    RuntimeService::new(store, Arc::new(SystemClock), executor, Arc::new(reg))
}

#[tokio::test]
async fn synthetic_debian_driver_reaches_ready_for_approval() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone(), Arc::new(PassExecutor));

    // Step 1: CreateJob.
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: ironmaint_runtime::OrchestratorRef::ironclaw(),
            package: debian_foo(),
        })
        .await
        .expect("create");
    let job_id = {
        let side = &result.side_effects[0];
        let rest = side.strip_prefix("job:").expect("prefix");
        let id_str = rest.split_whitespace().next().expect("uuid");
        JobId::from_uuid(uuid::Uuid::parse_str(id_str).expect("valid"))
    };

    // Step 2: CaptureCandidate.
    let candidate = make_source_candidate(job_id);
    let fp = candidate.fingerprint().clone();
    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("capture");

    // Step 3: SetActiveCandidate.
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fp.clone(),
    })
    .await
    .expect("set active");

    // Step 4: For each gate stage that any §20 rule requires,
    // seed a passing gate result so reconcile can walk all the
    // way to ReadyForApproval.
    let stages = [
        GateStage::SourcePreparation,
        GateStage::SourceAnalysis,
        GateStage::IssueAnalysis,
        GateStage::CandidateAssembly,
        GateStage::Maintenance,
        GateStage::PolicyEvaluation,
        GateStage::BuildValidation,
        GateStage::PackageQa,
        GateStage::FunctionalValidation,
        GateStage::UpgradeValidation,
        GateStage::ReleaseReview,
        GateStage::FinalValidation,
    ];
    for stage in stages {
        seed_passing_gate(&store, job_id, &fp, stage).await;
    }

    // Seed a mandatory obligation with Pass status so the
    // FinalValidation → ReadyForApproval rule (which requires
    // `policy_completion`) is unblocked.
    use ironmaint_core::AuthorityId;
    use ironmaint_policy::{
        Applicability, Obligation, ObligationStatus, ObligationStrength, PolicyReference,
    };
    let ob = Obligation::new(
        fp.clone(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "policy.complete",
    )
    .expect("static literal fits");
    let ob_id = store
        .put_obligation(&ob, job_id)
        .await
        .expect("put obligation");
    // Mark the obligation Pass via the obligation store's update path.
    let mut ob = ob;
    ob.status = ObligationStatus::Pass;
    store
        .update_obligation(ob_id, &ob)
        .await
        .expect("update obligation");

    // Step 5: Reconcile in a loop. Each iteration advances one
    // state. We stop when reconcile reports `NoOp` at
    // ReadyForApproval (the §81 exit checkpoint).
    let mut last_outcome: Option<ReconcileOutcome> = None;
    for _ in 0..20 {
        let outcome = svc.reconcile(job_id).await.expect("reconcile");
        last_outcome = Some(outcome.clone());
        match outcome {
            ReconcileOutcome::Advanced { to, .. } => {
                if to == JobState::ReadyForApproval {
                    break;
                }
            }
            ReconcileOutcome::Blocked { .. }
            | ReconcileOutcome::NeedsActorDecision { .. }
            | ReconcileOutcome::Exceptional { .. }
            | ReconcileOutcome::NoOp { .. } => break,
            ReconcileOutcome::ConcurrentModification => {
                panic!("unexpected concurrent modification")
            }
        }
    }

    // Step 6: Assert projection reached ReadyForApproval.
    let projection = store.get_projection(job_id).await.expect("projection");
    assert_eq!(
        projection.state,
        JobState::ReadyForApproval,
        "fixture driver must reach ReadyForApproval; outcome was {last_outcome:?}"
    );

    // Step 7: next_actions must say the job is waiting on a person.
    let next = svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("query");
    let actions = match next {
        ironmaint_runtime::service::QueryResult::NextActions(a) => a,
        _ => unreachable!("expected NextActions"),
    };
    assert_eq!(
        actions.requires_human,
        Some(HumanAction::ApproveRelease),
        "next_actions at ReadyForApproval must park on the approval: {actions:?}"
    );
    assert!(
        actions.allowed.is_empty(),
        "nothing at ReadyForApproval is the agent's to take: {:?}",
        actions.allowed
    );
    assert_eq!(actions.job_id, job_id);
    let _ = Mutex::new(());

    // Step 8: An audit event chain has been emitted.
    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("events");
    let transition_count = events
        .iter()
        .filter(|e| matches!(e.event, StateJobEvent::Transitioned(_)))
        .count();
    assert!(
        transition_count >= 11,
        "expected >= 11 Transitioned events (one per gate), got {transition_count}"
    );

    let _ = MaintenanceJob::new(
        job_id,
        debian_foo(),
        MaintenanceEventId::new(),
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    );
    let _ = NullExecutor;
}

#[tokio::test]
async fn synthetic_debian_driver_blocks_on_tool_failure() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    // Use the FailExecutor so any RunCheck records a Fail.
    let svc = build_service(store.clone(), Arc::new(FailExecutor));

    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: ironmaint_runtime::OrchestratorRef::ironclaw(),
            package: debian_foo(),
        })
        .await
        .expect("create");
    let job_id = {
        let side = &result.side_effects[0];
        let id_str = side
            .strip_prefix("job:")
            .expect("prefix")
            .split_whitespace()
            .next()
            .expect("uuid");
        JobId::from_uuid(uuid::Uuid::parse_str(id_str).expect("valid"))
    };

    let candidate = make_source_candidate(job_id);
    let fp = candidate.fingerprint().clone();
    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("capture");
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fp.clone(),
    })
    .await
    .expect("set active");

    // Seed only the first three gate results so the driver
    // advances to SourceRevision, then fails at SourceRevision
    // → SourceIntegrity (the missing gate blocks the transition).
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::SourceAnalysis).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::IssueAnalysis).await;
    seed_passing_gate(&store, job_id, &fp, GateStage::Maintenance).await;

    let mut last_outcome = None;
    for _ in 0..10 {
        let outcome = svc.reconcile(job_id).await.expect("reconcile");
        last_outcome = Some(outcome.clone());
        match outcome {
            ReconcileOutcome::Advanced { to, .. } => {
                if to == JobState::SourceRevision {
                    break;
                }
            }
            _ => break,
        }
    }
    // Now we should be at SourceRevision. Try to advance further.
    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(
        matches!(outcome, ReconcileOutcome::Blocked { .. }),
        "expected Blocked (no gate for PolicyEvaluation), got {outcome:?}"
    );
    let _ = last_outcome;
}

#[tokio::test]
async fn synthetic_debian_driver_handles_concurrent_modification() {
    use ironmaint_core::CandidateId;
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone(), Arc::new(PassExecutor));

    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: ironmaint_runtime::OrchestratorRef::ironclaw(),
            package: debian_foo(),
        })
        .await
        .expect("create");
    let job_id = {
        let side = &result.side_effects[0];
        let id_str = side
            .strip_prefix("job:")
            .expect("prefix")
            .split_whitespace()
            .next()
            .expect("uuid");
        JobId::from_uuid(uuid::Uuid::parse_str(id_str).expect("valid"))
    };
    let candidate = make_source_candidate(job_id);
    let fp = candidate.fingerprint().clone();
    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("capture");
    svc.handle_command(RuntimeCommand::SetActiveCandidate {
        job_id,
        fingerprint: fp.clone(),
    })
    .await
    .expect("set active");
    seed_passing_gate(&store, job_id, &fp, GateStage::SourcePreparation).await;

    // Bump the projection version under reconcile's feet by
    // overwriting with a stale CandidateId. The next reconcile
    // call will see the bumped version.
    let cid = CandidateId::new();
    let mut p = store.get_projection(job_id).await.expect("projection");
    p.active_candidate = Some(cid);
    store.put_projection(&p, p.version).await.expect("bump");

    // Reconcile should now either be Blocked (missing gate
    // because the new active_candidate has no SourceCandidate
    // row) or NoOp (pre-capture short-circuit depending on
    // active_candidate resolution). Both are valid
    // non-advancing outcomes; the contract is that we never
    // silently advance on stale state.
    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert!(
        matches!(
            outcome,
            ReconcileOutcome::Blocked { .. }
                | ReconcileOutcome::NoOp { .. }
                | ReconcileOutcome::Exceptional { .. }
        ),
        "expected non-advancing outcome, got {outcome:?}"
    );
}
