//! §101 as an agent sees it: the whole scenario through the eleven
//! MCP tools, and nothing else (PHASE-0B §102 item 29, DoD #29).
//!
//! ## What this file is for
//!
//! `tests/acceptance_scenario.rs` in this crate already walks §101.
//! That one holds a `RuntimeService` and issues `RuntimeCommand`s, so
//! it proves the *runtime* can do the scenario and says nothing about
//! whether an agent could. This one holds no command handle at all:
//! every step goes through `dispatch`, exactly as a real IronClaw
//! session's tool calls arrive. If a step in §101 needs something only
//! a `RuntimeCommand` can do, this file cannot express it, and that is
//! the finding — which is how C5 found the two gaps its own
//! prerequisites fixed.
//!
//! ## Why it lives here and not in `crates/ironmaint-mcp`
//!
//! §98.6 — "MCP must never bypass runtime" — is enforced by
//! `verify-architecture` against `ironmaint-mcp` in *both* dependency
//! kinds, so a test there cannot hold a store backend even to assert
//! against it. That is the rule working: the constraint is about the
//! crate, not about whether the caller is a test. The testkit is
//! where the store, the fixture binary and the scenario adapter
//! already live, and §98 blesses this direction for test support.
//!
//! ## Where the store is read directly, and why that is not the
//! point
//!
//! The store handle is here, and it is deliberate. `job.get` cannot
//! answer "how many evidence rows does C3 have", "does this gate have
//! a verdict for that fingerprint", or "does `rebuild_projection`
//! reproduce the log" — those are §30's candidate binding and §101
//! step 30, and no tool exposes them. The discipline this file holds
//! to is narrower and checkable: **no `RuntimeCommand`, no
//! `RuntimeQuery`, no `RuntimeService` handle**. Every state change,
//! every check, every capture, every patch goes through `dispatch`.
//! Where a fact is only observable in the store it is read from the
//! store rather than inferred from a tool's own report of itself.
//!
//! ## Four places §101 cannot be read literally
//!
//! Each is decided here rather than faked in the assertions, and each
//! says what it gives up.
//!
//! **"Create fixture repository" is implicit.** §101 step 1 assumes a
//! repository exists. Here the first `candidate.capture` creates it,
//! because `WorkspaceManager::ensure_workspace` is what runs `git
//! init`. An agent has no other way to get a working tree, and there
//! is no tool for one.
//!
//! **Step 18's obligation fails *and* its gate fails.** §53 forbids
//! `ironmaint_obligation_set_pass` and §102 item 25 forbids any
//! obligation-setting tool, so the only way an agent can leave a
//! mandatory obligation failing is for the adapter's policy evaluator
//! to produce failing evidence — and that evidence is also what the
//! `PolicyEvaluation` gate aggregates. So at C3 both fail, and the
//! walk is blocked at rule 5 (`SourceRevision → SourceIntegrity`, which
//! requires that gate) rather than at rule 12. Only rule 12 carries
//! `require_policy_completion`. The C4 driver, holding
//! `RecordObligationOutcome`, isolates rule 12; this one cannot, and
//! does not pretend to. What step 18 asserts *is* checked: the
//! obligation is `Fail` in the store, it is durable, and it is
//! reported to the agent.
//!
//! **Steps 26-28 reorder.** `reconcile` is a loop, so one call takes
//! the job from `EventDetected` to `ReadyForApproval` and there is no
//! instant at which the driver can interleave. The snapshot is
//! assembled after the job has arrived, and step 27 is asserted as a
//! *property* of it — it names C4, and it lists C4's twelve gates and
//! not C1's.
//!
//! **A gate result row is minted by running the check.** A fresh
//! candidate's gate reads `NotFound` rather than `NotEvaluated`,
//! because there is no row until a check runs. That is a *stronger*
//! form of §102 item 20 than "the gate is unevaluated", and the
//! assertions say so.
//!
//! ## What this driver cannot use, and why that is the design
//!
//! There is no obligation tool and there must not be one. The
//! obligation's verdict is *derived* by the adapter when policy
//! evidence lands, so the agent's only move is to run
//! `synthetic.policy.*` and let the evaluator answer. The scenario
//! script therefore varies by capture ordinal rather than by version,
//! because `candidate.capture` names the version itself
//! (`0+ironmaint`) and the tool surface does not expose it; see
//! `ScenarioScript::capture_ordinal` for what that weaker key gives
//! up.
//!
//! Three tools exist because of this file, and one gap in another
//! crate: `job.resume` (C2), `release.candidate.create` (§101 steps
//! 26-27), and — found by this walk — that `candidate.capture` was
//! not idempotent on an unchanged tree, and that an outstanding
//! obligation was invisible to `next_actions` below `ReleaseReview`.
//! Both were fixed in their own commits with their own regression
//! tests, because neither belonged in a diff that also carries the
//! driver which found them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_core::{
    CandidateFingerprint, CheckId, DistributionFamily, DistributionRef, DistributionRelease,
    GateId, JobId, JobState, ObligationId, PackageIdentity, PackageName,
};
use ironmaint_evidence::{EvidenceKind, EvidenceStatus, GateStatus};
use ironmaint_executor::{ProcessEnvironment, ProcessExecutor, ToolRegistry};
use ironmaint_mcp::{McpRuntime, McpToolName, dispatch};
use ironmaint_policy::{ObligationStatus, ReleaseCandidate};
use ironmaint_runtime::{
    ActionBlocker, AdapterRegistry, AllowedAction, HumanAction, JobNextActions, ReconcileOutcome,
    RuntimeService, SystemClock,
};
use ironmaint_state::{JobEvent, ToolOutcome};
use ironmaint_store::{
    CandidateStore, CheckStore, EventStore, EvidenceStore, GateStore, ObligationStore,
    OperationStore, ProjectionStore,
};
use ironmaint_store_sqlite::{SqliteStore, SqliteStoreConfig};
use ironmaint_testkit::executor_conformance::fixture_binary_path;
use ironmaint_testkit::scenario::{
    SCENARIO_OBLIGATION_REQUIREMENT, SCENARIO_STAGES, SYNTHETIC_FAMILY, ScenarioAdapter,
    register_scenario_tools,
};
use ironmaint_workspace::WorkspaceManager;

