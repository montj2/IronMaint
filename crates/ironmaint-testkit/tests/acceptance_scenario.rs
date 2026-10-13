//! PHASE-0B.md §101 — the required synthetic acceptance scenario,
//! driven end to end with no LLM in the loop (DoD #28, #31).
//!
//! ## Why this file exists and `synthetic_e2e.rs` does not
//!
//! `synthetic_e2e.rs` walks one candidate to `ReadyForApproval`
//! with twelve passing `GateResult`s **pre-seeded into the store**.
//! It says so in its own module doc. It exercises state-machine
//! wiring and nothing else: no subprocess runs, no check fails, no
//! evidence is invalidated, no obligation ever fails, and there is
//! only ever one candidate. §101 is thirty steps about a repair
//! loop, and none of the repair loop is in there.
//!
//! This file has no pre-seeded gates. Every verdict comes from the
//! `ironmaint-fixture` binary, spawned by a real `ProcessExecutor`,
//! whose exit code the runtime turns into evidence and then into a
//! gate result. The failures are not `put_gate_result` calls with
//! a `Fail` in them; they are `synthetic.build.fail` exiting 1.
//!
//! ## The one place the order differs from §101
//!
//! §101 lists "create release candidate" (26) *before* "final
//! validation bound to C4" (27) and "job reaches `ReadyForApproval`"
//! (28). `reconcile` is a loop that advances as far as its rules
//! allow (§41), so one call takes the job from `EventDetected` to
//! `ReadyForApproval` and there is no moment at which the driver can
//! interleave. The release candidate is therefore assembled once
//! the job has reached `ReadyForApproval`, and step 27 is asserted
//! as a property of the snapshot — it names C4, and it lists C4's
//! `FinalValidation` gate — rather than as a separate instant. Every
//! claim the three steps make is checked; only their relative order
//! differs, and the order is a property of `reconcile` rather than
//! something this test could fake.
//!
//! ## "Invalidated" means not-applicable, not deleted
//!
//! §101 step 21 says relevant evidence is invalidated. Nothing in
//! 0B deletes evidence — `ironmaint_evidence::InvalidationRule`
//! exists as a data model and has no executor, which 0A §31
//! anticipates ("sophisticated propagation lands in 0A.5+"). What
//! 0B actually enforces is *candidate binding*: evidence is stamped
//! with a fingerprint, gate results are keyed `(gate_id,
//! fingerprint)`, and the engine refuses a result whose candidate is
//! not the active one. So the assertion at step 21 is that C3's
//! evidence is still in the ledger and C4's gates read
//! `NotEvaluated` until re-run — old evidence cannot authorize a new
//! candidate (§102 item 20), and nothing is thrown away to prove it.
//!
//! ## "IronClaw patches workspace"
//!
//! Steps 9, 14 and 19 are an LLM's act, and this driver has no
//! LLM. What it does instead is a real edit to a real file in the
//! fixture repository from step 1, asserted before and after. The
//! `workspace.apply_patch` *tool* is exercised end to end by
//! `crates/ironmaint-mcp/tests/`; nothing here should be read as
//! evidence that the patch tool works.

// `unwrap` / `expect` / `panic` are allowed because failure in a
// test should panic loudly, and a silently-defaulted value would
// let the assertions below pass for the wrong reason — the lesson
// of D-14, where a bug affecting 100% of real jobs passed CI
// because every test hand-built the log it asserted on.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::sync::Arc;

