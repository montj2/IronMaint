//! End-to-end tests for the `debian.inspect.source_preparation`
//! tool against the hermetic §29 fixtures.
//!
//! Each test:
//!
//! 1. Copies a fixture tree into a `tempfile::TempDir`
//!    (the source_preparation check reads the workspace
//!    path, not a tarball).
//! 2. Constructs a `CandidateInput` with the fixture's
//!    `source_name` and `version`.
//! 3. Calls `ironmaint_debian_tool::source_preparation::run`.
//! 4. Asserts the report's `verdict` and per-check findings.
//!
//! The tests are hermetic (no subprocess) and exercise the
//! same logic the binary's `source-preparation` subcommand
//! exercises. The binary-level end-to-end test is the
//! daemon's `startup_shutdown` integration test
//! (`bins/ironmaintd/tests/startup_shutdown.rs`).

use std::fs;

use ironmaint_debian_tool::Verdict;
use ironmaint_debian_tool::source_preparation::{self, CandidateInput, Package};

/// Path to the good hermetic fixture, relative to the
/// crate root. `CARGO_MANIFEST_DIR` is set by Cargo at
/// build time and points at the directory containing
/// `Cargo.toml` — i.e., `bins/ironmaint-debian-tool/`.
/// The fixture itself is at
/// `containers/fixtures/debian/example-1.0/` at the
/// repository root.
const GOOD_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../containers/fixtures/debian/example-1.0"
);
const BROKEN_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../containers/fixtures/debian/example-broken-1.0.0"
);

/// Recursively copy `src` to `dst`. Used to stage the
/// hermetic fixture into a `TempDir` so the test owns the
/// directory lifecycle (no writes leak into the
/// committed fixture).
fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) {
    if src.is_dir() {
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let from = entry.path();
            let to = dst.join(entry.file_name());
            copy_recursive(&from, &to);
        }
    } else {
        fs::copy(src, dst).unwrap();
    }
}

/// Build a `CandidateInput` whose `package` matches the
/// good fixture (`example` 1.0.0-1). The version `1.0.0-1`
/// is the real Debian-style version the fixture's
/// `debian/changelog` declares; the workspace's
/// `capture_candidate` now resolves the version from the
/// changelog at capture time (PHASE-1.1 PR 1.1.A), so the
/// `version` we pass in here is what the runtime hands the
/// source-preparation check (the candidate's
/// `source_version`), and the cross-check is between the
/// candidate's version and what the changelog declares —
/// both should be `1.0.0-1`.
fn input_for(workspace_path: std::path::PathBuf) -> CandidateInput {
    CandidateInput {
        schema: "ironmaint.candidate_input.v2".to_string(),
        job_id: "0192beef-0000-7000-8000-000000000001".to_string(),
        check_id: "0192beef-0000-7000-8000-000000000002".to_string(),
        package: Package {
            name: "example".to_string(),
            version: "1.0.0-1".to_string(),
        },
        workspace_path: Some(workspace_path),
        fingerprint: Some("blake3:test-fixture".to_string()),
    }
}

#[test]
fn good_fixture_yields_a_pass_verdict() {
    let workspace = tempfile::tempdir().unwrap();
    let staged = workspace.path().to_path_buf();
    let fixture = std::path::Path::new(GOOD_FIXTURE);
    copy_recursive(fixture, &staged);
    let report = source_preparation::run(&input_for(staged));
    assert_eq!(
        report.verdict,
        Verdict::Pass,
        "diagnostics: {:?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics.is_empty(),
        "pass report must have no diagnostics"
    );
    assert!(report.findings.debian_directory_present);
    assert!(report.findings.control_parsable);
    assert!(report.findings.changelog_parsable);
    assert!(report.findings.format_identifiable);
    assert_eq!(
        report.findings.control_source_name.as_deref(),
        Some("example")
    );
    assert_eq!(
        report.findings.changelog_source_name.as_deref(),
        Some("example")
    );
    assert_eq!(
        report.findings.changelog_version.as_deref(),
        Some("1.0.0-1")
    );
    assert!(report.findings.control_source_name_matches_candidate);
    assert!(report.findings.changelog_source_name_matches_candidate);
    assert!(report.findings.changelog_version_matches_candidate);
    assert_eq!(
        report.findings.source_format.as_deref(),
        Some("3.0 (quilt)")
    );
    assert!(report.findings.rules_executable);
    let identity = report.identity.expect("pass fixture has an identity");
    assert_eq!(identity.source, "example");
    assert_eq!(identity.version, "1.0.0-1");
    assert_eq!(identity.distribution, "unstable");
    assert_eq!(identity.urgency, "medium");
}

#[test]
fn broken_fixture_yields_a_fail_verdict_with_a_changelog_mismatch_diagnostic() {
    let workspace = tempfile::tempdir().unwrap();
    let staged = workspace.path().to_path_buf();
    let fixture = std::path::Path::new(BROKEN_FIXTURE);
    copy_recursive(fixture, &staged);
    let report = source_preparation::run(&input_for(staged));
    assert_eq!(
        report.verdict,
        Verdict::Fail,
        "diagnostics: {:?}",
        report.diagnostics
    );
    let codes: Vec<&str> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert!(
        codes.contains(&"E_CHANGELOG_SOURCE_NAME_MISMATCH"),
        "expected a changelog-source-name mismatch diagnostic; got {codes:?}"
    );
    assert!(
        !report.findings.changelog_source_name_matches_candidate,
        "broken fixture has mismatched changelog source name"
    );
    // The control file in the broken fixture still says
    // `Source: example`, which matches the candidate, so
    // the control cross-check is `true`. The fail
    // verdict is driven by the changelog mismatch alone.
    assert!(report.findings.control_source_name_matches_candidate);
}

#[test]
fn missing_workspace_path_yields_infrastructure_error() {
    let mut input = input_for(std::path::PathBuf::from("/nonexistent/workspace"));
    input.workspace_path = None;
    let report = source_preparation::run(&input);
    assert_eq!(report.verdict, Verdict::InfrastructureError);
    assert_eq!(report.diagnostics[0].code, "E_WORKSPACE_PATH_MISSING");
}

#[test]
fn workspace_path_pointing_at_a_file_yields_infrastructure_error() {
    let workspace = tempfile::tempdir().unwrap();
    let file_path = workspace.path().join("not-a-directory");
    fs::write(&file_path, b"not a directory").unwrap();
    let report = source_preparation::run(&input_for(file_path));
    assert_eq!(report.verdict, Verdict::InfrastructureError);
    assert_eq!(report.diagnostics[0].code, "E_WORKSPACE_NOT_DIR");
}

#[test]
fn missing_debian_directory_yields_fail_verdict() {
    let workspace = tempfile::tempdir().unwrap();
    // No `debian/` directory under the workspace root.
    let report = source_preparation::run(&input_for(workspace.path().to_path_buf()));
    assert_eq!(report.verdict, Verdict::Fail);
    assert_eq!(report.diagnostics[0].code, "E_DEBIAN_DIR_MISSING");
    assert!(!report.findings.debian_directory_present);
}
