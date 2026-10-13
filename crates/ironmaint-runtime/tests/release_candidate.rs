//! `CreateReleaseCandidate` — the 0A §42 snapshot (PHASE-0B.md
//! §101 step 26).
//!
//! `ReleaseCandidate` and `put_release_candidate` have existed
//! since 0A, and until 0B.10 nothing in production ever called
//! them: the type was reachable only from `ironmaint-policy`'s own
//! unit tests. §101 requires a release candidate to be created
//! between "all gates pass" and "Final validation", and 0A §42 is
//! explicit that creating one is an act distinct from becoming
//! releasable — so it needs a path, and it needs that path to
//! refuse to do anything else.
//!
//! What each test pins:
//!   1. The snapshot is bound to the *active* candidate.
//!   2. It lists the gates and obligations of that candidate only.
//!   3. A superseded candidate's gates cannot leak into it.
//!   4. A job with no active candidate is refused.
//!   5. Creating one advances nothing and creates no
//!      `PrivilegedOperation` — 0A §42's "does not mean
//!      releasable", and §101 step 29 in miniature.

// `unimplemented` is allowed for one narrow reason: the test
// adapter must implement all six `DistributionAdapter` methods,
// and only `descriptor`, `build` and `policy` are consulted here.
// `versioning` / `package_model` returning `!` is a loud signal
// that a future edit started depending on them without saying so.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented
)]

use std::sync::Arc;

use ironmaint_adapter_api::{
    AdapterCapabilities, AdapterDescriptor, AdapterError, BuildCapability, BuildPlan,
    CandidateContext, DistributionAdapter, IssueCapability, ObligationTemplate,
    PackageModelCapability, PlannedCheck, PolicyCapability, PolicyContext, PolicyPlan, QaPlan,
    ReleaseCapability, ToolCapabilityKey, VersioningCapability, verdict_from_evidence_status,
};
use ironmaint_core::{
    AuthorityId, DistributionFamily, DistributionRef, DistributionRelease, GateId,
    GitHashAlgorithm, GitObjectId, JobId, ObligationId, PackageIdentity, PackageName,
    PackageRevision, PackageVersion, ReleaseCandidateId, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_evidence::EvidenceKind;
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationOutcome, ObligationStrength, PolicyBaseline,
    PolicyReference,
};
use ironmaint_runtime::{
    AdapterRegistry, FixedClock, OrchestratorRef, QueryResult, RuntimeCommand, RuntimeErrorKind,
    RuntimeQuery, RuntimeService,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{
    CandidateStore, EventStore, GateStore, ObligationStore, OperationStore, ProjectionStore,
};
use time::OffsetDateTime;
use url::Url;

// -----------------------------------------------------------------------------
// A minimal adapter: two checks and one obligation.
// -----------------------------------------------------------------------------

struct SnapshotAdapter;

impl DistributionAdapter for SnapshotAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            family: DistributionFamily::new("debian").unwrap(),
            implementation_name: "test-snapshot".into(),
            implementation_version: "0.0.0".into(),
            capabilities: AdapterCapabilities::new(),
        }
    }

    fn versioning(&self) -> &dyn VersioningCapability {
        unimplemented!("versioning is not consulted by release-candidate assembly")
    }

    fn package_model(&self) -> &dyn PackageModelCapability {
        unimplemented!("package model is not consulted by release-candidate assembly")
    }

    fn policy(&self) -> Option<&dyn PolicyCapability> {
        Some(self)
    }

    fn build(&self) -> Option<&dyn BuildCapability> {
        Some(self)
    }

    fn issues(&self) -> Option<&dyn IssueCapability> {
        None
    }

    fn release(&self) -> Option<&dyn ReleaseCapability> {
        None
    }
    fn inspection(&self) -> Option<&dyn ironmaint_adapter_api::InspectionCapability> {
        None
    }
}

impl BuildCapability for SnapshotAdapter {
    fn build_plan(&self, _ctx: &CandidateContext) -> Result<BuildPlan, AdapterError> {
        let mut plan = BuildPlan::new();
        plan = plan.with_check(PlannedCheck::new(
            tool_key("test.build.compile"),
            EvidenceKind::Build,
            true,
        ));
        plan = plan.with_check(PlannedCheck::new(
            tool_key("test.qa.lint"),
            EvidenceKind::PackageQa,
            true,
        ));
        Ok(plan)
    }

    fn qa_plan(&self, _ctx: &CandidateContext) -> Result<QaPlan, AdapterError> {
        Ok(QaPlan::new())
    }
}

impl PolicyCapability for SnapshotAdapter {
    fn authority_order(&self) -> Vec<AuthorityClassification> {
        vec![AuthorityClassification::NormativePolicy]
    }

