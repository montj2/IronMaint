//! Phase 0B.10 C1 — adapter-driven candidate planning
//! (PHASE-0B.md §67 "load adapter registry", §45 check planning,
//! §48 policy derivation).
//!
//! `CaptureCandidate` is the single production path that activates a
//! candidate, materialises its checks, and derives its obligations.
//! Before 0B.10 all three were reachable from tests only, so an
//! orchestrator driving the tool surface could capture candidates
//! forever and never leave `EventDetected` (`doc/DEBT.md` D-03).
//!
//! Each test pins one observable contract:
//!   1. Capture activates the candidate.
//!   2. Capture materialises exactly the adapter's planned checks.
//!   3. Capture persists the adapter's obligations, bound to the
//!      candidate fingerprint, all `NotEvaluated`.
//!   4. A second capture of the same fingerprint is idempotent.
//!   5. A family with no registered adapter still captures, and says
//!      so in `side_effects`.
//!   6. A job past `CandidateAssembly` captures without activating,
//!      and reports why.
//!   7. An adapter advertising neither `build` nor `policy` derives
//!      nothing, and does not fail.
//!   8. An adapter that errors yields a typed `RuntimeError`.
//!
//! `unwrap`/`expect` are allowed here because failure in a test
//! should panic; the production crate forbids them via the
//! workspace lint table.

// `unimplemented` is allowed for one narrow reason: the test adapter
// must implement all six `DistributionAdapter` methods, and
// `versioning` / `package_model` are not exercised by candidate
// planning. Every method that *is* exercised — `descriptor`,
// `build`, `policy` — is fully implemented below, so reaching a
// `!` would mean the test is asking for something it never wired.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented
)]

use std::sync::Arc;

use ironmaint_adapter_api::{
    AdapterCapabilities, AdapterDescriptor, AdapterError, AdapterErrorKind, BuildCapability,
    BuildPlan, CandidateContext, DistributionAdapter, InspectionCapability, InspectionPlan,
    IssueCapability, ObligationTemplate, PackageModelCapability, PlannedCheck, PolicyCapability,
    PolicyContext, PolicyPlan, QaPlan, ReleaseCapability, ToolCapabilityKey, VersioningCapability,
    verdict_from_evidence_status,
};
use ironmaint_core::{
    AuthorityId, DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm,
    GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision, PackageVersion,
    RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::EvidenceKind;
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationOutcome, ObligationStatus,
    ObligationStrength, PolicyBaseline, PolicyReference,
};
use ironmaint_runtime::{
    AdapterRegistry, Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeErrorKind,
    RuntimeService,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, CheckStore, ObligationStore, ProjectionStore};
use time::OffsetDateTime;
use url::Url;

// -----------------------------------------------------------------------------
// A configurable test adapter.
// -----------------------------------------------------------------------------