/// Path to the workspace `migrations/` directory. Cargo does not
/// propagate environment variables to integration tests, so
/// `CARGO_MANIFEST_DIR` plus a relative hop is what is available.
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");

/// The repository URL `candidate.capture` is given. Fixed, because
/// the fingerprint covers it and a changing URL would make every
/// capture a different candidate for a reason that has nothing to do
/// with the scenario.
const REPO: &str = "https://example.invalid/synthetic-pkg.git";

// -----------------------------------------------------------------------------
// Harness.
// -----------------------------------------------------------------------------

type Runtime = McpRuntime<SqliteStore, ProcessExecutor>;

/// The two directories a run needs, held for the whole test so the
/// §101 restarts can reuse them.
struct Fixture {
    /// SQLite state and the artifact tree.
    state: PathBuf,
    /// Root under which `WorkspaceManager` creates one working tree
    /// per job. Separate from `state` so a `git status` in a
    /// workspace can never see the database.
    workspaces: PathBuf,
    /// The scenario adapter, built once for the whole test.
    ///
    /// It outlives the §101 restart at step 4 deliberately, and the
    /// reason is not convenience. `ScenarioScript`'s
    /// capture-ordinal key is a stand-in for "the source content
    /// changed, so the repair took", and a real
    /// `DistributionAdapter`'s plan for a candidate is a pure
    /// function of that candidate — a Debian adapter asked about a
    /// tree it has already seen gives the same answer whether or not
    /// the process restarted in between. Rebuilding the adapter on
    /// the far side of the restart resets the counter, every
    /// subsequent candidate is told it is the first, and C1's
    /// scripted build failure silently reappears on C2.
    ///
    /// What the restart *does* rebuild is everything that holds
    /// state: the store, the executor, the MCP runtime, the
    /// workspace manager. That is the point of steps 4, 5 and 30.
    adapter: Arc<ScenarioAdapter>,
    /// Held so both directories outlive every `Harness` built from
    /// them. A `Harness` borrows the paths; this owns them.
    _tmp: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("a temp root for the scenario");
    let state = tmp.path().join("state");
    let workspaces = tmp.path().join("workspaces");
    std::fs::create_dir_all(&workspaces).expect("the workspace root");
    Fixture {
        state,
        workspaces,
        adapter: Arc::new(
            ScenarioAdapter::section_101_capture_ordinal()
                .expect("the scenario adapter's keys are well-formed"),
        ),
        _tmp: tmp,
    }
}

/// A dispatcher over a real SQLite store, a real git working tree,
/// and the real fixture binary — with no command handle anywhere in
/// the test.
///
/// Dropping it *is* the §101 restart: the `SqliteStore` inside takes
/// an exclusive `fs2` lock on the state directory, so reopening
/// before the old one drops would fail rather than silently give two
/// writers. The test holds no `Arc<SqliteStore>` beyond this.
struct Harness {
    mcp: Runtime,
    store: Arc<SqliteStore>,
}

async fn open(f: &Fixture) -> Harness {
    let config = SqliteStoreConfig::new(&f.state).with_migrations_dir(MIGRATIONS_DIR);
    let store = Arc::new(SqliteStore::open(config).await.expect("open the store"));

    let mut tools = ToolRegistry::new();
    register_scenario_tools(&mut tools, &fixture_binary_path()).expect("register fixture tools");
    let tools = Arc::new(tools);

    let artifact_root = f.state.join("artifacts");
    std::fs::create_dir_all(&artifact_root).expect("artifact root");
    let artifacts = Arc::new(ArtifactStore::open(ArtifactRoot::new(&artifact_root)));
    let executor = Arc::new(ProcessExecutor::new(
        Arc::clone(&tools),
        artifacts,
        ProcessEnvironment::new(),
    ));

    let mut adapters = AdapterRegistry::empty();
    adapters.register(Arc::clone(&f.adapter) as Arc<dyn DistributionAdapter>);
    let service = Arc::new(
        RuntimeService::new(Arc::clone(&store), Arc::new(SystemClock), executor, tools)
            .with_adapters(adapters),
    );

    let workspace = Arc::new(WorkspaceManager::new(
        f.workspaces.clone(),
        Arc::clone(&store),
    ));
    Harness {
        mcp: McpRuntime::new(service).with_workspace(workspace),
        store,
    }
}

