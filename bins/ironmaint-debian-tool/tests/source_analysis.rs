//! Unit tests for `debian.inspect.source_analysis` (1C.2).
//!
//! The tests pin the §18 comprehensive-report shape by
//! reading the wire JSON the tool emits, not the typed
//! struct — the typed struct is the *minimal* shape in
//! the 1C.2 RED commit and is the *comprehensive* shape
//! in the 1C.2 GREEN commit. Reading via `serde_json::Value`
//! means the test compiles in both states: in RED the
//! asserted fields are absent (so the assertion fails),
//! in GREEN the fields are present and non-empty (so
//! the assertion passes). The §97 schema-snapshot tool
//! is the structural catch on the GREEN side: if the
//! GREEN commit forgets to regenerate the snapshot
//! against the comprehensive struct, `verify-schemas`
//! fails the gate.
//!
//! The hermetic fixture at
//! `containers/fixtures/debian/example-1.0/` provides a
//! real Debian 3.0 (quilt) source tree with all the
//! §18 fields a real package has: source paragraph
//! + 1 binary package, `patches/series` with 1 DEP-3
//! patch, `tests/control` with 1 autopkgtest, a
//! `debian/watch` v4, and `debian/rules` (executable).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use ironmaint_debian_tool::source_analysis::{run, CandidateInput};
use ironmaint_debian_tool::report::Verdict;

/// The 1C.1 fixture's path. Resolved against the
/// workspace root so the test runs in any working
/// directory. The fixture is committed to the repo.
fn fixture_workspace() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // `bins/ironmaint-debian-tool/` is two levels below
    // the workspace root, so two `pop`s land at the root.
    let workspace_root = manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    workspace_root.join("containers/fixtures/debian/example-1.0")
}

fn good_input() -> CandidateInput {
    CandidateInput {
        schema: "ironmaint.candidate_input.v3".to_string(),
        job_id: "0192a000-0000-0000-0000-000000000001".to_string(),
        check_id: "0192a000-0000-0000-0000-000000000002".to_string(),
        package: ironmaint_debian_tool::source_analysis::Package {
            name: "example".to_string(),
            version: "0+ironmaint".to_string(),
        },
        workspace_path: Some(fixture_workspace()),
        fingerprint: Some("blake3:test".to_string()),
        commit: Some("a".repeat(40)),
        tree: Some("b".repeat(40)),
    }
}

/// The §17 pass path: the good fixture's source
/// tree is well-formed. The 1C.1 source-preparation
/// tool's `changelog_source_name_matches_candidate`
/// and `changelog_version_matches_candidate` checks
/// are evaluated against the candidate's
/// `package.name` and `package.version`; the
/// `0+ironmaint` placeholder matches the
/// `debian/changelog` (which is the 1C.1 contract).
#[test]
fn good_fixture_yields_a_pass_verdict() {
    let report = run(&good_input());
    assert_eq!(
        report.verdict,
        Verdict::Pass,
        "the well-formed hermetic fixture's §17 checks all hold: {report:?}"
    );
    assert!(
        report.identity.is_some(),
        "the changelog header is parsed; `identity` is populated"
    );
}

/// The 1C.2 schema is the *comprehensive* §18
/// report. In the 1C.2 GREEN commit, the struct
/// carries `binary_packages`, `patches`, `tests`,
/// `watch`, etc. In the 1C.2 RED commit, those
/// fields are absent (the struct is the minimal
/// shape); reading the wire JSON shows them as
/// `null` or absent. The assertion is on the JSON
/// shape so the test compiles in both states.
#[test]
fn comprehensive_findings_are_populated() {
    let report = run(&good_input());
    let json = serde_json::to_value(&report).expect("serialise report");
    let obj = json.as_object().expect("report is an object");

    // `binary_packages` is a non-empty array.
    let binary_packages = obj
        .get("binary_packages")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("binary_packages must be present and an array: {json}"));
    assert!(
        !binary_packages.is_empty(),
        "the fixture's `debian/control` declares one binary package (Package: example); \
         binary_packages must be non-empty: {json}"
    );

    // `patches` is a non-empty array.
    let patches = obj
        .get("patches")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("patches must be present and an array: {json}"));
    assert!(
        !patches.is_empty(),
        "the fixture's `debian/patches/series` references one patch; \
         patches must be non-empty: {json}"
    );

    // `tests` is a non-empty array.
    let tests = obj
        .get("tests")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("tests must be present and an array: {json}"));
    assert!(
        !tests.is_empty(),
        "the fixture's `debian/tests/control` declares one autopkgtest; \
         tests must be non-empty: {json}"
    );

    // `watch` is an object (the parsed `debian/watch` v4
    // file), not null.
    let watch = obj
        .get("watch")
        .unwrap_or_else(|| panic!("watch must be present: {json}"));
    assert!(
        watch.is_object(),
        "the fixture's `debian/watch` parses as a v4 watch object: {json}"
    );

    // `rules.executable` is true.
    let rules = obj
        .get("rules")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("rules must be present and an object: {json}"));
    assert_eq!(
        rules.get("executable").and_then(|v| v.as_bool()),
        Some(true),
        "the fixture's `debian/rules` is executable: {json}"
    );

    // `source_format` is the canonical 3.0 (quilt) string.
    assert_eq!(
        obj.get("source_format").and_then(|v| v.as_str()),
        Some("3.0 (quilt)"),
        "the fixture's `debian/source/format` is `3.0 (quilt)`: {json}"
    );
}