/// What the test adapter should report for each capability.
#[derive(Clone, Default)]
struct AdapterScript {
    family: Option<String>,
    build_checks: Vec<(&'static str, EvidenceKind, bool)>,
    qa_checks: Vec<(&'static str, EvidenceKind, bool)>,
    inspection_checks: Vec<(&'static str, EvidenceKind, bool)>,
    obligations: Vec<(ObligationStrength, Applicability, &'static str)>,
    advertise_build: bool,
    advertise_policy: bool,
    advertise_inspection: bool,
    fail_build: bool,
    fail_policy: bool,
}

struct TestAdapter {
    script: AdapterScript,
}

impl TestAdapter {
    fn with_family(mut self, family: &str) -> Self {
        self.script.family = Some(family.to_string());
        self
    }

    fn with_build_checks(mut self, checks: Vec<(&'static str, EvidenceKind, bool)>) -> Self {
        self.script.build_checks = checks;
        self.script.advertise_build = true;
        self
    }

    fn with_qa_checks(mut self, checks: Vec<(&'static str, EvidenceKind, bool)>) -> Self {
        self.script.qa_checks = checks;
        self.script.advertise_build = true;
        self
    }

    fn with_inspection_checks(mut self, checks: Vec<(&'static str, EvidenceKind, bool)>) -> Self {
        self.script.inspection_checks = checks;
        self.script.advertise_inspection = true;
        self
    }

    fn with_obligations(
        mut self,
        obligations: Vec<(ObligationStrength, Applicability, &'static str)>,
    ) -> Self {
        self.script.obligations = obligations;
        self.script.advertise_policy = true;
        self
    }

    fn failing(mut self, fail_build: bool, fail_policy: bool) -> Self {
        self.script.fail_build = fail_build;
        self.script.fail_policy = fail_policy;
        self
    }
}

impl Default for TestAdapter {
    fn default() -> Self {
        Self {
            script: AdapterScript {
                family: Some("debian".to_string()),
                ..AdapterScript::default()
            },
        }
    }
}

impl DistributionAdapter for TestAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        let family = self
            .script
            .family
            .clone()
            .unwrap_or_else(|| "debian".to_string());
        AdapterDescriptor {
            family: DistributionFamily::new(&family).unwrap(),
            implementation_name: format!("test-{family}"),
            implementation_version: "0.0.0".into(),
            capabilities: AdapterCapabilities::new(),
        }
    }

    fn versioning(&self) -> &dyn VersioningCapability {
        unimplemented!("versioning is not consulted by candidate planning")
    }

    fn package_model(&self) -> &dyn PackageModelCapability {
        unimplemented!("package model is not consulted by candidate planning")
    }

    fn policy(&self) -> Option<&dyn PolicyCapability> {
        if self.script.advertise_policy {
            Some(self)
        } else {
            None
        }
    }

    fn build(&self) -> Option<&dyn BuildCapability> {
        if self.script.advertise_build {
            Some(self)
        } else {
            None
        }
    }

    fn issues(&self) -> Option<&dyn IssueCapability> {
        None
    }

    fn release(&self) -> Option<&dyn ReleaseCapability> {
        None
    }
    fn inspection(&self) -> Option<&dyn ironmaint_adapter_api::InspectionCapability> {
        if self.script.advertise_inspection {
            Some(self)
        } else {
            None
        }
    }
}

impl BuildCapability for TestAdapter {
    fn build_plan(&self, _ctx: &CandidateContext) -> Result<BuildPlan, AdapterError> {
        if self.script.fail_build {
            return Err(AdapterError::new(
                AdapterErrorKind::InternalAdapterFailure,
                "scripted build-plan failure",
            ));
        }
        let mut plan = BuildPlan::new();
        for (key, kind, mandatory) in &self.script.build_checks {
            plan = plan.with_check(PlannedCheck::new(tool_key(key), kind.clone(), *mandatory));
        }
        Ok(plan)
    }

    fn qa_plan(&self, _ctx: &CandidateContext) -> Result<QaPlan, AdapterError> {
        if self.script.fail_build {
            return Err(AdapterError::new(
                AdapterErrorKind::InternalAdapterFailure,
                "scripted qa-plan failure",
            ));
        }
        let mut plan = QaPlan::new();
        for (key, kind, mandatory) in &self.script.qa_checks {
            plan = plan.with_check(PlannedCheck::new(tool_key(key), kind.clone(), *mandatory));
        }
        Ok(plan)
    }
}

impl InspectionCapability for TestAdapter {
    fn inspection_plan(&self, _ctx: &CandidateContext<'_>) -> Result<InspectionPlan, AdapterError> {
        let mut plan = InspectionPlan::empty();
        for (key, kind, mandatory) in &self.script.inspection_checks {
            plan.checks
                .push(PlannedCheck::new(tool_key(key), kind.clone(), *mandatory));
        }
        Ok(plan)
    }
}

impl PolicyCapability for TestAdapter {
    fn authority_order(&self) -> Vec<AuthorityClassification> {
        vec![AuthorityClassification::NormativePolicy]
    }

    fn derive_obligation_plan(&self, ctx: &PolicyContext) -> Result<PolicyPlan, AdapterError> {
        if self.script.fail_policy {
            return Err(AdapterError::new(
                AdapterErrorKind::PolicyUnavailable,
                "scripted policy failure",
            ));
        }
        let baseline = PolicyBaseline::new(ctx.candidate.package().package.distribution.clone());
        let templates = self
            .script
            .obligations
            .iter()
            .map(|(strength, applicability, requirement)| {
                ObligationTemplate::new(
                    PolicyReference::new(AuthorityId::new()),
                    *strength,
                    *applicability,
                    *requirement,
                )
            })
            .collect();
        Ok(PolicyPlan {
            baseline,
            obligation_templates: templates,
        })
    }

    /// §48: the verdict is the evaluator's, and nothing else. These
    /// tests exercise derivation, not evaluation, so this exists to
    /// keep the trait whole.
    fn evaluate_obligation(
        &self,
        _context: &PolicyContext,
        obligation: &ObligationTemplate,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<ObligationOutcome, AdapterError> {
        if !self
            .script
            .obligations
            .iter()
            .any(|(_, _, requirement)| requirement == &obligation.requirement)
        {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                format!(
                    "`{}` is not an obligation this adapter derives",
                    obligation.requirement
                ),
            ));
        }
        verdict_from_evidence_status(evidence.status)
    }
}

fn tool_key(literal: &str) -> ToolCapabilityKey {
    ToolCapabilityKey::new(literal).expect("test literal must be a valid tool key")
}

fn registry_with(adapter: TestAdapter) -> AdapterRegistry {
    let mut reg = AdapterRegistry::empty();
    reg.register(Arc::new(adapter));
    reg
}

// -----------------------------------------------------------------------------
// Fixtures.
// -----------------------------------------------------------------------------

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

fn source_candidate(job_id: JobId, version: &str) -> SourceCandidate {
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let pkg_revision = PackageRevision::new(package(), PackageVersion::new(version).unwrap());
    SourceCandidate::new(job_id, pkg_revision, repository, commit, tree, now)
}

fn build_service(
    store: Arc<MockStore>,
    registry: AdapterRegistry,
) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
    .with_adapters(registry)
}

async fn create_job(svc: &RuntimeService<MockStore, NullExecutor>) -> JobId {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let rest = result.side_effects[0]
        .strip_prefix("job:")
        .expect("side-effect prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("valid uuid"))
}

async fn capture(
    svc: &RuntimeService<MockStore, NullExecutor>,
    job_id: JobId,
    version: &str,
) -> Vec<String> {
    svc.handle_command(RuntimeCommand::CaptureCandidate {
        job_id,
        candidate: source_candidate(job_id, version),
    })
    .await
    .expect("capture")
    .side_effects
}

async fn checks_for(store: &MockStore, job_id: JobId) -> Vec<ironmaint_store::CheckDefinition> {
    let mut out = Vec::new();
    for id in store
        .list_checks_for_job(job_id)
        .await
        .expect("list checks")
    {
        out.push(store.get_check(id).await.expect("get check"));
    }
    out
}

async fn obligations_for(store: &MockStore, job_id: JobId) -> Vec<ironmaint_policy::Obligation> {
    let mut out = Vec::new();
    for id in store
        .list_obligations_for_job(job_id)
        .await
        .expect("list obligations")
    {
        out.push(store.get_obligation(id).await.expect("get obligation"));
    }
    out
}

// -----------------------------------------------------------------------------
// 1. Capture activates the candidate.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn capture_activates_the_candidate() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone(), registry_with(TestAdapter::default()));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;

    let projection = store.get_projection(job_id).await.expect("projection");
    assert!(
        projection.active_candidate.is_some(),
        "capture must leave the candidate active; an orchestrator that cannot \
         see which candidate it is evaluating cannot drive the workflow"
    );
    assert_eq!(projection.state, JobState::EventDetected);
}

#[tokio::test]
async fn capture_reports_the_activation_in_its_side_effects() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone(), registry_with(TestAdapter::default()));
    let job_id = create_job(&svc).await;

    let notes = capture(&svc, job_id, "1.0.0").await;
    assert!(
        notes.iter().any(|n| n.contains("activated fingerprint")),
        "capture must narrate the activation, got {notes:?}"
    );
}

// -----------------------------------------------------------------------------
// 2. Capture materialises the adapter's planned checks.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn capture_materialises_exactly_the_adapters_planned_checks() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_build_checks(vec![("test.build.compile", EvidenceKind::Build, true)])
        .with_qa_checks(vec![
            ("test.qa.lint", EvidenceKind::PackageQa, true),
            ("test.qa.optional", EvidenceKind::PackageQa, false),
        ]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;

    let checks = checks_for(&store, job_id).await;
    let mut keys: Vec<&str> = checks.iter().map(|c| c.capability.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["test.build.compile", "test.qa.lint", "test.qa.optional"]
    );
    assert!(
        checks
            .iter()
            .all(|c| c.candidate == *source_candidate(job_id, "1.0.0").fingerprint()),
        "every materialised check must be bound to the captured candidate"
    );
    let mandatory: Vec<bool> = checks.iter().map(|c| c.mandatory).collect();
    assert!(mandatory.contains(&true) && mandatory.contains(&false));
}

#[tokio::test]
async fn capture_materialises_inspection_checks_alongside_build_and_qa() {
    // PHASE-1.md §12: "The runtime aggregates inspection checks
    // during candidate capture exactly as it already aggregates
    // build/QA checks." This test pins that contract: when the
    // adapter advertises `SourceInspection` (gated by
    // `advertise_inspection` in `TestAdapter`) and returns an
    // `InspectionPlan` with one or more `PlannedCheck` records,
    // those records appear in the same materialised set as the
    // build/QA plans — the runtime doesn't distinguish "this came
    // from `inspection_plan`" at the materialise step. The
    // evidence-kind field on each `PlannedCheck` is what puts the
    // check on the right gate; `SourcePreparation` and
    // `SourceIntegrity` map to gates that exist today.
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_build_checks(vec![("test.build.compile", EvidenceKind::Build, true)])
        .with_qa_checks(vec![("test.qa.lint", EvidenceKind::PackageQa, true)])
        .with_inspection_checks(vec![
            (
                "test.inspect.source_walk",
                EvidenceKind::SourcePreparation,
                true,
            ),
            (
                "test.inspect.source_verify",
                EvidenceKind::SourceIntegrity,
                false,
            ),
        ]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;

    let checks = checks_for(&store, job_id).await;
    let mut keys: Vec<&str> = checks.iter().map(|c| c.capability.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "test.build.compile",
            "test.inspect.source_verify",
            "test.inspect.source_walk",
            "test.qa.lint",
        ],
        "inspection checks must materialise alongside build/qa checks in the same set"
    );
}

#[tokio::test]
async fn capture_without_inspection_capability_does_not_add_inspection_checks() {
    // Negative control: when the adapter's `inspection()` returns
    // `None` (the Debian stub case), the materialised set is
    // exactly the build + qa plan — no inspection tuples leak in
    // from somewhere else. This pins the "adapters that don't
    // advertise SourceInspection contribute zero inspection
    // tuples" half of §12.
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_build_checks(vec![("test.build.compile", EvidenceKind::Build, true)])
        .with_qa_checks(vec![("test.qa.lint", EvidenceKind::PackageQa, true)]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;

    let checks = checks_for(&store, job_id).await;
    let mut keys: Vec<&str> = checks.iter().map(|c| c.capability.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["test.build.compile", "test.qa.lint"]);
}

#[tokio::test]
async fn each_planned_check_gets_its_own_gate_definition() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_build_checks(vec![("test.build.compile", EvidenceKind::Build, true)])
        .with_qa_checks(vec![("test.qa.lint", EvidenceKind::PackageQa, true)]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;

    let checks = checks_for(&store, job_id).await;
    let gate_ids: std::collections::BTreeSet<_> = checks.iter().map(|c| c.gate_id).collect();
    assert_eq!(
        gate_ids.len(),
        2,
        "a build gate and a QA gate are different stages and must not \
         share one GateDefinition"
    );
}

// -----------------------------------------------------------------------------
// 3. Capture persists the adapter's obligations.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn capture_persists_the_adapters_obligations() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default().with_obligations(vec![
        (
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "must build reproducibly",
        ),
        (
            ObligationStrength::Recommended,
            Applicability::Applicable,
            "prefer quilt series",
        ),
    ]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;
    let candidate = source_candidate(job_id, "1.0.0");

    capture(&svc, job_id, "1.0.0").await;

    let obligations = obligations_for(&store, job_id).await;
    assert_eq!(obligations.len(), 2);
    for obligation in &obligations {
        assert_eq!(
            obligation.candidate,
            *candidate.fingerprint(),
            "obligations are candidate-scoped; an unscoped one would block \
             a future candidate on the grounds of an earlier one"
        );
        assert_eq!(
            obligation.status,
            ObligationStatus::NotEvaluated,
            "the runtime may not pre-judge an obligation — the adapter says \
             what must hold, not whether it does"
        );
    }
    let strengths: Vec<_> = obligations.iter().map(|o| o.strength).collect();
    assert!(strengths.contains(&ObligationStrength::Mandatory));
    assert!(strengths.contains(&ObligationStrength::Recommended));
}

#[tokio::test]
async fn a_mandatory_applicable_obligation_survives_materialisation_as_blocking() {
    // The reason the obligation exists at all: a mandatory,
    // applicable obligation that is not `Pass` must trigger the §35
    // invariant, or the state machine would advance a job whose
    // policy assertion has never been checked.
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default().with_obligations(vec![(
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "must build reproducibly",
    )]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;

    let obligations = obligations_for(&store, job_id).await;
    assert!(obligations[0].triggers_section_35_invariant());
}

// -----------------------------------------------------------------------------
// 4. Idempotence.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn a_second_capture_of_the_same_fingerprint_is_idempotent() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_build_checks(vec![("test.build.compile", EvidenceKind::Build, true)])
        .with_obligations(vec![(
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "must build reproducibly",
        )]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;
    let checks_after_first = checks_for(&store, job_id).await.len();
    let obligations_after_first = obligations_for(&store, job_id).await.len();

    let notes = capture(&svc, job_id, "1.0.0").await;

    assert_eq!(
        checks_for(&store, job_id).await.len(),
        checks_after_first,
        "re-capturing must not mint a second GateDefinition for the same stage"
    );
    assert_eq!(
        obligations_for(&store, job_id).await.len(),
        obligations_after_first,
        "re-capturing must not duplicate obligations"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.contains("already the active candidate")),
        "the second capture should say it changed nothing, got {notes:?}"
    );
}

#[tokio::test]
async fn a_new_fingerprint_gets_its_own_checks_and_obligations() {
    // The §101 shape: a repaired workspace yields a new candidate,
    // which must get its own contracts rather than inheriting the
    // old candidate's.
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_build_checks(vec![("test.build.compile", EvidenceKind::Build, true)])
        .with_obligations(vec![(
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "must build reproducibly",
        )]);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;
    let first = source_candidate(job_id, "1.0.0");
    capture(&svc, job_id, "1.0.0-1").await;
    let second = source_candidate(job_id, "1.0.0-1");
    assert_ne!(first.fingerprint(), second.fingerprint());

    let checks = checks_for(&store, job_id).await;
    let on_first = checks
        .iter()
        .filter(|c| c.candidate == *first.fingerprint())
        .count();
    let on_second = checks
        .iter()
        .filter(|c| c.candidate == *second.fingerprint())
        .count();
    assert_eq!((on_first, on_second), (1, 1));

    let obligations = obligations_for(&store, job_id).await;
    assert_eq!(obligations.len(), 2, "one obligation per candidate");
    let projection = store.get_projection(job_id).await.expect("projection");
    let second_id = store
        .find_source_by_fingerprint(second.fingerprint())
        .await
        .expect("lookup")
        .expect("captured");
    assert_eq!(projection.active_candidate, Some(second_id));
}

// -----------------------------------------------------------------------------
// 5. No adapter for the family.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn a_family_with_no_registered_adapter_still_captures() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default().with_family("fedora");
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    // The job is `debian`; only `fedora` is registered.
    let notes = capture(&svc, job_id, "1.0.0").await;