// -----------------------------------------------------------------------------
// Tool helpers. Every one of them is a `dispatch` call.
// -----------------------------------------------------------------------------

async fn call(h: &Harness, tool: &str, input: serde_json::Value) -> serde_json::Value {
    dispatch(h.mcp.clone(), &McpToolName(tool.to_string()), input)
        .await
        .unwrap_or_else(|e| panic!("`{tool}` must succeed over the tool surface: {e:?}"))
}

fn package() -> PackageIdentity {
    PackageIdentity::new(
        DistributionRef::new(
            DistributionFamily::new(SYNTHETIC_FAMILY).expect("family"),
            DistributionRelease::new("unstable").expect("release"),
        ),
        PackageName::new("synthetic-pkg").expect("name"),
    )
}

async fn create_job(h: &Harness) -> JobId {
    let out = call(
        h,
        "job.create",
        serde_json::json!({
            "orchestrator": { "kind": "ironclaw" },
            "package": package(),
        }),
    )
    .await;
    serde_json::from_value(out["job_id"].clone()).expect("job_id deserializable")
}

async fn state_of(h: &Harness, job_id: JobId) -> JobState {
    let out = call(h, "job.get", serde_json::json!({ "job_id": job_id })).await;
    serde_json::from_value(out["projection"]["state"].clone()).expect("state deserializable")
}

async fn next_actions(h: &Harness, job_id: JobId) -> JobNextActions {
    let out = call(
        h,
        "job.next_actions",
        serde_json::json!({ "job_id": job_id }),
    )
    .await;
    serde_json::from_value(out["actions"].clone()).expect("actions deserialize")
}

async fn reconcile(h: &Harness, job_id: JobId) -> ReconcileOutcome {
    let out = call(h, "job.reconcile", serde_json::json!({ "job_id": job_id })).await;
    serde_json::from_value(out["outcome"].clone()).expect("outcome deserializable")
}

/// §101 steps 3, 10, 15, 20.
///
/// Also the only way an agent can mark the workspace clean, which is
/// what makes step 9's second patch possible at all — §43 ends the
/// capture pipeline there and `apply_patch` refuses a dirty tree.
async fn capture(h: &Harness, job_id: JobId) -> CandidateFingerprint {
    let out = call(
        h,
        "candidate.capture",
        serde_json::json!({
            "job_id": job_id,
            "package": package(),
            "repository_url": REPO,
        }),
    )
    .await;
    let notes: Vec<String> =
        serde_json::from_value(out["notes"].clone()).expect("notes deserialize");
    assert!(
        notes.iter().any(|n| n.contains("workspace marked clean")),
        "the capture must mark the workspace clean, or the next \
         workspace.apply_patch is refused with a cause the agent has no \
         tool to clear (§43). Got {notes:?}"
    );
    serde_json::from_value(out["fingerprint"].clone()).expect("fingerprint deserializable")
}

async fn revision(h: &Harness, job_id: JobId) -> u64 {
    let out = call(h, "workspace.stat", serde_json::json!({ "job_id": job_id })).await;
    out["revision"].as_u64().expect("revision present")
}

/// A patch adding `path` with `body` as its single line.
fn add_file_patch(path: &str, body: &str) -> String {
    format!(
        "diff --git a/{path} b/{path}\n\
         new file mode 100644\n\
         index 0000000..0000001\n\
         --- /dev/null\n\
         +++ b/{path}\n\
         @@ -0,0 +1 @@\n\
         +{body}\n"
    )
}

/// §101 steps 9, 14, 19.
///
/// The expected revision is *read* rather than assumed: it is what
/// makes the call a real compare-and-swap rather than a decoration,
/// and §101 patches the same tree three times.
async fn apply_patch(h: &Harness, job_id: JobId, path: &str, body: &str) {
    let expected = revision(h, job_id).await;
    let out = call(
        h,
        "workspace.apply_patch",
        serde_json::json!({
            "job_id": job_id,
            "expected_revision": expected,
            "patch": add_file_patch(path, body),
        }),
    )
    .await;
    assert_eq!(
        out["new_revision"].as_u64(),
        Some(expected + 1),
        "a patch bumps the revision by exactly one: {out}"
    );
}

