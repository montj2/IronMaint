//! PHASE-1.md §31 — `evidence.list`, `evidence.get`,
//! `evidence.artifact.read`.
//!
//! These are the three read-only MCP tools 1A.4 adds so the
//! §31 exit-checkpoint ("have an MCP client read the report")
//! has a real surface. The runtime layer's `RuntimeQuery`
//! gets three new variants that the MCP dispatcher routes
//! to. The teeth-checks are:
//!
//!   list_evidence_returns_rows_attached_to_the_job
//!     Two evidence rows on the job. The runtime's
//!     `ListEvidence` returns both, in the order the store
//!     gives them.
//!
//!   list_evidence_can_narrow_to_one_candidate
//!     Three rows: two bound to candidate A, one to
//!     candidate B. With a `candidate_fingerprint`
//!     argument, the runtime returns the two A rows and
//!     not the B row.
//!
//!   get_evidence_returns_one_row
//!     A single evidence row is fetched by id; mismatched
//!     ids are a typed error.
//!
//!   read_evidence_artifact_returns_bytes
//!     An evidence row with one `Report` artifact, the
//!     artifact's bytes on disk in the runtime's
//!     configured artifact store. The runtime round-trips
//!     the digest, the media type, and the bytes verbatim.
//!
//!   read_evidence_artifact_with_wrong_artifact_id_errors
//!     A row with one Report; the caller names a
//!     different `ArtifactId`. The runtime refuses: the
//!     MCP tool exposes the bytes of an artifact the
//!     evidence row actually references, not an arbitrary
//!     digest.
//!
//!   read_evidence_artifact_without_artifact_store_errors
//!     The runtime has no `ArtifactStore` configured. The
//!     runtime refuses, rather than returning empty bytes
//!     (a `null` payload would be a different,
//!     silently-broken contract).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_artifacts::{ArtifactRoot, ArtifactStore};
use ironmaint_core::{
    ArtifactId, CandidateFingerprint, Digest, DigestAlgorithm, EvidenceId, JobId,
};
use ironmaint_evidence::{
    ArtifactKind, ArtifactRef, Evidence, EvidenceKind, EvidenceProducer, EvidenceScope,
    EvidenceStatus,
};
use ironmaint_executor::{ExecutionRequest, Executor, ExecutorError, ToolRegistry};
use ironmaint_runtime::{Clock, FixedClock, QueryResult, RuntimeQuery, RuntimeService};
use ironmaint_store::mock::MockStore;
use ironmaint_store::EvidenceStore;
use time::OffsetDateTime;

fn fp(hex_byte: u8) -> CandidateFingerprint {
    CandidateFingerprint::from_hex(format!("{hex_byte:064x}")).unwrap()
}

fn make_evidence(
    fingerprint: &CandidateFingerprint,
    producer_name: &str,
    status: EvidenceStatus,
) -> Evidence {
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let producer = EvidenceProducer::new(producer_name.to_string());
    Evidence::new(
        fingerprint.clone(),
        EvidenceKind::SourcePreparation,
        status,
        producer,
        EvidenceScope::Candidate(fingerprint.clone()),
        now,
    )
}

fn make_evidence_with_artifact(
    fingerprint: &CandidateFingerprint,
    producer_name: &str,
    status: EvidenceStatus,
    artifact_ref: ArtifactRef,
) -> Evidence {
    let mut ev = make_evidence(fingerprint, producer_name, status);
    ev = ev.with_artifact(artifact_ref);
    ev
}

async fn seed_evidence_rows(
    store: &MockStore,
    job_id: JobId,
    rows: &[Evidence],
) {
    for ev in rows {
        store.put_evidence(ev, job_id).await.expect("put_evidence");
    }
}

fn runtime_service(
    store: Arc<MockStore>,
    artifacts: Option<Arc<ArtifactStore>>,
) -> Arc<RuntimeService<MockStore, NeverExecutor>> {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
    ));
    let executor = Arc::new(NeverExecutor);
    let registry = Arc::new(ToolRegistry::new());
    let mut svc = RuntimeService::new(store, clock, executor, registry);
    if let Some(a) = artifacts {
        svc = svc.with_artifact_store(a);
    }
    Arc::new(svc)
}