    assert!(
        checks_for(&store, job_id).await.is_empty(),
        "no adapter, no checks — inventing checks would make the gate \
         unsatisfiable by anything that can actually run"
    );
    assert!(obligations_for(&store, job_id).await.is_empty());
    let note = notes
        .iter()
        .find(|n| n.contains("no adapter registered for family `debian`"))
        .unwrap_or_else(|| panic!("expected a 'no adapter' note, got {notes:?}"));
    assert!(note.contains("debian"));
}

#[tokio::test]
async fn an_empty_registry_captures_without_panicking() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone(), AdapterRegistry::empty());
    let job_id = create_job(&svc).await;

    let notes = capture(&svc, job_id, "1.0.0").await;

    assert!(notes.iter().any(|n| n.contains("no adapter registered")));
    // The candidate is still durable, and still active: activation
    // does not depend on the adapter.
    let projection = store.get_projection(job_id).await.expect("projection");
    assert!(projection.active_candidate.is_some());
}

// -----------------------------------------------------------------------------
// 6. Where the repair loop stops.
// -----------------------------------------------------------------------------

/// Park the job in `state` without going through the engine: this
/// is about what capture does in that state, not about how the job
/// got there.
async fn park(store: &MockStore, job_id: JobId, state: JobState) {
    let mut projection = store.get_projection(job_id).await.expect("projection");
    projection.state = state;
    store
        .put_projection(&projection, projection.version)
        .await
        .expect("put projection");
}