/// §101 steps 4, 6, 12, 13, 16, 22: run the *active* candidate's
/// check of `kind`.
///
/// The fingerprint filter is not decoration. By C2 the job has four
/// candidates and four checks per kind, and `list_checks_for_job`
/// returns them id-sorted — so an unfiltered lookup hands back C1's
/// check, and `check.run` refuses it with "candidate ... != active
/// fingerprint ...". That refusal is the runtime being right and the
/// test being wrong, which is why the scenario driver reads the
/// store's candidate binding rather than taking "the first one I
/// found".
async fn run_check(h: &Harness, job_id: JobId, kind: &EvidenceKind) -> GateStatus {
    let active = active_fingerprint(h, job_id)
        .await
        .unwrap_or_else(|| panic!("no active candidate to run {kind:?} against"));
    let check = check_on(h, job_id, Some(&active), kind).await;
    let out = call(
        h,
        "check.run",
        serde_json::json!({ "job_id": job_id, "check_id": check }),
    )
    .await;
    serde_json::from_value(out["gate_status"].clone()).expect("gate_status deserializable")
}

/// Every check `next_actions` currently offers, in the order it
/// offers them.
///
/// Driven off `next_actions` rather than off the scenario's own stage
/// list, because that is what an agent has: it asks what it can do
/// and does it. Asserting the count against `SCENARIO_STAGES` is what
/// keeps "it did everything" from meaning "it did some of it".
async fn run_all_checks(h: &Harness, job_id: JobId) -> Vec<(EvidenceKind, GateStatus)> {
    let advertised = advertised_checks(h, job_id).await;
    assert_eq!(
        advertised.len(),
        SCENARIO_STAGES.len(),
        "§101 needs a check for every gate the walk crosses, and \
         next_actions must offer all of them; got {:?}",
        advertised
    );
    let mut verdicts = Vec::new();
    for check in advertised {
        let kind = h
            .store
            .get_check(check)
            .await
            .expect("the advertised check resolves")
            .evidence_kind;
        let out = call(
            h,
            "check.run",
            serde_json::json!({ "job_id": job_id, "check_id": check }),
        )
        .await;
        let status: GateStatus =
            serde_json::from_value(out["gate_status"].clone()).expect("gate_status deserializable");
        verdicts.push((kind, status));
    }
    verdicts
}

/// The `check_id`s `job.next_actions` currently offers, read off the
/// tool response — the list an agent would act on.
async fn advertised_checks(h: &Harness, job_id: JobId) -> Vec<CheckId> {
    next_actions(h, job_id)
        .await
        .allowed
        .iter()
        .filter_map(|a| match a {
            AllowedAction::RunCheck { check_id, .. } => Some(*check_id),
            _ => None,
        })
        .collect()
}

// -----------------------------------------------------------------------------
// Store reads the tool surface does not expose.
// -----------------------------------------------------------------------------

/// The `CheckDefinition` materialised for `kind`, optionally
/// restricted to one candidate.
async fn check_on(
    h: &Harness,
    job_id: JobId,
    candidate: Option<&CandidateFingerprint>,
    kind: &EvidenceKind,
) -> CheckId {
    let mut found = None;
    for id in h
        .store
        .list_checks_for_job(job_id)
        .await
        .expect("list checks")
    {
        let check = h.store.get_check(id).await.expect("get check");
        if check.evidence_kind == *kind
            && candidate.is_none_or(|f| check.candidate == *f)
            && found.is_none()
        {
            found = Some(check.id);
        }
    }
    found.unwrap_or_else(|| panic!("no {kind:?} check materialised for candidate {candidate:?}"))
}

/// The gate a candidate's check of `kind` writes its verdict against.
async fn gate_on(
    h: &Harness,
    job_id: JobId,
    candidate: &CandidateFingerprint,
    kind: &EvidenceKind,
) -> GateId {
    h.store
        .get_check(check_on(h, job_id, Some(candidate), kind).await)
        .await
        .expect("get check")
        .gate_id
}

/// C4's obligation, and the scenario adapter is the only thing that
/// derives one — so its absence is a defect, not a state.
async fn obligation_on(
    h: &Harness,
    job_id: JobId,
    candidate: &CandidateFingerprint,
) -> (ObligationId, ObligationStatus) {
    for id in h
        .store
        .list_obligations_for_job(job_id)
        .await
        .expect("list obligations")
    {
        let o = h.store.get_obligation(id).await.expect("get obligation");
        if o.candidate == *candidate {
            assert_eq!(
                o.requirement, SCENARIO_OBLIGATION_REQUIREMENT,
                "the scenario adapter derives exactly one obligation, addressed \
                 by its requirement text"
            );
            return (id, o.status);
        }
    }
    panic!("no obligation derived for {candidate}")
}

async fn release_candidate(h: &Harness, job_id: JobId) -> ReleaseCandidate {
    let out = call(
        h,
        "release.candidate.create",
        serde_json::json!({ "job_id": job_id }),
    )
    .await;
    serde_json::from_value(out["release_candidate"].clone()).expect("the snapshot deserializes")
}

fn failing(verdicts: &[(EvidenceKind, GateStatus)]) -> Vec<EvidenceKind> {
    verdicts
        .iter()
        .filter(|(_, s)| *s != GateStatus::Pass)
        .map(|(k, _)| k.clone())
        .collect()
}

fn status_of(verdicts: &[(EvidenceKind, GateStatus)], kind: &EvidenceKind) -> GateStatus {
    verdicts
        .iter()
        .find(|(k, _)| k == kind)
        .unwrap_or_else(|| panic!("no {kind:?} check ran"))
        .1
}

