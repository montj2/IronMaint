//! End-to-end tests for the two `RuntimeService::handle_command`
//! handlers wired in 0B.17: `CreateJob` and `SetActiveCandidate`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease,
    GitHashAlgorithm, GitObjectId, JobId, JobState, PackageIdentity, PackageName, PackageRevision,
    PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use ironmaint_runtime::{
    Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeErrorKind, RuntimeQuery,
    RuntimeService,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CandidateStore, ProjectionStore};
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

/// Seed a SourceCandidate against the given store and return its
/// `(candidate_id, fingerprint)` pair.
async fn seed_candidate(
    store: &MockStore,
    job_id: JobId,
) -> (ironmaint_core::CandidateId, CandidateFingerprint) {
    let url = Url::parse("https://example.invalid/foo.git").unwrap();
    let repository = RepositoryRef::new(VcsKind::Git, url).unwrap();
    let commit = GitObjectId::new(GitHashAlgorithm::Sha1, "1".repeat(40)).unwrap();
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

#[tokio::test]
async fn create_job_persists_projection_at_version_zero() {
    let store = Arc::new(MockStore::new());
    let svc = RuntimeService::new(store.clone(), fixed_clock());

    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create job");

    // First write of a fresh projection lands at version 0 and
    // sequence 1 (the seed Domain event).
    assert_eq!(result.new_version, 0);
    assert_eq!(result.new_sequence, 1);
    assert_eq!(result.side_effects.len(), 1);
    assert!(result.side_effects[0].starts_with("job:"));
}

#[tokio::test]
async fn create_job_then_query_projection_round_trips() {
    let store = Arc::new(MockStore::new());
    let svc = RuntimeService::new(store.clone(), fixed_clock());

    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create job");

    let job_id = parse_job_id(&result.side_effects[0]);

    let query_result = svc
        .handle_query(RuntimeQuery::GetProjection { job_id })
        .await
        .expect("get projection");
    let projection_json = match query_result {
        ironmaint_runtime::service::QueryResult::Projection(v) => v,
        _ => unreachable!("expected Projection variant"),
    };
    let state = projection_json
        .get("state")
        .and_then(|v| v.as_str())
        .expect("state field");
    assert_eq!(state, "event_detected");
}

#[tokio::test]
async fn set_active_candidate_attaches_fingerprint_to_projection() {
    let store = Arc::new(MockStore::new());
    let svc = RuntimeService::new(store.clone(), fixed_clock());

    // 1. Create the job (version 0).
    let create = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let job_id = parse_job_id(&create.side_effects[0]);

    // 2. Seed a SourceCandidate and capture its id and fingerprint.
    let (candidate_id, fingerprint) = seed_candidate(&store, job_id).await;

    // 3. Activate the candidate via SetActiveCandidate.
    let set = svc
        .handle_command(RuntimeCommand::SetActiveCandidate {
            job_id,
            fingerprint,
        })
        .await
        .expect("set active");

    assert_eq!(set.new_version, 1);
    assert_eq!(set.new_sequence, 2);
    assert!(set.side_effects.is_empty());

    // 4. Projection now has the candidate id set.
    let projection = store.get_projection(job_id).await.expect("get projection");
    assert_eq!(projection.active_candidate, Some(candidate_id));
    assert_eq!(projection.version, 1);
}

#[tokio::test]
async fn set_active_candidate_rejects_unknown_fingerprint() {
    let store = Arc::new(MockStore::new());
    let svc = RuntimeService::new(store.clone(), fixed_clock());

    let create = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let job_id = parse_job_id(&create.side_effects[0]);

    let bogus_fp = CandidateFingerprint::from_hex("a".repeat(64)).unwrap();
    let err = svc
        .handle_command(RuntimeCommand::SetActiveCandidate {
            job_id,
            fingerprint: bogus_fp,
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn set_active_candidate_rejects_after_build() {
    let store = Arc::new(MockStore::new());
    let svc = RuntimeService::new(store.clone(), fixed_clock());

    let create = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let job_id = parse_job_id(&create.side_effects[0]);

    let (_candidate_id, fingerprint) = seed_candidate(&store, job_id).await;

    // Force the projection past the candidate-capture window.
    let mut projection = store.get_projection(job_id).await.expect("get");
    projection.state = JobState::SourceRevision;
    let v = projection.version;
    store.put_projection(&projection, v).await.expect("put");

    let err = svc
        .handle_command(RuntimeCommand::SetActiveCandidate {
            job_id,
            fingerprint,
        })
        .await
        .expect_err("must reject");
    assert_eq!(err.kind, RuntimeErrorKind::InvalidInput);
}

/// Extract the `JobId` from a side-effect string of the form
/// `job:<uuid> created by orchestrator=<kind>`.
fn parse_job_id(side_effect: &str) -> JobId {
    let rest = side_effect
        .strip_prefix("job:")
        .expect("side-effect should start with job:");
    let id_str = rest
        .split_whitespace()
        .next()
        .expect("side-effect should contain a uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(id_str).expect("valid uuid"))
}