/// §61's loop is "inspect the failure, capture a new candidate,
/// rerun the required checks", and the failures it repairs happen
/// in `BuildValidation` and `PackageQaValidation`. A guard that
/// confined activation to the four pre-build states made the loop
/// unreachable — which is how §101 steps 15 and 20 were found to
/// be unrepresentable.
#[tokio::test]
async fn a_repairable_state_still_activates_the_new_candidate() {
    for state in [
        JobState::SourceRevision,
        JobState::BuildValidation,
        JobState::PackageQaValidation,
        JobState::FinalValidation,
    ] {
        let store = Arc::new(MockStore::new());
        let svc = build_service(store.clone(), registry_with(TestAdapter::default()));
        let job_id = create_job(&svc).await;
        park(&store, job_id, state).await;

        capture(&svc, job_id, "1.0.0").await;

        let after = store.get_projection(job_id).await.expect("projection");
        assert!(
            after.active_candidate.is_some(),
            "a job in {state} is mid-repair and a new candidate is how it is \
             repaired; capture must activate it"
        );
        assert_eq!(after.state, state, "capture must not move the job");
    }
}

/// The line is `ReadyForApproval`, where the source and the
/// evidence behind it are what a human is about to decide on.
/// Swapping either there would change the subject of the review
/// without recording that it changed.
#[tokio::test]
async fn a_decided_job_captures_without_activating() {
    for state in [
        JobState::ReadyForApproval,
        JobState::Approved,
        JobState::PublicationPending,
        JobState::Published,
        JobState::Cancelled,
    ] {
        let store = Arc::new(MockStore::new());
        let svc = build_service(store.clone(), registry_with(TestAdapter::default()));
        let job_id = create_job(&svc).await;
        park(&store, job_id, state).await;

        let notes = capture(&svc, job_id, "1.0.0").await;

        let after = store.get_projection(job_id).await.expect("projection");
        assert_eq!(
            after.active_candidate, None,
            "a job in {state} is decided; capture must not swap the source under \
             the reviewer"
        );
        assert_eq!(after.state, state, "capture must not move the job");
        assert!(
            notes.iter().any(|n| n.contains("did not activate it")),
            "the refusal must be visible, not silent; got {notes:?}"
        );
        // The candidate is still durable — a capture that is refused for
        // activation is not a capture that is refused outright.
        assert_eq!(
            store
                .list_source_candidates_for_job(job_id)
                .await
                .expect("list")
                .len(),
            1
        );
    }
}