    fn derive_obligation_plan(&self, ctx: &PolicyContext) -> Result<PolicyPlan, AdapterError> {
        Ok(PolicyPlan {
            baseline: PolicyBaseline::new(ctx.candidate.package().package.distribution.clone()),
            obligation_templates: vec![ObligationTemplate::new(
                PolicyReference::new(AuthorityId::new()),
                ObligationStrength::Mandatory,
                Applicability::Applicable,
                SNAPSHOT_OBLIGATION_REQUIREMENT,
            )],
        })
    }

    /// §48: the verdict is the evaluator's, and nothing else.
    /// These tests never run the evaluator, so this exists to keep
    /// the trait whole rather than to be exercised.
    fn evaluate_obligation(
        &self,
        _context: &PolicyContext,
        obligation: &ObligationTemplate,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<ObligationOutcome, AdapterError> {
        if obligation.requirement != SNAPSHOT_OBLIGATION_REQUIREMENT {
            return Err(AdapterError::new(
                ironmaint_adapter_api::AdapterErrorKind::InvalidConfiguration,
                format!(
                    "`{}` is not an obligation this adapter derives",
                    obligation.requirement
                ),
            ));
        }
        verdict_from_evidence_status(evidence.status)
    }
}

/// The one obligation `SnapshotAdapter` derives. Named so
/// `evaluate_obligation` and `derive_obligation_plan` cannot
/// disagree about it.
const SNAPSHOT_OBLIGATION_REQUIREMENT: &str = "policy.source-is-maintained";

fn tool_key(literal: &str) -> ToolCapabilityKey {
    ToolCapabilityKey::new(literal).expect("test literal must be a valid tool key")
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

fn source_candidate(job_id: JobId, version: &str) -> SourceCandidate {
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).unwrap();
    let tree = GitObjectId::new(GitHashAlgorithm::Sha1, "2".repeat(40)).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let revision = PackageRevision::new(package(), PackageVersion::new(version).unwrap());
    SourceCandidate::new(job_id, revision, repository, commit, tree, now)
}

fn build_service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    let mut registry = AdapterRegistry::empty();
    registry.register(Arc::new(SnapshotAdapter));
    RuntimeService::new(
        store,
        Arc::new(FixedClock::new(
            OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        )),
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

async fn capture(svc: &RuntimeService<MockStore, NullExecutor>, job_id: JobId, version: &str) {
    svc.handle_command(RuntimeCommand::CaptureCandidate {
        job_id,
        candidate: source_candidate(job_id, version),
    })
    .await
    .expect("capture");
}

/// Create the release candidate and read it back, returning the
/// side effects alongside so a test can assert on what the caller
/// is told.
async fn create_release_candidate(
    svc: &RuntimeService<MockStore, NullExecutor>,
    store: &MockStore,
    job_id: JobId,
) -> (ironmaint_policy::ReleaseCandidate, Vec<String>) {
    let result = svc
        .handle_command(RuntimeCommand::CreateReleaseCandidate { job_id })
        .await
        .expect("create release candidate");
    let side = &result.side_effects[0];
    let id_str = side
        .strip_prefix("release_candidate:")
        .expect("side-effect prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    let id = ReleaseCandidateId::from_uuid(uuid::Uuid::parse_str(id_str).expect("valid uuid"));
    let candidate = store
        .get_release_candidate(id)
        .await
        .expect("release candidate round-trips through the store");
    (candidate, result.side_effects.clone())
}

// -----------------------------------------------------------------------------
// Tests.
// -----------------------------------------------------------------------------

/// The snapshot means "this candidate, and nothing else". 0A §42
/// makes `source` the field that gives the record its identity, and
/// §101 step 27 says the final validation is bound to C4 — which is
/// only meaningful if the release candidate names C4.
#[tokio::test]
async fn the_snapshot_is_bound_to_the_active_candidate() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    capture(&svc, job_id, "1.0.0").await;

    let (release, _) = create_release_candidate(&svc, &store, job_id).await;
    let projection = store.get_projection(job_id).await.expect("projection");
    let active = projection
        .active_candidate
        .expect("capture left a candidate active");
    let active = store
        .get_source_candidate(active)
        .await
        .expect("active candidate");

    assert_eq!(release.job_id, job_id);
    assert_eq!(
        release.source,
        *active.fingerprint(),
        "the snapshot must name the candidate that was active when it was taken"
    );
    assert_eq!(
        release.policy_baseline.distribution,
        package().distribution,
        "the baseline names the distribution the job was raised against"
    );
}

/// §101 steps 23 and 24: by the time the snapshot is taken, the
/// gates it lists and the obligations it lists are the ones that
/// actually passed. If the list were empty the snapshot would be a
/// record of nothing, and §42's "everything required to release"
/// would not be true of it.
#[tokio::test]
async fn it_lists_this_candidates_gates_and_obligations() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    capture(&svc, job_id, "1.0.0").await;

    let (release, side_effects) = create_release_candidate(&svc, &store, job_id).await;

    let expected_gates: Vec<GateId> = store.list_gates_for_job(job_id).await.expect("gates");
    let expected_obligations: Vec<ObligationId> = store
        .list_obligations_for_job(job_id)
        .await
        .expect("obligations");

    let mut listed_gates = release.gate_ids.clone();
    let mut want_gates = expected_gates;
    listed_gates.sort();
    want_gates.sort();
    assert_eq!(
        listed_gates, want_gates,
        "the snapshot must list every gate the active candidate has"
    );

    let mut listed_obligations = release.obligation_ids.clone();
    let mut want_obligations = expected_obligations;
    listed_obligations.sort();
    want_obligations.sort();
    assert_eq!(
        listed_obligations, want_obligations,
        "the snapshot must list every obligation the active candidate has"
    );

    assert!(
        side_effects[0].contains("2 gate(s)") && side_effects[0].contains("1 obligation(s)"),
        "the caller is told what the snapshot covers, got {:?}",
        side_effects[0]
    );
}

/// The binding rule §30 puts on evidence, applied to the snapshot.
/// A superseded candidate's gates are still in the store — they are
/// append-only history — and a snapshot that swept them up would
/// claim a release was validated against evidence for code that is
/// no longer the candidate.
#[tokio::test]
async fn a_superseded_candidates_gates_do_not_leak_into_the_snapshot() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    capture(&svc, job_id, "1.0.0").await;
    let (first, _) = create_release_candidate(&svc, &store, job_id).await;