/// `NeverExecutor` is the minimal executor that lets the
/// runtime build. It is never invoked in this test file —
/// `ListEvidence` / `GetEvidence` / `ReadEvidenceArtifact`
/// are pure store/artifact reads, no subprocess. The error
/// type and the implementation are deliberately the loudest
/// failure the executor can produce, so an accidental call
/// surfaces immediately rather than passing silently.
struct NeverExecutor;
impl Executor for NeverExecutor {
    async fn execute(
        &self,
        _request: ExecutionRequest,
    ) -> Result<ironmaint_executor::ExecutionRecord, ExecutorError> {
        Err(ExecutorError::new(
            ironmaint_executor::ExecutorErrorKind::InfrastructureFailed,
            "NeverExecutor must not be invoked in evidence_queries",
        ))
    }
}

#[tokio::test]
async fn list_evidence_returns_rows_attached_to_the_job() {
    // Two evidence rows on one job, two different
    // candidates. The runtime's `ListEvidence { job_id,
    // candidate_fingerprint: None }` returns both.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let job_id = JobId::new();

    let fp_a = fp(0xA1);
    let fp_b = fp(0xB2);
    let row_a = make_evidence(&fp_a, "sbuild", EvidenceStatus::Pass);
    let row_b = make_evidence(&fp_b, "lintian", EvidenceStatus::Fail);
    seed_evidence_rows(&*store, job_id, &[row_a.clone(), row_b.clone()]).await;

    let svc = runtime_service(store.clone(), None);

    let result = svc
        .handle_query(RuntimeQuery::ListEvidence {
            job_id,
            candidate_fingerprint: None,
        })
        .await
        .expect("ListEvidence");
    let QueryResult::EvidenceList(rows) = result else {
        panic!("ListEvidence must return EvidenceList, got {result:?}");
    };
    assert_eq!(rows.len(), 2, "both rows must come back; got {rows:?}");
    let ids: std::collections::BTreeSet<_> = rows.iter().map(|e| e.id).collect();
    assert!(ids.contains(&row_a.id));
    assert!(ids.contains(&row_b.id));
}

#[tokio::test]
async fn list_evidence_can_narrow_to_one_candidate() {
    // Three rows: two bound to candidate A, one to
    // candidate B. With a `candidate_fingerprint`
    // argument, the runtime returns only the two A rows.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let job_id = JobId::new();

    let fp_a = fp(0xA1);
    let fp_b = fp(0xB2);
    let a1 = make_evidence(&fp_a, "sbuild", EvidenceStatus::Pass);
    let a2 = make_evidence(&fp_a, "lintian", EvidenceStatus::Fail);
    let b1 = make_evidence(&fp_b, "rpmlint", EvidenceStatus::Pass);
    seed_evidence_rows(&*store, job_id, &[a1.clone(), a2.clone(), b1.clone()]).await;

    let svc = runtime_service(store.clone(), None);

    let result = svc
        .handle_query(RuntimeQuery::ListEvidence {
            job_id,
            candidate_fingerprint: Some(fp_a.clone()),
        })
        .await
        .expect("ListEvidence narrowed");
    let QueryResult::EvidenceList(rows) = result else {
        panic!("narrowed ListEvidence must return EvidenceList, got {result:?}");
    };
    assert_eq!(rows.len(), 2, "only the A rows must come back; got {rows:?}");
    let ids: std::collections::BTreeSet<_> = rows.iter().map(|e| e.id).collect();
    assert!(ids.contains(&a1.id));
    assert!(ids.contains(&a2.id));
    assert!(
        !ids.contains(&b1.id),
        "B rows must not leak when the caller narrows to A"
    );
    for r in &rows {
        assert_eq!(r.candidate, fp_a);
    }
}