/// A job awaiting human intervention is not one an agent may
/// quietly re-point at new source, and `allowed` must not offer it
/// the checks to do so with. §21: return to the recorded state
/// first.
#[tokio::test]
async fn an_exceptional_job_captures_without_activating() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone(), registry_with(TestAdapter::default()));
    let job_id = create_job(&svc).await;
    park(&store, job_id, JobState::HumanReviewRequired).await;

    let notes = capture(&svc, job_id, "1.0.0").await;

    assert!(
        notes.iter().any(|n| n.contains("resume the job first")),
        "the note must name the remedy, not just the refusal; got {notes:?}"
    );
    assert_eq!(
        store
            .get_projection(job_id)
            .await
            .expect("projection")
            .active_candidate,
        None
    );
}

// -----------------------------------------------------------------------------
// 7. Adapter without the relevant capabilities.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn an_adapter_with_neither_build_nor_policy_derives_nothing() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default().with_family("debian");
    // Default TestAdapter advertises neither.
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    let notes = capture(&svc, job_id, "1.0.0").await;

    assert!(checks_for(&store, job_id).await.is_empty());
    assert!(obligations_for(&store, job_id).await.is_empty());
    assert!(
        !notes.iter().any(|n| n.contains("materialized")),
        "a plan of zero checks is not a materialisation; got {notes:?}"
    );
    let projection = store.get_projection(job_id).await.expect("projection");
    assert!(projection.active_candidate.is_some());
}