    capture(&svc, job_id, "1.0.1").await;
    let (second, _) = create_release_candidate(&svc, &store, job_id).await;

    assert_ne!(
        first.source, second.source,
        "the second capture must be a different fingerprint for this test to mean anything"
    );
    assert_ne!(
        first.gate_ids, second.gate_ids,
        "each materialisation mints fresh gate ids, so the two snapshots must differ"
    );

    let first_gates: std::collections::HashSet<GateId> = first.gate_ids.iter().copied().collect();
    for gate_id in &second.gate_ids {
        assert!(
            !first_gates.contains(gate_id),
            "gate {gate_id} belongs to the superseded candidate and must not be re-listed"
        );
    }

    // And the store really does still hold the old ones — the
    // filter above is doing work, not passing because there was
    // nothing to filter.
    let all: Vec<GateId> = store.list_gates_for_job(job_id).await.expect("gates");
    assert_eq!(
        all.len(),
        4,
        "two candidates x two planned gates must all still be present; got {all:?}"
    );
}

/// A snapshot bound to nothing is not a snapshot. 0A §42 gives
/// `ReleaseCandidate::source` the job of identifying what is being
/// released, and there is no sensible default for it.
#[tokio::test]
async fn a_job_with_no_active_candidate_is_refused() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;

    let err = svc
        .handle_command(RuntimeCommand::CreateReleaseCandidate { job_id })
        .await
        .expect_err("nothing to release");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
    assert!(
        err.to_string().contains("no active candidate"),
        "the refusal must say what is missing: {err}"
    );
}

/// 0A §42: "Release candidate creation itself does not mean
/// releasable. The state engine determines whether it may become
/// `ReadyForApproval`." This is that sentence, executable — and it
/// is the shape §101 step 29 asserts at scenario scale.
#[tokio::test]
async fn creating_one_advances_nothing_and_authorizes_nothing() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    capture(&svc, job_id, "1.0.0").await;

    let before = store.get_projection(job_id).await.expect("projection");
    let events_before = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("events")
        .len();

    create_release_candidate(&svc, &store, job_id).await;

    let after = store.get_projection(job_id).await.expect("projection");
    assert_eq!(after.state, before.state, "no state advance");
    assert_eq!(after.version, before.version, "no version bump");

    let events_after = store
        .list_events_for_job(job_id, 1, None)
        .await
        .expect("events")
        .len();
    assert_eq!(
        events_after, events_before,
        "a read-model assembly appends no events — there is no state change to record"
    );

    let operations = store
        .list_operations_for_job(job_id)
        .await
        .expect("operations");
    assert!(
        operations.is_empty(),
        "assembling a release candidate must not propose a privileged side effect; got {operations:?}"
    );
}