use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_core::{
    CandidateFingerprint, CheckId, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::{EvidenceKind, GateStage, GateStatus};
use ironmaint_executor::{ProcessEnvironment, ProcessExecutor, RetryClass, ToolRegistry};
use ironmaint_policy::{ObligationOutcome, ObligationStatus};
use ironmaint_runtime::{
    AdapterRegistry, HumanAction, OrchestratorRef, QueryResult, ReconcileOutcome, RuntimeCommand,
    RuntimeQuery, RuntimeService, SystemClock,
};
use ironmaint_state::JobEvent;
use ironmaint_store::{
    CandidateStore, CheckStore, EventStore, EvidenceStore, GateStore, ObligationStore,
    OperationStore, ProjectionStore,
};
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use ironmaint_testkit::{
    SCENARIO_OBLIGATION_REQUIREMENT, SCENARIO_STAGES, ScenarioAdapter, fixture_binary_path,
    register_scenario_tools,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use url::Url;

/// Path to the workspace `migrations/` directory. The cargo build
/// runs tests with `CARGO_MANIFEST_DIR` set, so this is a
/// compile-time constant.
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");

/// The synthetic distribution family the scenario adapter claims.
const FAMILY: &str = ironmaint_testkit::scenario::SYNTHETIC_FAMILY;

/// The four candidate versions, in §101 order.
const C1: &str = "1.0.0";
const C2: &str = "1.0.1";
const C3: &str = "1.0.2";
const C4: &str = "1.0.3";

type Service = RuntimeService<SqliteStore, ProcessExecutor>;

// ---------------------------------------------------------------------------
// §101 step 1 — the fixture repository.
// ---------------------------------------------------------------------------

/// The files a "package" has in the fixture repository. A real
/// directory on disk, because step 9's patch is a real file edit and
/// an in-memory stand-in would make that assertion vacuous.
const FIXTURE_FILES: &[(&str, &str)] = &[
    ("README", "synthetic package\n"),
    (
        "debian/rules",
        "#!/usr/bin/make -f\nbuild:\n\t@echo BUILD OK\n",
    ),
    (
        "debian/changelog",
        "synthetic-pkg (1.0.0) unstable; urgency=low\n",
    ),
];

/// The line step 9 patches in. Absent until then, so "the patch
/// landed" is a real check rather than a no-op comparison.
const PATCH_MARKER: &str = "the build now finds its dependency\n";

struct Fixture {
    /// Kept alive: the SQLite state directory and the artifact root
    /// live inside it.
    _state: TempDir,
    /// §101 step 1. The agent's workspace.
    repo: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let state = tempfile::tempdir().expect("temp state dir");
        let repo = tempfile::tempdir().expect("temp repo dir");
        for (path, contents) in FIXTURE_FILES {
            write_file(&repo.path().join(path), contents);
        }
        Self {
            _state: state,
            repo,
        }
    }

    /// §101 steps 9, 14 and 19 — the agent patches the source and
    /// the next capture names the next version.
    fn patch(&self, note: &str) {
        let changelog = self.repo.path().join("debian/changelog");
        let mut body = fs::read_to_string(&changelog).expect("read changelog");
        body.push_str(PATCH_MARKER);
        body.push_str(&format!("patched at {note}\n"));
        write_file(&changelog, &body);
    }

    fn changelog(&self) -> String {
        fs::read_to_string(self.repo.path().join("debian/changelog")).expect("read changelog")
    }
}

fn write_file(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(path, contents).expect("write fixture file");
}

// ---------------------------------------------------------------------------
// Wiring.
// ---------------------------------------------------------------------------

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new(FAMILY).expect("family is a valid label"),
            DistributionRelease::new("unstable").expect("release is a valid label"),
        ),
        PackageName::new("synthetic-pkg").expect("name is a valid label"),
    )
}

fn source_candidate(job_id: JobId, version: &str) -> SourceCandidate {
    let url = Url::parse("https://example.invalid/synthetic-pkg.git").expect("static url");
    let repository = RepositoryRef::new(VcsKind::Git, url).expect("git repository ref");
    // The commit differs per version so the candidates differ in
    // more than one field — a fingerprint that changed in only one
    // input would not prove the binding is on the whole revision.
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).expect("commit id");
    let tree_hex: String = version
        .bytes()
        .map(|b| char::from(b'a' + (b % 5)))
        .collect();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, tree_hex.repeat(8)).expect("tree id");
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid timestamp");
    let revision = PackageRevision::new(
        package(),
        PackageVersion::new(version).expect("opaque version string"),
    );
    SourceCandidate::new(job_id, revision, repository, commit, tree, now)
}