// -----------------------------------------------------------------------------
// 8. Adapter errors are typed, not panics.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn a_failing_build_plan_surfaces_as_a_typed_runtime_error() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_build_checks(vec![("test.build.compile", EvidenceKind::Build, true)])
        .failing(true, false);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    let err = svc
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id,
            candidate: source_candidate(job_id, "1.0.0"),
        })
        .await
        .expect_err("adapter failure must not be swallowed");

    assert_eq!(
        err.kind,
        RuntimeErrorKind::Other("internal_adapter_failure".to_string())
    );
    assert!(
        err.message.contains("scripted build-plan failure"),
        "the adapter's own explanation must survive the mapping, got {}",
        err.message
    );
}

#[tokio::test]
async fn a_failing_policy_plan_surfaces_as_a_typed_runtime_error() {
    let store = Arc::new(MockStore::new());
    let adapter = TestAdapter::default()
        .with_obligations(vec![(
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            "must build reproducibly",
        )])
        .failing(false, true);
    let svc = build_service(store.clone(), registry_with(adapter));
    let job_id = create_job(&svc).await;

    let err = svc
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id,
            candidate: source_candidate(job_id, "1.0.0"),
        })
        .await
        .expect_err("policy failure must not be swallowed");

    assert_eq!(
        err.kind,
        RuntimeErrorKind::Other("policy_unavailable".to_string())
    );
    assert!(err.message.contains("scripted policy failure"));
}

#[tokio::test]
async fn a_mismatched_candidate_job_id_is_still_rejected_before_any_planning() {
    // The fingerprint/job mismatch check must stay first: planning
    // against a candidate belonging to another job would write
    // checks that a different job's state machine then walks.
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone(), registry_with(TestAdapter::default()));
    let job_id = create_job(&svc).await;
    let other = JobId::new();

    let err = svc
        .handle_command(RuntimeCommand::CaptureCandidate {
            job_id,
            candidate: source_candidate(other, "1.0.0"),
        })
        .await
        .expect_err("mismatched job id must be rejected");

    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
    assert!(checks_for(&store, job_id).await.is_empty());
}