// -----------------------------------------------------------------------------
// The scenario.
// -----------------------------------------------------------------------------

/// §101, all thirty steps, over the tool surface alone. DoD #29, and
/// #28's property re-proved through the surface an agent actually has.
#[tokio::test]
async fn the_synthetic_acceptance_scenario_completes_over_the_tool_surface() {
    let f = fixture();

    // ---- steps 1-3: a repository, a job, and C1 ----
    // Step 1 has no tool: `candidate.capture` creates the working
    // tree, which is the only way an agent gets one.
    let h = open(&f).await;
    let job_id = create_job(&h).await;
    let c1 = capture(&h, job_id).await;
    assert_eq!(
        state_of(&h, job_id).await,
        JobState::EventDetected,
        "steps 2-3: a captured candidate activates, and the job sits at \
         EventDetected until something has actually been checked"
    );

    // ---- step 4: restart the daemon ----
    drop(h);
    let h = open(&f).await;

    // ---- step 5: C1 and the job survived ----
    assert_eq!(
        state_of(&h, job_id).await,
        JobState::EventDetected,
        "step 5: the job state survived the restart"
    );
    let candidates = h
        .store
        .list_source_candidates_for_job(job_id)
        .await
        .expect("list candidates");
    assert_eq!(candidates.len(), 1, "step 5: C1 survived the restart");
    assert_eq!(
        active_fingerprint(&h, job_id).await.as_ref(),
        Some(&c1),
        "step 5: the active candidate survived, and is still C1"
    );

    // ---- step 6: the build check against C1 fails ----
    assert_eq!(
        run_check(&h, job_id, &EvidenceKind::Build).await,
        GateStatus::Fail,
        "step 6: `synthetic.build.fail` exits 1, and a tool failure is a \
         `Fail` gate — not an infrastructure error, which is a different fact \
         with a different remedy"
    );

    // ---- step 7: the logs and the FAIL evidence are persisted ----
    let evidence = h
        .store
        .list_evidence_for_candidate(&c1)
        .await
        .expect("list evidence");
    assert_eq!(
        evidence.len(),
        1,
        "step 7: exactly one check ran, so exactly one evidence row exists"
    );
    assert_eq!(evidence[0].kind, EvidenceKind::Build);
    assert_eq!(evidence[0].status, EvidenceStatus::Fail);
    assert_eq!(
        evidence[0].producer.name, "synthetic.build.fail",
        "step 7: the evidence names the tool that produced it, so a reader can \
         tell a scripted failure from a real one"
    );
    assert!(
        h.store
            .list_events_for_job(job_id, 1, None)
            .await
            .expect("list events")
            .iter()
            .any(
                |e| matches!(&e.event, JobEvent::ToolRunFinished(t) if t.outcome
                == ToolOutcome::Fail)
            ),
        "step 7: the audit log records the tool run and its outcome"
    );

    // ---- step 8: the failure is legible to the agent ----
    let advertised = advertised_checks(&h, job_id).await;
    assert_eq!(
        advertised.len(),
        SCENARIO_STAGES.len(),
        "step 8: every check on the active candidate is still offered, \
         including the one that just failed — re-running it is the agent's next \
         move"
    );
    assert_eq!(
        next_actions(&h, job_id).await.requires_human,
        None,
        "step 8: a failed check is the agent's problem, not a human's"
    );

    // ---- step 9: the agent patches the workspace ----
    apply_patch(&h, job_id, "debian/patches/fix-build.patch", "step 9").await;

    // ---- step 10: capture C2 ----
    let c2 = capture(&h, job_id).await;
    assert_ne!(c2, c1, "step 10: a patched source is a new candidate");

    // ---- step 11: C1's evidence does not satisfy C2's gates ----
    let c1_build = gate_on(&h, job_id, &c1, &EvidenceKind::Build).await;
    let c2_build = gate_on(&h, job_id, &c2, &EvidenceKind::Build).await;
    assert_ne!(
        c1_build, c2_build,
        "step 11: each capture mints its own gate, so C1's `Fail` is attached to \
         C1's gate and cannot leak into C2's"
    );
    assert!(
        h.store.get_gate_result(c1_build, &c2).await.is_err(),
        "step 11: a gate result is keyed (gate_id, fingerprint); C1's result \
         cannot be read against C2 even by id"
    );
    assert!(
        h.store
            .get_gate_result(
                gate_on(&h, job_id, &c2, &EvidenceKind::SourcePreparation).await,
                &c2,
            )
            .await
            .is_err(),
        "step 11: a gate result row is minted by running the check, so C2's \
         `SourcePreparation` gate carries no verdict at all until C2's own check \
         runs — there is nothing in the ledger for C1's evidence to satisfy it with"
    );
    assert_eq!(
        h.store
            .list_evidence_for_candidate(&c1)
            .await
            .expect("list evidence")
            .len(),
        1,
        "step 11: C1's evidence is still in the ledger; §30 binds it to C1 rather \
         than removing it"
    );

    // ---- steps 12-13: the build passes, QA fails ----
    assert_eq!(
        run_check(&h, job_id, &EvidenceKind::Build).await,
        GateStatus::Pass,
        "step 12: the repair made the build pass"
    );
    assert_eq!(
        run_check(&h, job_id, &EvidenceKind::PackageQa).await,
        GateStatus::Fail,
        "step 13: QA fails"
    );

    // ---- step 14: patch again ----
    apply_patch(&h, job_id, "debian/patches/fix-qa.patch", "step 14").await;

    // ---- step 15: capture C3 ----
    let c3 = capture(&h, job_id).await;
    assert_ne!(c3, c2, "step 15: another patch is another candidate");

    // ---- step 16: the required C3 checks rerun ----
    let c3_verdicts = run_all_checks(&h, job_id).await;

    // ---- step 17: QA passes ----
    assert_eq!(
        status_of(&c3_verdicts, &EvidenceKind::PackageQa),
        GateStatus::Pass,
        "step 17: QA passes at C3"
    );
    assert_eq!(
        failing(&c3_verdicts),
        vec![EvidenceKind::PolicyEvaluation],
        "steps 16-18: at C3 the only failing check is the policy evaluation. \
         §48 says the obligation's verdict is that evaluation's verdict and \
         nothing else, and no tool may write it — so the policy check failing is \
         the *only* way an agent can leave a mandatory obligation failing. The \
         price is that the `PolicyEvaluation` gate fails too."
    );

    // ---- step 18: the mandatory synthetic policy obligation fails ----
    let (_, c3_status) = obligation_on(&h, job_id, &c3).await;
    assert_eq!(
        c3_status,
        ObligationStatus::Fail,
        "step 18: the negative verdict is durable, not implied, and the adapter's \
         evaluator is the only thing that wrote it"
    );
    // And the agent is told, rather than left to infer it from a
    // reconcile that refuses to move. It is reported as a *blocker*
    // and not as a human action: at `EventDetected` the fix is
    // source, which is the agent's work, and SKILL.md §"Human
    // review" says a failed obligation is not an escalation. The
    // distinction is the whole point of the `blockers` /
    // `requires_human` split C3 made.
    let actions = next_actions(&h, job_id).await;
    assert_eq!(
        actions.blockers,
        vec![ActionBlocker::ObligationPending {
            reference: SCENARIO_OBLIGATION_REQUIREMENT.to_string(),
        }],
        "step 18: the outstanding assertion is named, so the agent knows what to \
         fix rather than that something failed"
    );
    assert_eq!(
        actions.requires_human, None,
        "step 18: and it is nobody's but the agent's"
    );
    // A failing obligation and a failing policy gate both hold the
    // job, and which one is *reported* is a property of the rule
    // table: rule 5 (`SourceRevision → SourceIntegrity`) needs the
    // `PolicyEvaluation` gate, and only rule 12 requires policy
    // completion. So the walk advances through every earlier rule
    // and stops there — a blocked reconcile, not a refused one.
    let blocked = reconcile(&h, job_id).await;
    assert!(
        matches!(&blocked, ReconcileOutcome::Blocked { .. }),
        "step 18: the job must not advance past SourceIntegrity on a failed \
         mandatory obligation; got {blocked:?}"
    );
    assert_eq!(
        state_of(&h, job_id).await,
        JobState::SourceRevision,
        "step 18: the walk took the rules it could and stopped at the first one \
         the failed policy evaluation holds. The C4 driver, which can write an \
         obligation verdict directly, isolates rule 12 instead; over the tool \
         surface the policy gate is the earlier wall, and §101 step 18 only says \
         the obligation fails"
    );
    // And it is a wall, not a one-off: asking again does not get
    // past it.
    let again = reconcile(&h, job_id).await;
    assert!(
        matches!(&again, ReconcileOutcome::Blocked { .. }),
        "step 18: re-running the walk over unchanged evidence must not get \
         through; got {again:?}"
    );

    // ---- step 19: patch again ----
    apply_patch(&h, job_id, "debian/patches/fix-policy.patch", "step 19").await;

    // ---- step 20: capture C4 ----
    let c4 = capture(&h, job_id).await;
    assert_ne!(c4, c3, "step 20: the repair is a new candidate");
    assert_eq!(
        active_fingerprint(&h, job_id).await.as_ref(),
        Some(&c4),
        "step 20: capture activates the candidate — §101 step 21 is about C3's \
         evidence ceasing to apply to C4, which only means something if C4 is the \
         one the runtime is now evaluating"
    );

    // ---- step 21: C3's evidence no longer applies ----
    assert!(
        h.store
            .get_gate_result(gate_on(&h, job_id, &c3, &EvidenceKind::Build).await, &c4)
            .await
            .is_err(),
        "step 21: C3's gate result is not readable against C4"
    );
    assert_eq!(
        h.store
            .list_evidence_for_candidate(&c3)
            .await
            .expect("list C3 evidence")
            .len(),
        SCENARIO_STAGES.len(),
        "step 21: C3's evidence is all still there — nothing is deleted"
    );
    let c4_build = gate_on(&h, job_id, &c4, &EvidenceKind::Build).await;
    assert!(
        h.store.get_gate_result(c4_build, &c4).await.is_err(),
        "step 21: C4's own `Build` gate has no verdict until step 22 runs it, so \
         C3's passing ledger cannot stand in for it (§102 item 20)"
    );

    // ---- step 22: the required checks rerun ----
    // ---- step 23: all mandatory gates pass ----
    let c4_verdicts = run_all_checks(&h, job_id).await;
    assert!(
        failing(&c4_verdicts).is_empty(),
        "steps 22-23: every mandatory gate passes at C4; failing: {:?}",
        failing(&c4_verdicts)
    );

    // ---- step 24: the policy obligation passes ----
    let (c4_obligation, c4_status) = obligation_on(&h, job_id, &c4).await;
    assert_eq!(
        c4_status,
        ObligationStatus::Pass,
        "step 24: the positive verdict is durable too, and equally derived — the \
         same evaluator, the same code path, the other answer"
    );

    // ---- step 25: the runtime reconciles the workflow ----
    let outcome = reconcile(&h, job_id).await;
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
        "step 25: what stops the walk is rule 13's `ApprovalCategory::HumanReview`, \
         and nothing else; got {outcome:?}"
    );
    assert_eq!(
        state_of(&h, job_id).await,
        JobState::ReadyForApproval,
        "step 25: the walk advanced through every rule in one call, because §41 \
         says reconciliation may continue through multiple trivially satisfied \
         stages"
    );

    // ---- step 26: the synthetic release candidate is created ----
    let release = release_candidate(&h, job_id).await;

    // ---- step 27: final validation is bound to C4 ----
    assert_eq!(
        release.source, c4,
        "step 27: the release candidate names the candidate that was validated"
    );
    assert_eq!(
        release.gate_ids.len(),
        SCENARIO_STAGES.len(),
        "step 27: the snapshot lists C4's {} gates and not C1-C3's, because they \
         are filtered by fingerprint; got {:?}",
        SCENARIO_STAGES.len(),
        release.gate_ids
    );
    assert!(
        release
            .gate_ids
            .contains(&gate_on(&h, job_id, &c4, &EvidenceKind::LicenseReview).await),
        "step 27: and it includes C4's `FinalValidation` gate"
    );
    assert!(
        !release
            .gate_ids
            .contains(&gate_on(&h, job_id, &c1, &EvidenceKind::LicenseReview).await),
        "step 27: C1's `FinalValidation` gate is not in it"
    );
    assert_eq!(
        release.obligation_ids,
        vec![c4_obligation],
        "step 27: and C4's obligation, not C3's failed one"
    );

    // ---- step 28: the job reaches ReadyForApproval ----
    assert_eq!(
        state_of(&h, job_id).await,
        JobState::ReadyForApproval,
        "step 28 / DoD #31"
    );

    // ---- step 29: no signing or publication executes ----
    assert!(
        h.store
            .list_operations_for_job(job_id)
            .await
            .expect("list operations")
            .is_empty(),
        "step 29: the walk proposed no privileged operation, so nothing could have \
         advanced past `Proposed`"
    );
    assert_eq!(
        h.store
            .list_executing_operations()
            .await
            .expect("executing operations")
            .len(),
        0,
        "step 29: nothing is executing"
    );
    let actions = next_actions(&h, job_id).await;
    assert_eq!(
        actions.requires_human,
        Some(HumanAction::ApproveRelease),
        "step 29: the job parks on a person"
    );
    assert!(
        actions.allowed.is_empty(),
        "step 29: nothing at `ReadyForApproval` is the agent's to take — not even \
         re-running a check, which would overwrite the evidence a human is about \
         to decide on; got {:?}",
        actions.allowed
    );

    // ---- step 30: the entire history is reconstructable after a restart ----
    let events_before: Vec<JobEvent> = h
        .store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events")
        .into_iter()
        .map(|e| e.event)
        .collect();
    let rebuilt_before = h
        .store
        .rebuild_projection(job_id)
        .await
        .expect("rebuild before the restart");
    let evidence_before = h
        .store
        .list_evidence_for_candidate(&c4)
        .await
        .expect("list C4 evidence")
        .len();

    drop(h);
    let h = open(&f).await;

    assert_eq!(
        state_of(&h, job_id).await,
        JobState::ReadyForApproval,
        "step 30: the job is still at the exit checkpoint"
    );
    let events_after: Vec<JobEvent> = h
        .store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("list events after the restart")
        .into_iter()
        .map(|e| e.event)
        .collect();
    assert_eq!(
        events_after, events_before,
        "step 30: the event payloads are identical, in order, and none was lost or \
         gained"
    );
    assert_eq!(
        h.store
            .rebuild_projection(job_id)
            .await
            .expect("rebuild after the restart"),
        rebuilt_before,
        "step 30: replaying the log alone reproduces the projection (§102 item 3)"
    );
    assert_eq!(
        h.store
            .list_evidence_for_candidate(&c4)
            .await
            .expect("list C4 evidence after the restart")
            .len(),
        evidence_before,
        "step 30: evidence survived the restart (§102 item 6)"
    );
    assert_eq!(
        active_fingerprint(&h, job_id).await.as_ref(),
        Some(&c4),
        "step 30: C4 is still the active candidate (§102 item 5)"
    );
    assert!(
        h.store.get_gate_result(c4_build, &c4).await.is_ok(),
        "step 30: the gate verdicts survived too. A job that has reached \
         `ReadyForApproval` on evidence the engine can no longer read has been \
         approved on nothing."
    );
    // And the snapshot, read back through the tool, is the same one.
    let after = release_candidate(&h, job_id).await;
    assert_eq!(
        after, release,
        "step 30: the release candidate survived, and the tool is idempotent rather \
         than minting a second one on the way past"
    );
    assert_eq!(
        h.store
            .list_release_candidates_for_job(job_id)
            .await
            .expect("list snapshots")
            .len(),
        1,
        "and there is exactly one of it"
    );
}

