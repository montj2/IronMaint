//! Phase 0B.5 C2 — `RuntimeCommand::MaterializeChecks` integration
//! tests (PHASE-0B.md §44, §45).
//!
//! Each test exercises one observable contract:
//!   1. Materialisation creates one `CheckDefinition` per
//!      `PlannedCheck`.
//!   2. Distinct `PlannedCheck`s mint distinct `CheckId`s
//!      (no collisions).
//!   3. `evidence_kind` → `gate_stage` mapping matches
//!      `gate_stage_for` exactly.
//!   4. Persisted `CheckDefinition`s round-trip via
//!      `CheckStore::get_check`.
//!   5. Per-job index returns the new check ids.
//!
//! `unwrap`/`expect` are allowed here because failure in a test
//! should panic; the production crate forbids them via the
//! workspace lint table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease, JobId,
    PackageIdentity, PackageName,
};
use ironmaint_evidence::{EvidenceKind, GateStage};
use ironmaint_executor::{NullExecutor, ToolRegistry};
use ironmaint_runtime::{
    Clock, FixedClock, OrchestratorRef, RuntimeCommand, RuntimeService, gate_stage_for,
};
use ironmaint_store::mock::MockStore;
use ironmaint_store::{CheckStore, GateStore};
use time::OffsetDateTime;

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

fn build_service(store: Arc<MockStore>) -> RuntimeService<MockStore, NullExecutor> {
    RuntimeService::new(
        store,
        fixed_clock(),
        Arc::new(NullExecutor),
        Arc::new(ToolRegistry::new()),
    )
}

fn fingerprint_zero() -> CandidateFingerprint {
    CandidateFingerprint::from_hex("00".repeat(32)).unwrap()
}

async fn create_job(svc: &RuntimeService<MockStore, NullExecutor>) -> JobId {
    let result = svc
        .handle_command(RuntimeCommand::CreateJob {
            orchestrator: OrchestratorRef::ironclaw(),
            package: package(),
        })
        .await
        .expect("create");
    let side_effect = &result.side_effects[0];
    let rest = side_effect
        .strip_prefix("job:")
        .expect("side-effect prefix")
        .split_whitespace()
        .next()
        .expect("uuid");
    JobId::from_uuid(uuid::Uuid::parse_str(rest).expect("valid uuid"))
}

#[tokio::test]
async fn materialize_checks_creates_one_definition_per_planned_check() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    let candidate = fingerprint_zero();

    let planned = vec![
        (
            ToolCapabilityKey::new("debian.build.sbuild").unwrap(),
            EvidenceKind::Build,
            true,
        ),
        (
            ToolCapabilityKey::new("debian.qa.lintian").unwrap(),
            EvidenceKind::PackageQa,
            true,
        ),
        (
            ToolCapabilityKey::new("debian.qa.piuparts").unwrap(),
            EvidenceKind::PackageQa,
            false,
        ),
        (
            ToolCapabilityKey::new("debian.test.autopkgtest").unwrap(),
            EvidenceKind::FunctionalTest,
            true,
        ),
    ];

    let result = svc
        .handle_command(RuntimeCommand::MaterializeChecks {
            job_id,
            candidate: candidate.clone(),
            planned: planned.clone(),
        })
        .await
        .expect("materialize");

    assert!(
        result.side_effects[0].contains("4 check(s)"),
        "side effect must report 4 checks, got: {}",
        result.side_effects[0]
    );

    let ids = store
        .list_checks_for_job(job_id)
        .await
        .expect("list checks");
    assert_eq!(ids.len(), 4);
}

#[tokio::test]
async fn materialize_checks_assigns_distinct_check_ids() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    let candidate = fingerprint_zero();

    let planned = vec![
        (
            ToolCapabilityKey::new("debian.build.sbuild").unwrap(),
            EvidenceKind::Build,
            true,
        ),
        (
            ToolCapabilityKey::new("debian.qa.lintian").unwrap(),
            EvidenceKind::PackageQa,
            true,
        ),
    ];

    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate,
        planned,
    })
    .await
    .expect("materialize");

    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        ids.len(),
        "CheckIds must be distinct — no UUIDv7 collisions"
    );
}