/// The tenth instance of the pattern this phase keeps finding, and
/// the first one about a *write* rather than a missing one.
///
/// `CreateReleaseCandidate` minted a fresh `ReleaseCandidateId` on
/// every call, so two calls for one job produced two snapshots with
/// identical content and no way to tell which one a release is. It
/// is a record of what was validated, and a record that can be
/// written twice is not a record.
///
/// Idempotent on (job, fingerprint), which is sound because a
/// snapshot's content is derived entirely from the gates and
/// obligations *of that fingerprint*, and both are fixed at capture:
/// re-running a check writes evidence and a verdict, never a new gate
/// or obligation.
#[tokio::test]
async fn assembling_the_snapshot_twice_reuses_it() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(Arc::clone(&store));
    let job_id = create_job(&svc).await;
    capture(&svc, job_id, "1.0.0").await;

    let (first, first_side) = create_release_candidate(&svc, &store, job_id).await;
    let (second, second_side) = create_release_candidate(&svc, &store, job_id).await;
    assert_eq!(
        first, second,
        "a second assembly for the same candidate must return the snapshot that \
         already exists, not a byte-identical twin under a new id"
    );
    assert_eq!(
        store
            .list_release_candidates_for_job(job_id)
            .await
            .expect("list snapshots"),
        vec![first.clone()],
        "exactly one snapshot exists for the job"
    );
    assert!(
        first_side[0].contains("assembled") && second_side[0].contains("already assembled"),
        "and the second call says so rather than reporting a fresh \
         assembly: {first_side:?} then {second_side:?}"
    );

    // A new candidate is a different fingerprint and therefore a
    // different subject, so it gets its own snapshot — the
    // idempotency is not a blanket "one per job".
    capture(&svc, job_id, "1.0.1").await;
    let (third, _) = create_release_candidate(&svc, &store, job_id).await;
    assert_ne!(
        first.id, third.id,
        "a superseded candidate's snapshot must not be handed back for its successor"
    );
    assert_eq!(
        store
            .list_release_candidates_for_job(job_id)
            .await
            .expect("list snapshots")
            .len(),
        2,
        "both subjects keep a snapshot, and a re-assembly of either adds \
         nothing"
    );
}

/// §101 step 26 needs the snapshot *readable* by a caller holding
/// only a `JobId` — the id is minted by the command, so a caller that
/// arrived first cannot know it. Both failure modes are reported
/// distinctly, because they have different next moves.
#[tokio::test]
async fn the_snapshot_is_readable_by_job_and_its_absence_is_named() {
    let store = Arc::new(MockStore::new());
    let svc = build_service(Arc::clone(&store));
    let job_id = create_job(&svc).await;

    let before_capture = svc
        .handle_query(RuntimeQuery::GetReleaseCandidate { job_id })
        .await
        .expect_err("no active candidate, so no snapshot");
    assert_eq!(
        before_capture.kind,
        RuntimeErrorKind::InvalidInput,
        "and the failure is typed, not a store error"
    );
    assert!(
        before_capture.message.contains("no active candidate"),
        "the message says which of the two reasons it is: {:?}",
        before_capture.message
    );

    capture(&svc, job_id, "1.0.0").await;

    // Active candidate, no snapshot yet: a *different* fact from the
    // one above, and the one a caller at the exit checkpoint hits.
    let not_yet = svc
        .handle_query(RuntimeQuery::GetReleaseCandidate { job_id })
        .await
        .expect_err("no snapshot has been assembled");
    assert!(
        not_yet
            .message
            .contains("has no release candidate for its active candidate"),
        "got {:?}",
        not_yet.message
    );
    assert!(
        not_yet.message.contains(&job_id.to_string()),
        "the message names the candidate it would be for, so a caller can \
         tell which subject is outstanding: {:?}",
        not_yet.message
    );

    let (expected, _) = create_release_candidate(&svc, &store, job_id).await;
    let QueryResult::ReleaseCandidate(release) = svc
        .handle_query(RuntimeQuery::GetReleaseCandidate { job_id })
        .await
        .expect("the snapshot is readable once it exists")
    else {
        panic!("GetReleaseCandidate returned the wrong variant");
    };
    assert_eq!(
        release, expected,
        "the query finds the one the command wrote"
    );

    // And the same query about a different job says so rather than
    // returning this one: the selection is by job, not by "whichever
    // snapshot was written last".
    let other_job = create_job(&svc).await;
    let other = svc
        .handle_query(RuntimeQuery::GetReleaseCandidate { job_id: other_job })
        .await
        .expect_err("the other job has no active candidate");
    assert!(
        other.message.contains(&other_job.to_string()),
        "the error names the job asked about, not the one that has a \
         snapshot: {:?}",
        other.message
    );
}