#[tokio::test]
async fn get_evidence_returns_one_row() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let job_id = JobId::new();
    let fp_a = fp(0xA1);
    let row = make_evidence(&fp_a, "sbuild", EvidenceStatus::Pass);
    seed_evidence_rows(&*store, job_id, &[row.clone()]).await;

    let svc = runtime_service(store.clone(), None);

    let result = svc
        .handle_query(RuntimeQuery::GetEvidence {
            evidence_id: row.id,
        })
        .await
        .expect("GetEvidence");
    let QueryResult::Evidence(got) = result else {
        panic!("GetEvidence must return Evidence, got {result:?}");
    };
    assert_eq!(got, row);

    // Unknown id: a typed error. The MCP layer maps
    // `InvalidInput` to `McpError::InvalidInput` so a
    // client can tell "no such row" from a runtime crash.
    let unknown = EvidenceId::new();
    let err = svc
        .handle_query(RuntimeQuery::GetEvidence { evidence_id: unknown })
        .await
        .expect_err("unknown id must error");
    assert_eq!(err.kind, ironmaint_runtime::RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn read_evidence_artifact_returns_bytes() {
    // An evidence row with one `Report` artifact whose
    // bytes have been written to the runtime's artifact
    // store. The runtime round-trips the digest, media
    // type, and bytes.
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifact_store = Arc::new(ArtifactStore::open(ArtifactRoot::new(
        tmp.path().to_path_buf(),
    )));
    let payload = br#"{"status":"pass","observations":[]}"#;
    let rec = artifact_store.put_bytes(payload).await.expect("put_bytes");
    let digest = Digest::new(DigestAlgorithm::Sha256, rec.digest.as_str()).unwrap();
    let artifact_id = ArtifactId::new();
    let artifact_ref = ArtifactRef::new(artifact_id, ArtifactKind::Report, digest.clone())
        .with_size_bytes(rec.size)
        .with_media_type("application/json");

    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let job_id = JobId::new();
    let fp_a = fp(0xA1);
    let row = make_evidence_with_artifact(
        &fp_a,
        "sbuild",
        EvidenceStatus::Pass,
        artifact_ref,
    );
    seed_evidence_rows(&*store, job_id, &[row.clone()]).await;

    let svc = runtime_service(store.clone(), Some(artifact_store.clone()));

    let result = svc
        .handle_query(RuntimeQuery::ReadEvidenceArtifact {
            evidence_id: row.id,
            artifact_id,
        })
        .await
        .expect("ReadEvidenceArtifact");
    let QueryResult::EvidenceArtifact(art) = result else {
        panic!("ReadEvidenceArtifact must return EvidenceArtifact, got {result:?}");
    };
    assert_eq!(art.digest, digest, "digest must round-trip");
    assert_eq!(art.media_type.as_deref(), Some("application/json"));
    assert_eq!(art.bytes, payload);
}

#[tokio::test]
async fn read_evidence_artifact_with_wrong_artifact_id_errors() {
    // The row references one Report. The caller names a
    // different `ArtifactId`. The runtime refuses: the
    // bytes are the contract, and the tool exposes *that
    // evidence row's* bytes, not an arbitrary digest.
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifact_store = Arc::new(ArtifactStore::open(ArtifactRoot::new(
        tmp.path().to_path_buf(),
    )));
    let payload = br#"{"status":"pass"}"#;
    let rec = artifact_store.put_bytes(payload).await.expect("put_bytes");
    let digest = Digest::new(DigestAlgorithm::Sha256, rec.digest.as_str()).unwrap();
    let bound_id = ArtifactId::new();
    let bound_ref = ArtifactRef::new(bound_id, ArtifactKind::Report, digest);
    let wrong_id = ArtifactId::new();

    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let job_id = JobId::new();
    let fp_a = fp(0xA1);
    let row = make_evidence_with_artifact(
        &fp_a,
        "sbuild",
        EvidenceStatus::Pass,
        bound_ref,
    );
    seed_evidence_rows(&*store, job_id, &[row.clone()]).await;

    let svc = runtime_service(store.clone(), Some(artifact_store));

    let err = svc
        .handle_query(RuntimeQuery::ReadEvidenceArtifact {
            evidence_id: row.id,
            artifact_id: wrong_id,
        })
        .await
        .expect_err("wrong artifact id must error");
    assert_eq!(err.kind, ironmaint_runtime::RuntimeErrorKind::InvalidInput);
}

#[tokio::test]
async fn read_evidence_artifact_without_artifact_store_errors() {
    // The runtime has no artifact store configured
    // (None). The runtime refuses, rather than returning
    // empty bytes.
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let job_id = JobId::new();
    let fp_a = fp(0xA1);
    let digest = Digest::new(DigestAlgorithm::Sha256, &"0".repeat(64)).unwrap();
    let artifact_id = ArtifactId::new();
    let artifact_ref = ArtifactRef::new(artifact_id, ArtifactKind::Report, digest);
    let row = make_evidence_with_artifact(
        &fp_a,
        "sbuild",
        EvidenceStatus::Pass,
        artifact_ref,
    );
    seed_evidence_rows(&*store, job_id, &[row.clone()]).await;

    let svc = runtime_service(store.clone(), None);

    let err = svc
        .handle_query(RuntimeQuery::ReadEvidenceArtifact {
            evidence_id: row.id,
            artifact_id,
        })
        .await
        .expect_err("missing artifact store must error");
    assert_eq!(err.kind, ironmaint_runtime::RuntimeErrorKind::InvalidInput);
}