/// The active candidate's fingerprint, through the store. `job.get`
/// reports it as a `CandidateId` and no tool exposes a candidate by
/// id, so this is one of the facts the driver reads directly.
async fn active_fingerprint(h: &Harness, job_id: JobId) -> Option<CandidateFingerprint> {
    let id = match h.store.active_source_candidate(job_id).await {
        Ok(id) => id,
        Err(e) => panic!("read the active candidate: {e}"),
    };
    match id {
        Some(id) => Some(
            h.store
                .get_source_candidate(id)
                .await
                .expect("the active candidate")
                .fingerprint()
                .clone(),
        ),
        None => None,
    }
}

/// §102 item 20: "Old evidence cannot authorize new candidates" —
/// stated on its own, over the tool surface, so a regression names
/// itself rather than showing up as a hundred-line walk failing
/// somewhere in the middle.
#[tokio::test]
async fn old_evidence_cannot_authorize_a_new_candidate() {
    let f = fixture();
    let h = open(&f).await;
    let job_id = create_job(&h).await;

    let c1 = capture(&h, job_id).await;
    // Everything on C1 passes except the build, which is the only
    // scripted failure. Eleven passing gates is far more than rule 1
    // asks for.
    for (kind, _) in SCENARIO_STAGES {
        let expected = if *kind == EvidenceKind::Build {
            GateStatus::Fail
        } else {
            GateStatus::Pass
        };
        assert_eq!(run_check(&h, job_id, kind).await, expected, "C1's {kind:?}");
    }

    // C1 has a passing `SourcePreparation` gate, which is rule 1's
    // requirement. Change the source and capture C2, run nothing,
    // and the walk must still be blocked: C2 has no verdict of its
    // own to stand on.
    apply_patch(
        &h,
        job_id,
        "debian/patches/unrelated.patch",
        "this changes the fingerprint without changing any verdict",
    )
    .await;
    let c2 = capture(&h, job_id).await;
    assert_ne!(c2, c1, "a patched source is a different fingerprint");

    let outcome = reconcile(&h, job_id).await;
    let ReconcileOutcome::Blocked { blockers, .. } = &outcome else {
        panic!("C2 has evaluated nothing, so the walk must be blocked: {outcome:?}");
    };
    assert!(
        !blockers.is_empty() && blockers.iter().all(|b| b.starts_with("MissingGate(")),
        "C2's own gates carry no verdicts, so rule 1 is blocked. C1's eleven \
         passes authorize nothing. Got {outcome:?}"
    );
    assert_eq!(
        state_of(&h, job_id).await,
        JobState::EventDetected,
        "and the job did not move"
    );
}