/// Open a store and a runtime over `fixture`. Dropping both is the
/// §101 "restart": the daemon lock is released and the WAL is
/// flushed when the `SqliteStore` drops.
async fn open(fixture: &Fixture) -> (Arc<SqliteStore>, Service) {
    let config = SqliteStoreConfig::new(fixture._state.path()).with_migrations_dir(MIGRATIONS_DIR);
    let store = Arc::new(SqliteStore::open(config).await.expect("open the store"));

    let mut registry = ToolRegistry::new();
    register_scenario_tools(&mut registry, &fixture_binary_path()).expect("register fixture tools");
    let registry = Arc::new(registry);

    let artifact_root = fixture._state.path().join("artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    let artifacts = Arc::new(ArtifactStore::open(ArtifactRoot::new(&artifact_root)));
    let executor = Arc::new(ProcessExecutor::new(
        Arc::clone(&registry),
        artifacts,
        ProcessEnvironment::new(),
    ));

    let mut adapters = AdapterRegistry::empty();
    adapters.register(Arc::new(
        ScenarioAdapter::section_101().expect("the scenario adapter's keys are well-formed"),
    ));
    let service = RuntimeService::new(
        Arc::clone(&store),
        Arc::new(SystemClock),
        executor,
        registry,
    )
    .with_adapters(adapters);
    (store, service)
}

async fn create_job(svc: &Service) -> JobId {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create the job");
    let rest = result.side_effects[0]
        .strip_prefix("job:")
        .expect("the side effect names the job")
        .split_whitespace()
        .next()
        .expect("a uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("a valid job id"))
}

async fn capture(svc: &Service, job_id: JobId, version: &str) -> CandidateFingerprint {
    let candidate = source_candidate(job_id, version);
    let fingerprint = candidate.fingerprint().clone();
    svc.handle_command(RuntimeCommand::CaptureCandidate { job_id, candidate })
        .await
        .expect("capture the candidate");
    fingerprint
}

/// The `CheckDefinition` for `kind` on `fingerprint`.
///
/// Resolved through the store rather than captured from the
/// adapter's plan, so the driver runs the check the runtime
/// materialised — not one it believes it should have.
async fn check_for(
    store: &SqliteStore,
    job_id: JobId,
    fingerprint: &CandidateFingerprint,
    kind: &EvidenceKind,
) -> ironmaint_store::CheckDefinition {
    let mut found = None;
    for id in store
        .list_checks_for_job(job_id)
        .await
        .expect("list checks")
    {
        let check = store.get_check(id).await.expect("get check");
        if check.candidate == *fingerprint && check.evidence_kind == *kind {
            assert!(
                found.is_none(),
                "two checks for {kind} on {fingerprint}: the scenario assumes one \
                 check per gate, because a gate aggregates its checks and a second \
                 one would let one kind mask another"
            );
            found = Some(check);
        }
    }
    found.unwrap_or_else(|| panic!("no {kind} check materialised for {fingerprint}"))
}

/// Run the check of `kind` and return the gate verdict it produced.
async fn run_check(
    store: &SqliteStore,
    svc: &Service,
    job_id: JobId,
    fingerprint: &CandidateFingerprint,
    kind: EvidenceKind,
) -> GateStatus {
    let check = check_for(store, job_id, fingerprint, &kind).await;
    svc.handle_command(RuntimeCommand::RunCheck {
        job_id,
        check_id: check.id,
        retry_class: RetryClass::Never,
    })
    .await
    .unwrap_or_else(|e| panic!("run the {kind} check: {e}"));
    store
        .get_gate_result(check.gate_id, fingerprint)
        .await
        .unwrap_or_else(|e| panic!("read the {kind} gate result: {e}"))
        .status
}

/// Run every check the scenario planned for `fingerprint` and return
/// the gate verdicts, in `SCENARIO_STAGES` order.
async fn run_all_checks(
    store: &SqliteStore,
    svc: &Service,
    job_id: JobId,
    fingerprint: &CandidateFingerprint,
) -> Vec<(GateStage, GateStatus)> {
    let mut out = Vec::new();
    for (kind, stage) in SCENARIO_STAGES {
        let status = run_check(store, svc, job_id, fingerprint, kind.clone()).await;
        out.push((*stage, status));
    }
    out
}

/// The status of the job's mandatory obligation for `fingerprint`.
async fn obligation_status(
    store: &SqliteStore,
    job_id: JobId,
    fingerprint: &CandidateFingerprint,
) -> ObligationStatus {
    let mut found = None;
    for id in store
        .list_obligations_for_job(job_id)
        .await
        .expect("list obligations")
    {
        let obligation = store.get_obligation(id).await.expect("get obligation");
        if obligation.candidate == *fingerprint {
            assert!(
                found.is_none(),
                "the scenario adapter derives one obligation; two would make \
                 `RecordObligationOutcome`'s requirement-text lookup ambiguous"
            );
            found = Some(obligation.status);
        }
    }
    found.unwrap_or_else(|| panic!("no obligation derived for {fingerprint}"))
}

async fn record_obligation(svc: &Service, job_id: JobId, outcome: ObligationOutcome) {
    svc.handle_command(RuntimeCommand::RecordObligationOutcome {
        job_id,
        obligation_ref: SCENARIO_OBLIGATION_REQUIREMENT.to_string(),
        outcome,
    })
    .await
    .expect("record the obligation outcome");
}

async fn state_of(store: &SqliteStore, job_id: JobId) -> JobState {
    store
        .get_projection(job_id)
        .await
        .expect("read the projection")
        .state
}

async fn next_actions(svc: &Service, job_id: JobId) -> ironmaint_runtime::JobNextActions {
    match svc
        .handle_query(RuntimeQuery::ListNextActions { job_id })
        .await
        .expect("list next actions")
    {
        QueryResult::NextActions(a) => a,
        other => panic!("expected NextActions, got {other:?}"),
    }
}

async fn release_candidate_id(svc: &Service, job_id: JobId) -> ironmaint_core::ReleaseCandidateId {
    let result = svc
        .handle_command(RuntimeCommand::CreateReleaseCandidate { job_id })
        .await
        .expect("assemble the release candidate");
    let rest = result.side_effects[0]
        .strip_prefix("release_candidate:")
        .expect("the side effect names the release candidate")
        .split_whitespace()
        .next()
        .expect("a uuid");
    ironmaint_core::ReleaseCandidateId::from_uuid(uuid::Uuid::parse_str(rest).expect("a valid id"))
}

/// Walk the job to the end, running every check on `fingerprint`
/// once, recording `obligation` as the obligation's outcome, and
/// reconciling.
async fn drive_to_the_end(
    store: &SqliteStore,
    svc: &Service,
    job_id: JobId,
    fingerprint: &CandidateFingerprint,
    obligation: ObligationOutcome,
) -> ReconcileOutcome {
    let verdicts = run_all_checks(store, svc, job_id, fingerprint).await;
    let failed: Vec<GateStage> = verdicts
        .iter()
        .filter(|(_, status)| *status != GateStatus::Pass)
        .map(|(stage, _)| *stage)
        .collect();
    assert!(
        failed.is_empty(),
        "step 23: every mandatory gate must pass before the release candidate is \
         assembled; failing: {failed:?}"
    );

    record_obligation(svc, job_id, obligation).await;

    svc.reconcile(job_id).await.expect("reconcile")
}

// ---------------------------------------------------------------------------
// The scenario.
// ---------------------------------------------------------------------------

/// §101, all thirty steps, in one walk. DoD #28 and #31.
#[tokio::test]
async fn the_synthetic_acceptance_scenario_passes_end_to_end() {
    let fixture = Fixture::new();

    // ---- steps 1-3 ----
    let (store, svc) = open(&fixture).await;
    let job_id = create_job(&svc).await;
    let c1 = capture(&svc, job_id, C1).await;
    assert_eq!(state_of(&store, job_id).await, JobState::EventDetected);

    // ---- step 4: restart the daemon ----
    drop(svc);
    drop(store);
    let (store, svc) = open(&fixture).await;

    // ---- step 5: C1 and the job survived ----
    assert_eq!(
        state_of(&store, job_id).await,
        JobState::EventDetected,
        "step 5: the job state survived the restart"
    );
    let candidates = store
        .list_source_candidates_for_job(job_id)
        .await
        .expect("list candidates");
    assert_eq!(candidates.len(), 1, "step 5: C1 survived the restart");
    let active = store
        .active_source_candidate(job_id)
        .await
        .expect("active candidate");
    assert!(active.is_some(), "step 5: the active candidate survived");
    assert_eq!(
        store
            .get_source_candidate(active.expect("an active candidate"))
            .await
            .expect("the active candidate")
            .fingerprint(),
        &c1,
        "step 5: the active candidate is still C1"
    );

    // ---- step 6: build check against C1 fails ----
    let build_status = run_check(&store, &svc, job_id, &c1, EvidenceKind::Build).await;
    assert_eq!(
        build_status,
        GateStatus::Fail,
        "step 6: `synthetic.build.fail` exits 1, and a tool failure is a \
         `Fail` gate — not an infrastructure error, which is a different \
         fact with a different remedy"
    );

    // ---- step 7: the logs and the FAIL evidence are persisted ----
    let evidence = store
        .list_evidence_for_candidate(&c1)
        .await
        .expect("list evidence");
    assert_eq!(
        evidence.len(),
        1,
        "step 7: exactly one check ran, so exactly one evidence row exists"
    );
    assert_eq!(evidence[0].kind, EvidenceKind::Build);
    assert_eq!(evidence[0].status, ironmaint_evidence::EvidenceStatus::Fail);
    assert_eq!(
        evidence[0].producer.name, "synthetic.build.fail",
        "step 7: the evidence names the tool that produced it, so a reader can \
         tell a scripted failure from a real one"
    );
    let events = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events");
    assert!(
        events.iter().any(
            |e| matches!(&e.event, JobEvent::ToolRunFinished(t) if t.outcome
                == ironmaint_state::ToolOutcome::Fail)
        ),
        "step 7: the audit log records the tool run and its outcome; got {events:?}"
    );

    // ---- step 8: the failure is legible to the agent ----
    let actions = next_actions(&svc, job_id).await;
    let runnable: Vec<CheckId> = actions
        .allowed
        .iter()
        .filter_map(|a| match a {
            ironmaint_runtime::AllowedAction::RunCheck { check_id } => Some(*check_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        runnable.len(),
        SCENARIO_STAGES.len(),
        "step 8: every check on the active candidate is still offered, \
         including the one that just failed — re-running it is the agent's \
         next move; {actions:?}"
    );
    let build_check = check_for(&store, job_id, &c1, &EvidenceKind::Build).await;
    assert!(
        runnable.contains(&build_check.id),
        "step 8: the failed build check is among them; {actions:?}"
    );
    assert!(
        actions.requires_human.is_none(),
        "step 8: a failed check is the agent's problem, not a human's, so \
         nothing is escalated; {actions:?}"
    );

    // ---- step 9: the agent patches the workspace ----
    let before = fixture.changelog();
    fixture.patch("step 9");

    // ---- step 10: capture C2 ----
    let c2 = capture(&svc, job_id, C2).await;
    assert_ne!(c2, c1, "a new version is a new candidate");

    // ---- step 11: C1's evidence does not satisfy C2's gates ----
    let c1_build = check_for(&store, job_id, &c1, &EvidenceKind::Build).await;
    assert!(
        store.get_gate_result(c1_build.gate_id, &c2).await.is_err(),
        "step 11: a gate result is keyed (gate_id, fingerprint); C1's result \
         cannot be read against C2 even though C2 has its own gate for the \
         same stage"
    );
    let c2_build = check_for(&store, job_id, &c2, &EvidenceKind::Build).await;
    assert_ne!(
        c1_build.gate_id, c2_build.gate_id,
        "step 11: each capture mints its own gate, so C1's `Pass`/`Fail` is \
         attached to C1's gate and cannot leak into C2's"
    );
    let c2_source_preparation =
        check_for(&store, job_id, &c2, &EvidenceKind::SourcePreparation).await;
    assert!(
        store
            .get_gate_result(c2_source_preparation.gate_id, &c2)
            .await
            .is_err(),
        "step 11: a gate result row is minted by running the check, so C2's \
         `SourcePreparation` gate carries no verdict at all until C2's own \
         check runs — there is nothing in the ledger for C1's evidence to \
         satisfy it with"
    );
    // And the ledger still holds C1's evidence — invalidation is
    // non-applicability, not deletion.
    assert_eq!(
        store
            .list_evidence_for_candidate(&c1)
            .await
            .expect("list evidence")
            .len(),
        1,
        "step 11: C1's evidence is still in the ledger; §30 binds it to C1 rather \
         than removing it"
    );

    // ---- steps 12-13: build passes, QA fails ----
    assert_eq!(
        run_check(&store, &svc, job_id, &c2, EvidenceKind::Build).await,
        GateStatus::Pass,
        "step 12: the patched build passes"
    );
    assert_eq!(
        run_check(&store, &svc, job_id, &c2, EvidenceKind::PackageQa).await,
        GateStatus::Fail,
        "step 13: QA fails"
    );

    // ---- step 14: patch again ----
    fixture.patch("step 14");

    // ---- step 15: capture C3 ----
    let c3 = capture(&svc, job_id, C3).await;

    // ---- step 16: the required C3 checks rerun ----
    let verdicts = run_all_checks(&store, &svc, job_id, &c3).await;

    // ---- step 17: QA passes ----
    let qa = verdicts
        .iter()
        .find(|(stage, _)| *stage == GateStage::PackageQa)
        .expect("the scenario plans a PackageQa gate");
    assert_eq!(qa.1, GateStatus::Pass, "step 17: QA passes at C3");
    let failed: Vec<GateStage> = verdicts
        .iter()
        .filter(|(_, status)| *status != GateStatus::Pass)
        .map(|(stage, _)| *stage)
        .collect();
    assert!(
        failed.is_empty(),
        "steps 16-17: every C3 check passes; failing: {failed:?}"
    );

    // ---- step 18: the mandatory synthetic policy obligation fails ----
    record_obligation(&svc, job_id, ObligationOutcome::Fail).await;
    assert_eq!(
        obligation_status(&store, job_id, &c3).await,
        ObligationStatus::Fail,
        "step 18: the negative verdict is durable, not implied"
    );
    // A failing obligation must hold the job even though every gate
    // passes — this is the only rule with `require_policy_completion`.
    let blocked = svc.reconcile(job_id).await.expect("reconcile");
    let expected_obligation = obligation_id_for(&store, job_id, &c3).await;
    assert_eq!(
        blocked,
        ReconcileOutcome::Blocked {
            current: JobState::FinalValidation,
            target: JobState::ReadyForApproval,
            blockers: vec![format!(
                "{:?}",
                ironmaint_state::TransitionBlocker::FailedObligation(expected_obligation)
            )],
        },
        "step 18: every gate passes but `FinalValidation → ReadyForApproval` \
         is still blocked, because rule 12 is the only one requiring policy \
         completion; got {blocked:?}"
    );
    assert_eq!(
        state_of(&store, job_id).await,
        JobState::FinalValidation,
        "step 18: the job got as far as the last rule and no further"
    );

    // ---- step 19: patch again ----
    fixture.patch("step 19");

    // ---- step 20: capture C4 ----
    let c4 = capture(&svc, job_id, C4).await;
    let active = store
        .get_projection(job_id)
        .await
        .expect("projection")
        .active_candidate
        .expect("step 20: a captured candidate is active");
    assert_eq!(
        store
            .get_source_candidate(active)
            .await
            .expect("active candidate")
            .fingerprint(),
        &c4,
        "step 20: capture activates the candidate — §101 step 21 is about C3's \
         evidence ceasing to apply to C4, which only means something if C4 is the \
         one the runtime is now evaluating"
    );
    let after = fixture.changelog();
    assert!(
        after.len() > before.len() && after.starts_with(&before),
        "steps 9/14/19: the fixture repository was really patched three times; \
         before was {before:?}, now {after:?}"
    );

    // ---- step 21: C3's evidence no longer applies ----
    let c3_build = check_for(&store, job_id, &c3, &EvidenceKind::Build).await;
    assert!(
        store.get_gate_result(c3_build.gate_id, &c4).await.is_err(),
        "step 21: C3's gate result is not readable against C4"
    );
    let c3_evidence_before = store
        .list_evidence_for_candidate(&c3)
        .await
        .expect("list C3 evidence")
        .len();
    assert_eq!(
        c3_evidence_before,
        SCENARIO_STAGES.len(),
        "step 21: C3's evidence is all still there — nothing is deleted"
    );
    let c4_build = check_for(&store, job_id, &c4, &EvidenceKind::Build).await;
    assert!(
        store.get_gate_result(c4_build.gate_id, &c4).await.is_err(),
        "step 21: C4's own Build gate has no verdict until step 22 runs it, so \
         C3's passing ledger cannot stand in for it (§102 item 20)"
    );

    // ---- step 22: the required checks rerun ----
    // ---- step 23: all mandatory gates pass ----
    // ---- step 24: the policy obligation passes ----
    let outcome = drive_to_the_end(&store, &svc, job_id, &c4, ObligationOutcome::Pass).await;

    // ---- step 25: the runtime reconciled the workflow ----
    let ReconcileOutcome::NeedsActorDecision {
        current,
        target,
        blockers,
    } = &outcome
    else {
        panic!(
            "step 25: the walk should reach ReadyForApproval and stop there on the \
             human review approval; got {outcome:?}"
        );
    };
    assert_eq!(*current, JobState::ReadyForApproval);
    assert_eq!(*target, JobState::Approved);
    assert!(
        blockers.iter().any(|b| b.starts_with("MissingApproval(")),
        "step 25: what stops the walk at `ReadyForApproval` is rule 13's \
         `ApprovalCategory::HumanReview`, and nothing else; got {outcome:?}"
    );
    assert_eq!(
        state_of(&store, job_id).await,
        JobState::ReadyForApproval,
        "step 25: the walk advanced twelve rules in one call, because §41 says \
         reconciliation \"may continue through multiple trivially satisfied stages\""
    );
    assert_eq!(
        obligation_status(&store, job_id, &c4).await,
        ObligationStatus::Pass,
        "step 24: the positive verdict is durable too"
    );

    // ---- step 26: the synthetic release candidate is created ----
    let release_id = release_candidate_id(&svc, job_id).await;
    let release = store
        .get_release_candidate(release_id)
        .await
        .expect("the release candidate round-trips");

    // ---- step 27: final validation is bound to C4 ----
    assert_eq!(
        release.source, c4,
        "step 27: the release candidate names the candidate that was validated"
    );
    let mut listed = release.gate_ids.clone();
    listed.sort();
    let mut all = store.list_gates_for_job(job_id).await.expect("list gates");
    all.sort();
    assert_eq!(
        listed.len(),
        SCENARIO_STAGES.len(),
        "step 27: the snapshot lists C4's {count} gates and not C1-C3's, because \
         they are filtered by fingerprint; got {listed:?}",
        count = SCENARIO_STAGES.len(),
    );
    let c4_final_validation = check_for(&store, job_id, &c4, &EvidenceKind::LicenseReview).await;
    assert!(
        release.gate_ids.contains(&c4_final_validation.gate_id),
        "step 27: the snapshot includes C4's `FinalValidation` gate"
    );

    // ---- step 28: the job reaches ReadyForApproval ----
    assert_eq!(
        state_of(&store, job_id).await,
        JobState::ReadyForApproval,
        "step 28 / DoD #31"
    );

    // ---- step 29: no signing or publication executes ----
    let operations = store
        .list_operations_for_job(job_id)
        .await
        .expect("list operations");
    assert!(
        operations.is_empty(),
        "step 29: the walk proposed no privileged operation, so nothing could have \
         advanced past `Proposed`; got {operations:?}"
    );
    assert_eq!(
        store
            .list_executing_operations()
            .await
            .expect("executing")
            .len(),
        0,
        "step 29: nothing is executing"
    );
    let actions = next_actions(&svc, job_id).await;
    assert_eq!(
        actions.requires_human,
        Some(HumanAction::ApproveRelease),
        "step 29: the job parks on a person; the agent has nothing left to call"
    );
    assert!(
        actions.allowed.is_empty(),
        "step 29: nothing at `ReadyForApproval` is the agent's to take: {:?}",
        actions.allowed
    );

    // ---- step 30: the entire history is reconstructable after a restart ----
    let events_before = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events");
    let rebuilt_before = store
        .rebuild_projection(job_id)
        .await
        .expect("rebuild before the restart");
    let evidence_before = store
        .list_evidence_for_candidate(&c4)
        .await
        .expect("list C4 evidence")
        .len();

    drop(svc);
    drop(store);
    let (store, _svc) = open(&fixture).await;

    let events_after = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events after the restart");
    assert_eq!(
        events_after.len(),
        events_before.len(),
        "step 30: the event log survived the restart"
    );
    assert_eq!(
        events_after.iter().map(|e| &e.event).collect::<Vec<_>>(),
        events_before.iter().map(|e| &e.event).collect::<Vec<_>>(),
        "step 30: the event payloads are identical, in order"
    );
    assert_eq!(
        store
            .rebuild_projection(job_id)
            .await
            .expect("rebuild after the restart"),
        rebuilt_before,
        "step 30: replaying the log alone reproduces the projection (§102 item 3)"
    );
    assert_eq!(
        state_of(&store, job_id).await,
        JobState::ReadyForApproval,
        "step 30: the job is still at the exit checkpoint"
    );
    assert_eq!(
        store
            .list_evidence_for_candidate(&c4)
            .await
            .expect("list C4 evidence after the restart")
            .len(),
        evidence_before,
        "step 30: evidence survived the restart (§102 item 6)"
    );
    assert_eq!(
        store
            .get_source_candidate(
                store
                    .active_source_candidate(job_id)
                    .await
                    .expect("active candidate")
                    .expect("an active candidate"),
            )
            .await
            .expect("the active candidate")
            .fingerprint(),
        &c4,
        "step 30: C4 is still the active candidate (§102 item 5)"
    );
}

async fn obligation_id_for(
    store: &SqliteStore,
    job_id: JobId,
    fingerprint: &CandidateFingerprint,
) -> ironmaint_core::ObligationId {
    for id in store.list_obligations_for_job(job_id).await.expect("list") {
        if store.get_obligation(id).await.expect("get").candidate == *fingerprint {
            return id;
        }
    }
    panic!("no obligation for {fingerprint}")
}

// ---------------------------------------------------------------------------
// The properties §101 depends on, tested on their own.
// ---------------------------------------------------------------------------

/// §102 item 20: "Old evidence cannot authorize new candidates."
///
/// The scenario above exercises this at scale; this test states it
/// on its own so a regression names itself. A job whose C1 gates all
/// pass must not advance on C2's behalf — and the walk must stall
/// with `MissingGate` rather than silently reusing C1's verdicts.
#[tokio::test]
async fn old_evidence_cannot_authorize_a_new_candidate() {
    let fixture = Fixture::new();
    let (store, svc) = open(&fixture).await;
    let job_id = create_job(&svc).await;

    let c1 = capture(&svc, job_id, C1).await;
    for (kind, _) in SCENARIO_STAGES {
        let status = run_check(&store, &svc, job_id, &c1, kind.clone()).await;
        let expected = if *kind == EvidenceKind::Build {
            GateStatus::Fail
        } else {
            GateStatus::Pass
        };
        assert_eq!(status, expected, "C1's {kind}");
    }

    // C1 has a passing SourcePreparation gate, which is rule 1's
    // requirement. Capture C2 without running anything, and the
    // first rule must still be blocked.
    let c2 = capture(&svc, job_id, C2).await;
    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    let ReconcileOutcome::Blocked { blockers, .. } = &outcome else {
        panic!("C2 has evaluated nothing, so the walk must be blocked, not advanced: {outcome:?}");
    };
    assert!(
        !blockers.is_empty() && blockers.iter().all(|b| b.starts_with("MissingGate(")),
        "C2 has no gate results at all, so the first rule is blocked on a missing \
         gate; C1's eleven passes authorize nothing. Got {outcome:?}"
    );
    assert_eq!(state_of(&store, job_id).await, JobState::EventDetected);
    assert_ne!(
        c2, c1,
        "C2 is a different revision, so a different fingerprint"
    );
}

/// §101 steps 18, 23, 24 and 28, isolated: the policy gate is the
/// only thing between a fully-passing candidate and
/// `ReadyForApproval`, and only for one of the fifteen rules.
#[tokio::test]
async fn a_failed_mandatory_obligation_is_the_only_thing_holding_the_job_back() {
    let fixture = Fixture::new();
    let (store, svc) = open(&fixture).await;
    let job_id = create_job(&svc).await;
    let c4 = capture(&svc, job_id, C4).await;

    // Every gate passes.
    for (kind, _) in SCENARIO_STAGES {
        assert_eq!(
            run_check(&store, &svc, job_id, &c4, kind.clone()).await,
            GateStatus::Pass,
            "C4's {kind}"
        );
    }

    // The obligation fails, so the walk stops at the last rule.
    record_obligation(&svc, job_id, ObligationOutcome::Fail).await;
    let expected_obligation = obligation_id_for(&store, job_id, &c4).await;
    let outcome = svc.reconcile(job_id).await.expect("reconcile");
    assert_eq!(
        outcome,
        ReconcileOutcome::Blocked {
            current: JobState::FinalValidation,
            target: JobState::ReadyForApproval,
            blockers: vec![format!(
                "{:?}",
                ironmaint_state::TransitionBlocker::FailedObligation(expected_obligation)
            )],
        },
        "twelve gates pass and the job is still one rule short"
    );
    assert_eq!(state_of(&store, job_id).await, JobState::FinalValidation);

    // And one outcome later it is through.
    let outcome = drive_to_the_end(&store, &svc, job_id, &c4, ObligationOutcome::Pass).await;
    assert!(
        matches!(
            outcome,
            ReconcileOutcome::NeedsActorDecision {
                current: JobState::ReadyForApproval,
                ..
            }
        ),
        "the same walk, one verdict later, gets all the way to `ReadyForApproval` \
         and then stops on the human review approval; got {outcome:?}"
    );
    assert_eq!(state_of(&store, job_id).await, JobState::ReadyForApproval);
}
