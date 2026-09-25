//! §74 acceptance: cross-crate round-trip suite for the spec-listed
//! snapshot fixtures (PHASE-0A.md §74).
//!
//! 1. Each fixture lives at `tests/snapshots/<name>.json` and is a
//!    canonical value for one public wire type.
//! 2. The test reads the JSON, parses it into the wire type, asserts
//!    that serialize → parse → serialize is stable (round-trip), and
//!    asserts an `insta::assert_json_snapshot!` of the parsed value
//!    against the committed `*.snap` file.
//!
//! Six fixtures: `MaintenanceJob` (Debian), `MaintenanceJob` (Fedora),
//! `Evidence`, `Obligation`, `PublicationPlan`, `event_envelope` (a
//! `JobEvent::Transitioned` payload).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use ironmaint_core::{
    AuthorityId, CandidateFingerprint, Digest, DigestAlgorithm, DistributionFamily,
    DistributionRef, DistributionRelease, JobId, JobProjection, JobState, MaintenanceEventId,
    MaintenanceJob, PackageIdentity, PackageName, ReleaseCandidateId,
};
use ironmaint_evidence::{
    ArtifactKind, ArtifactRef, Evidence, EvidenceKind, EvidenceProducer, EvidenceScope,
    EvidenceStatus,
};
use ironmaint_policy::{
    Applicability, Obligation, ObligationStatus, ObligationStrength, PolicyReference,
    PrivilegedOperation, PrivilegedOperationKind, PublicationPlan,
};
use ironmaint_state::{JobEvent, StateTransitioned, Transition};
use serde::Serialize;
use time::macros::datetime;

const SNAPSHOTS_DIR: &str = "tests/snapshots";

// =============================================================================
// Canonical wire values used by the round-trip tests. Each function
// constructs a hand-built canonical value (via the public API surface)
// so that the resulting JSON is predictable and reviewable.
// =============================================================================

fn family(s: &str) -> DistributionFamily {
    DistributionFamily::new(s).unwrap()
}
fn release(s: &str) -> DistributionRelease {
    DistributionRelease::new(s).unwrap()
}
fn dref(family_str: &str, release_str: &str) -> DistributionRef {
    DistributionRef::new(family(family_str), release(release_str))
}
fn pkg_name(s: &str) -> PackageName {
    PackageName::new(s).unwrap()
}
fn package_identity(dref: DistributionRef, name: &str) -> PackageIdentity {
    PackageIdentity::new(dref, pkg_name(name))
}
fn fingerprint() -> CandidateFingerprint {
    CandidateFingerprint::from_hex("a".repeat(64).as_str()).unwrap()
}

fn maintenance_job_debian() -> MaintenanceJob {
    MaintenanceJob::new(
        JobId::new(),
        package_identity(dref("debian", "sid"), "rust-core"),
        MaintenanceEventId::new(),
        datetime!(2026-01-15 09:00:00 UTC),
    )
}

fn maintenance_job_fedora() -> MaintenanceJob {
    MaintenanceJob::new(
        JobId::new(),
        package_identity(dref("fedora", "rawhide"), "rust-core"),
        MaintenanceEventId::new(),
        datetime!(2026-01-15 09:00:00 UTC),
    )
}

fn evidence_value() -> Evidence {
    let artifact = ArtifactRef::new(
        ironmaint_core::ArtifactId::new(),
        ArtifactKind::BuildArtifact,
        Digest::new(DigestAlgorithm::Sha256, "b".repeat(64)).unwrap(),
    );
    Evidence::new(
        fingerprint(),
        EvidenceKind::Build,
        EvidenceStatus::Pass,
        EvidenceProducer::new("debian-stub").with_version("0.1.0"),
        EvidenceScope::Distribution(dref("debian", "sid")),
        datetime!(2026-01-15 09:30:00 UTC),
    )
    .with_artifact(artifact)
    .with_notes("First build attempt succeeded.")
    .unwrap()
}

fn obligation_value() -> Obligation {
    Obligation::new(
        fingerprint(),
        PolicyReference::new(AuthorityId::new()),
        ObligationStrength::Mandatory,
        Applicability::Applicable,
        "All Rust binaries must link against the secure-libs package.",
    )
    .unwrap()
    .with_status(ObligationStatus::NotEvaluated)
}

fn publication_plan_value() -> PublicationPlan {
    let plan = PublicationPlan::new(ReleaseCandidateId::new(), dref("debian", "sid"));
    let op = PrivilegedOperation::proposed(
        PrivilegedOperationKind::CanonicalRepositoryPush,
        fingerprint(),
    );
    plan.with_operation(op)
}

fn event_envelope_value() -> JobEvent {
    let job = maintenance_job_debian();
    let projection = JobProjection {
        job,
        state: JobState::Intake,
        active_candidate: None,
        version: 1,
        updated_at: datetime!(2026-01-15 09:00:00 UTC),
    };
    let t = Transition {
        from: JobState::EventDetected,
        to: JobState::Intake,
        rule_index: 0,
    };
    let ev = StateTransitioned::new(t, projection, datetime!(2026-01-15 09:00:00 UTC));
    JobEvent::Transitioned(ev)
}

// =============================================================================
// Fixture generator — run once with `IRONMAINT_FIXTURES_WRITE=1 cargo test
// -p ironmaint-testkit --test snapshots` to produce the committed JSON
// files. Subsequent runs load and verify.
// =============================================================================

fn maybe_write_fixture<T: Serialize>(filename: &str, value: &T) {
    if std::env::var_os("IRONMAINT_FIXTURES_WRITE").is_some() {
        let path = Path::new(SNAPSHOTS_DIR).join(filename);
        fs::create_dir_all(SNAPSHOTS_DIR).unwrap();
        let json = serde_json::to_string_pretty(value).expect("serialize fixture");
        fs::write(&path, format!("{json}\n")).expect("write fixture");
        eprintln!("wrote fixture: {}", path.display());
    }
}

// =============================================================================
// Round-trip tests
// =============================================================================

fn round_trip<T>(filename: &str, canonical: T)
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug + Clone,
{
    maybe_write_fixture(filename, &canonical);

    let path = Path::new(SNAPSHOTS_DIR).join(filename);
    let json = fs::read_to_string(&path).expect("snapshot fixture must be committed");
    let parsed: T = serde_json::from_str(&json).expect("snapshot must deserialize");
    let reserialized = serde_json::to_string(&parsed).expect("snapshot must serialize");
    let reparsed: T =
        serde_json::from_str(&reserialized).expect("re-serialized snapshot must parse");
    assert_eq!(parsed, reparsed, "round-trip not stable for {filename}");
    insta::assert_json_snapshot!(filename.trim_end_matches(".json"), &parsed);
}

#[test]
fn debian_maintenance_job_fixture_round_trips() {
    round_trip("maintenance_job.debian.json", maintenance_job_debian());
}

#[test]
fn fedora_maintenance_job_fixture_round_trips() {
    round_trip("maintenance_job.fedora.json", maintenance_job_fedora());
}

#[test]
fn evidence_fixture_round_trips() {
    round_trip("evidence.json", evidence_value());
}

#[test]
fn obligation_fixture_round_trips() {
    round_trip("obligation.json", obligation_value());
}

#[test]
fn publication_plan_fixture_round_trips() {
    round_trip("publication_plan.json", publication_plan_value());
}

#[test]
fn event_envelope_fixture_round_trips() {
    round_trip("event_envelope.json", event_envelope_value());
}
