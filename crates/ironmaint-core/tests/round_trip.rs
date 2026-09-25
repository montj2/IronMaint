//! Cross-type JSON round-trip integration tests.
//!
//! These exercise the wire format end-to-end: build → `serde_json::to_string`
//! → `serde_json::from_str` → assert structural equality. They guard
//! against silent changes to the JSON shape (PHASE-0A.md §59, §76).

// Integration tests are compiled as a separate crate and don't inherit
// `cfg_attr(test, allow(...))` from `lib.rs`. Allow `unwrap`/`expect`
// here for the same reason as the unit tests: the constructors under
// test are pre-validated by construction.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_core::{
    CandidateFingerprint, DistributionFamily, DistributionRef, DistributionRelease, EventSource,
    GitHashAlgorithm, GitObjectId, JobId, JobProjection, JobState, MaintenanceEvent,
    MaintenanceEventId, MaintenanceEventType, MaintenanceJob, PackageIdentity, PackageName,
    PackageRevision, PackageVersion, RepositoryRef, SourceCandidate, VcsKind,
};
use time::macros::datetime;
use url::Url;

fn fam(s: &str) -> DistributionFamily {
    DistributionFamily::new(s).unwrap()
}

fn rel(s: &str) -> DistributionRelease {
    DistributionRelease::new(s).unwrap()
}

fn name(s: &str) -> PackageName {
    PackageName::new(s).unwrap()
}

fn ver(s: &str) -> PackageVersion {
    PackageVersion::new(s).unwrap()
}

fn git_oid(hex: &str) -> GitObjectId {
    GitObjectId::new(GitHashAlgorithm::Sha1, hex).unwrap()
}

fn repo(url: &str) -> RepositoryRef {
    RepositoryRef::new(VcsKind::Git, Url::parse(url).unwrap()).unwrap()
}

fn pkg(dist: &str, release: &str, n: &str, v: &str) -> PackageRevision {
    PackageRevision::new(
        PackageIdentity::new(DistributionRef::new(fam(dist), rel(release)), name(n)),
        ver(v),
    )
}

fn pkg_identity(dist: &str, release: &str, n: &str) -> PackageIdentity {
    PackageIdentity::new(DistributionRef::new(fam(dist), rel(release)), name(n))
}

#[test]
fn distribution_ref_round_trips() {
    let dr = DistributionRef::new(fam("debian"), rel("unstable"));
    let json = serde_json::to_string(&dr).unwrap();
    assert_eq!(json, r#"{"family":"debian","release":"unstable"}"#);
    let parsed: DistributionRef = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, dr);
}

#[test]
fn package_revision_round_trips() {
    let pr = pkg("debian", "unstable", "foo", "1.9.0-1");
    let json = serde_json::to_string(&pr).unwrap();
    let parsed: PackageRevision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, pr);
    // Spot-check the wire shape so a refactor that silently changes the
    // JSON layout is caught here, not in a downstream consumer.
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["package"]["distribution"]["family"], "debian");
    assert_eq!(value["package"]["distribution"]["release"], "unstable");
    assert_eq!(value["package"]["source_name"], "foo");
    assert_eq!(value["version"], "1.9.0-1");
}

#[test]
fn source_candidate_round_trips() {
    let c = SourceCandidate::new(
        JobId::new(),
        pkg("debian", "unstable", "foo", "1.9.0-1"),
        repo("https://salsa.debian.org/foo/foo.git"),
        git_oid(&"1".repeat(40)),
        git_oid(&"2".repeat(40)),
        datetime!(2026-01-01 00:00:00 UTC),
    );
    let json = serde_json::to_string(&c).unwrap();
    let parsed: SourceCandidate = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, c);
}

#[test]
fn source_candidate_revision_round_trips() {
    let parent = SourceCandidate::new(
        JobId::new(),
        pkg("debian", "unstable", "foo", "1.9.0-1"),
        repo("https://salsa.debian.org/foo/foo.git"),
        git_oid(&"1".repeat(40)),
        git_oid(&"2".repeat(40)),
        datetime!(2026-01-01 00:00:00 UTC),
    );
    let child = SourceCandidate::new_revision(
        &parent,
        repo("https://salsa.debian.org/foo/foo.git"),
        git_oid(&"3".repeat(40)),
        git_oid(&"4".repeat(40)),
        datetime!(2026-01-02 00:00:00 UTC),
    );
    let json = serde_json::to_string(&child).unwrap();
    let parsed: SourceCandidate = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, child);
    // parent_candidate must serialize as a string (transparent UUID).
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value["parent_candidate"].is_string());
}

#[test]
fn maintenance_event_round_trips_without_external_reference() {
    let e = MaintenanceEvent::new(
        MaintenanceEventId::new(),
        pkg_identity("debian", "unstable", "foo"),
        MaintenanceEventType::UpstreamRelease,
        EventSource::new("upstream-watch").unwrap(),
        datetime!(2026-01-01 00:00:00 UTC),
        None,
    )
    .unwrap();
    let json = serde_json::to_string(&e).unwrap();
    let parsed: MaintenanceEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, e);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["event_type"], "upstream_release");
    // `external_reference` is `skip_serializing_if = "Option::is_none"`,
    // so it must be absent from the wire format when None.
    assert!(value.get("external_reference").is_none());
}

#[test]
fn maintenance_event_with_external_reference_round_trips() {
    let e = MaintenanceEvent::new(
        MaintenanceEventId::new(),
        pkg_identity("debian", "unstable", "foo"),
        MaintenanceEventType::IssueReported,
        EventSource::new("issue-tracker").unwrap(),
        datetime!(2026-01-01 00:00:00 UTC),
        Some("tracker/12345".to_string()),
    )
    .unwrap();
    let json = serde_json::to_string(&e).unwrap();
    let parsed: MaintenanceEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, e);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["external_reference"], "tracker/12345");
}

#[test]
fn maintenance_job_round_trips() {
    let job = MaintenanceJob::new(
        JobId::new(),
        pkg_identity("debian", "unstable", "foo"),
        MaintenanceEventId::new(),
        datetime!(2026-01-01 00:00:00 UTC),
    );
    let json = serde_json::to_string(&job).unwrap();
    let parsed: MaintenanceJob = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, job);
}

#[test]
fn job_projection_round_trips() {
    let job = MaintenanceJob::new(
        JobId::new(),
        pkg_identity("debian", "unstable", "foo"),
        MaintenanceEventId::new(),
        datetime!(2026-01-01 00:00:00 UTC),
    );
    let proj = JobProjection {
        job,
        state: JobState::Intake,
        active_candidate: None,
        version: 1,
        updated_at: datetime!(2026-01-02 00:00:00 UTC),
    };
    let json = serde_json::to_string(&proj).unwrap();
    let parsed: JobProjection = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, proj);
}

#[test]
fn fingerprint_transparent_string_round_trip() {
    let fp = CandidateFingerprint::from_hex("0".repeat(64)).unwrap();
    let json = serde_json::to_string(&fp).unwrap();
    assert!(json.starts_with('"') && json.ends_with('"'));
    let parsed: CandidateFingerprint = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, fp);
}

#[test]
fn job_state_round_trips_for_every_variant() {
    // Belt-and-braces: every JobState variant must serialize to its
    // declared `name()` and parse back to itself.
    for s in JobState::ALL {
        let json = serde_json::to_string(&s).unwrap();
        let parsed: JobState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s, "round-trip failed for {s:?}");
    }
}