#[tokio::test]
async fn materialize_checks_maps_evidence_kind_to_gate_stage() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    let candidate = fingerprint_zero();

    let planned = vec![(
        ToolCapabilityKey::new("debian.build.sbuild").unwrap(),
        EvidenceKind::Build,
        true,
    )];

    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate,
        planned,
    })
    .await
    .expect("materialize");

    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let check = store.get_check(ids[0]).await.expect("get check");
    assert_eq!(check.gate_stage, GateStage::BuildValidation);
    assert_eq!(check.evidence_kind, EvidenceKind::Build);
    assert!(check.mandatory);
}

#[test]
fn gate_stage_for_maps_all_kinds() {
    // Table-driven mapping — pins down the canonical lookup.
    assert_eq!(
        gate_stage_for(&EvidenceKind::SourceIntegrity),
        Some(GateStage::SourceAnalysis)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::Build),
        Some(GateStage::BuildValidation)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::PackageQa),
        Some(GateStage::PackageQa)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::FunctionalTest),
        Some(GateStage::FunctionalValidation)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::UpgradeTest),
        Some(GateStage::UpgradeValidation)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::Reproducibility),
        Some(GateStage::FinalValidation)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::PolicyEvaluation),
        Some(GateStage::PolicyEvaluation)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::LicenseReview),
        Some(GateStage::FinalValidation)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::IssueCorrelation),
        Some(GateStage::IssueAnalysis)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::ReleaseAssembly),
        Some(GateStage::CandidateAssembly)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::PublicationValidation),
        Some(GateStage::Publication)
    );
    assert_eq!(
        gate_stage_for(&EvidenceKind::Other("tool_output".into())),
        None
    );
}

#[tokio::test]
async fn materialize_checks_round_trips_through_store() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    let candidate = fingerprint_zero();

    let planned = vec![(
        ToolCapabilityKey::new("debian.qa.lintian").unwrap(),
        EvidenceKind::PackageQa,
        true,
    )];

    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate,
        planned,
    })
    .await
    .expect("materialize");

    let ids = store.list_checks_for_job(job_id).await.expect("list");
    let id = ids[0];
    let check = store.get_check(id).await.expect("get check");
    assert_eq!(check.id, id);
    assert_eq!(check.job_id, job_id);
    assert_eq!(
        check.capability,
        ToolCapabilityKey::new("debian.qa.lintian").unwrap()
    );
    assert_eq!(check.gate_stage, GateStage::PackageQa);
}

#[tokio::test]
async fn materialize_checks_persists_per_job_index() {
    let store: Arc<MockStore> = Arc::new(MockStore::new());
    let svc = build_service(store.clone());
    let job_id = create_job(&svc).await;
    let candidate = fingerprint_zero();

    let planned = vec![
        (
            ToolCapabilityKey::new("debian.build.sbuild").unwrap(),
            EvidenceKind::Build,
            true,
        ),
        (
            ToolCapabilityKey::new("debian.qa.lintian").unwrap(),
            EvidenceKind::PackageQa,
            true,
        ),
    ];

    svc.handle_command(RuntimeCommand::MaterializeChecks {
        job_id,
        candidate,
        planned,
    })
    .await
    .expect("materialize");

    let ids = store.list_checks_for_job(job_id).await.expect("list");
    assert_eq!(ids.len(), 2);

    // The corresponding `GateDefinition`s must also be persisted.
    let gate_ids = store.list_gates_for_job(job_id).await.expect("list gates");
    assert_eq!(gate_ids.len(), 2, "one GateDefinition per CheckDefinition");
}