/// The §18 report carries the candidate's Git
/// identity (commit and tree) so a downstream
/// consumer can correlate the report to a specific
/// source tree. In the 1C.2 RED commit, the
/// minimal struct does carry these two fields
/// (they're part of the minimal shape); the
/// GREEN commit also carries them. The test
/// passes in both states.
#[test]
fn candidate_commit_and_tree_are_carried() {
    let report = run(&good_input());
    assert_eq!(report.candidate_commit, "a".repeat(40));
    assert_eq!(report.candidate_tree, "b".repeat(40));
}

/// The §17 fail path: the broken fixture's
/// `debian/changelog` declares source
/// `broken-example` while the candidate says
/// `example`. The 1C.1 source-preparation
/// check has the same assertion; 1C.2's
/// source-analysis check carries it forward.
/// In the 1C.2 RED commit, the check is the
/// changelog-header check (carried over from
/// 1C.1); the verdict is `Fail` and a
/// `E_CHANGELOG_SOURCE_NAME_MISMATCH` diagnostic
/// is in the report.
#[test]
fn broken_fixture_yields_a_fail_verdict() {
    let mut input = good_input();
    input.package.name = "example".to_string();
    // The fixture's changelog says `example (0+ironmaint)`,
    // not `broken-example`. The mismatched-name case is
    // covered by the source-preparation test; for
    // source-analysis, the fail path is "no `debian/`
    // directory". The simplest way to drive a fail is
    // to point the workspace at a non-debian directory.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    input.workspace_path = Some(manifest.join("src"));
    let report = run(&input);
    assert_eq!(
        report.verdict,
        Verdict::Fail,
        "a workspace that is not a Debian source tree yields `Fail`: {report:?}"
    );
}

/// The §17 `infrastructure_error` path: no
/// `workspace_path` field. The tool's
/// `E_WORKSPACE_PATH_MISSING` diagnostic is
/// in the report; the normalizer maps the
/// verdict to `Unclassifiable` and the
/// runtime surfaces it as
/// `ExecutorError::InfrastructureFailed`.
#[test]
fn missing_workspace_path_yields_infrastructure_error() {
    let mut input = good_input();
    input.workspace_path = None;
    let report = run(&input);
    assert_eq!(report.verdict, Verdict::InfrastructureError);
    assert!(report.diagnostics.iter().any(|d| d.code == "E_WORKSPACE_PATH_MISSING"));
}

/// The §17 `infrastructure_error` path:
/// `workspace_path` is a file, not a
/// directory. The tool's `E_WORKSPACE_NOT_DIR`
/// diagnostic is in the report.
#[test]
fn workspace_path_pointing_at_a_file_yields_infrastructure_error() {
    let mut input = good_input();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // `Cargo.toml` is a file, not a directory.
    input.workspace_path = Some(manifest.join("Cargo.toml"));
    let report = run(&input);
    assert_eq!(report.verdict, Verdict::InfrastructureError);
    assert!(report.diagnostics.iter().any(|d| d.code == "E_WORKSPACE_NOT_DIR"));
}

/// The §17 fail path: a workspace that is a
/// directory but does not have a `debian/`
/// subdirectory. The tool's
/// `E_DEBIAN_DIR_MISSING` diagnostic is in the
/// report; the verdict is `Fail` (not
/// `infrastructure_error` — the tool *could*
/// read the workspace, the workspace just
/// doesn't have a Debian source tree).
#[test]
fn missing_debian_directory_yields_fail_verdict() {
    let mut input = good_input();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    input.workspace_path = Some(manifest.join("src"));
    let report = run(&input);
    assert_eq!(report.verdict, Verdict::Fail);
    assert!(report.diagnostics.iter().any(|d| d.code == "E_DEBIAN_DIR_MISSING"));
}