/// The eleventh tool exists because §101 steps 26-27 are not
/// expressible without it. This states what it does *not* do, since a
/// tool that assembles a release snapshot and a tool that publishes a
/// release are one comma apart in prose and nothing at all in code.
#[tokio::test]
async fn assembling_a_release_candidate_publishes_nothing() {
    let f = fixture();
    let h = open(&f).await;
    let job_id = create_job(&h).await;
    capture(&h, job_id).await;
    run_all_checks(&h, job_id).await;

    let release = release_candidate(&h, job_id).await;

    assert!(
        release.issue_actions.is_empty(),
        "§42's snapshot carries the issue actions a release would file; the \
         scenario adapter plans none, and nothing here invents one"
    );
    assert!(
        h.store
            .list_operations_for_job(job_id)
            .await
            .expect("list operations")
            .is_empty(),
        "no `PrivilegedOperation` exists, so there is nothing that could have \
         advanced past `Proposed` — §54's `ironmaint_publish` is absent from the \
         surface entirely"
    );
    let all: Vec<GateId> = h
        .store
        .list_gates_for_job(job_id)
        .await
        .expect("list gates");
    assert_eq!(
        release.gate_ids.len(),
        all.len(),
        "the job has exactly one candidate, so the snapshot covers every gate: {} \
         listed, {} in the job",
        release.gate_ids.len(),
        all.len()
    );
    // And the state machine did not move for it: the walk is still
    // wherever the evidence put it. C1 fails its build, so it never
    // started — assembling a snapshot is a record of what was
    // validated, not a step toward releasing it.
    assert_eq!(state_of(&h, job_id).await, JobState::EventDetected);
}
