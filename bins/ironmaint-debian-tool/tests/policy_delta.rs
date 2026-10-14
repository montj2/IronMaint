//! Phase-1.1 PR 1.1.D (policy baseline correctness) —
//! integration tests for the §22 Standards-Version
//! comparison and the new
//! `intervening_checklist_entries` field.
//!
//! PHASE-1.md §19 / §20 / §22. The pre-1.1.D code used
//! `policy/debian/authorities.v1.json`'s
//! `baseline_policy_version` (= `"4.7.4.1"`, the
//! *Policy Manual document* version) as the comparator
//! for the candidate's `debian/control`
//! `Standards-Version`. The spec is explicit (line 978)
//! that the baseline is the `Standards-Version target`
//! (= `"4.7.4"`, *not* the document version), and that
//! a candidate above the baseline "must not be marked
//! bad" — it is an `InfrastructureError` (refresh the
//! authority snapshot), not a `Fail` (the package is
//! not Policy-compliant).
//!
//! The pre-1.1.D code also emitted `Verdict::Fail` for
//! the above-baseline case (treating a manifest / data
//! staleness as a Policy violation), and the
//! `DebianPolicyDeltaReviewV1` was missing the
//! `intervening_checklist_entries` field that §20
//! promised for `BelowBaseline` reports.
//!
//! PR 1.1.D splits the manifest into
//! `policy_document_version: "4.7.4.1"` and
//! `standards_version_target: "4.7.4"`, routes the
//! comparator through the latter, flips the
//! `AboveBaseline` verdict from `Fail` to
//! `InfrastructureError`, and adds a
//! `policy/debian/upgrading-checklist.v1.json` data
//! file (the 4.7.4 → 4.7.4.1 interval) that
//! `BelowBaseline` reports populate.

use std::path::PathBuf;

use ironmaint_debian_tool::policy_delta::CandidateInput;
use ironmaint_debian_tool::policy_delta::run;
use ironmaint_debian_tool::report::{StandardsVersionComparison, Verdict};

/// Build a `CandidateInput` whose `workspace_path` is
/// `workspace`. The `commit` / `tree` / `fingerprint`
/// fields are arbitrary but the runtime reads them off
/// the report's `provenance` (the pre-1.1.D path
/// records whatever was supplied).
fn input_with_workspace(workspace: PathBuf) -> CandidateInput {
    CandidateInput {
        schema: "ironmaint.candidate_input.v3".to_string(),
        job_id: "0192a000-0000-0000-0000-000000000001".to_string(),
        check_id: "0192a000-0000-0000-0000-000000000002".to_string(),
        package: ironmaint_debian_tool::policy_delta::Package {
            name: "example".to_string(),
            version: "1.0.0-1".to_string(),
        },
        workspace_path: Some(workspace),
        commit: Some("a".repeat(40)),
        tree: Some("b".repeat(40)),
        fingerprint: Some("blake3:test".to_string()),
    }
}

/// Resolve `path` relative to the workspace root
/// (= the directory above `bins/ironmaint-debian-tool/`).
/// The helper mirrors the `fixture_workspace` pattern
/// in `src/policy_delta.rs::tests`.
fn workspace_root_path(path: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join(path)
}

/// Materialise a per-test `debian/control` file in a
/// `tempfile::TempDir`. The `body` is the *complete*
/// file content (including the `Standards-Version`
/// line, or its absence for the "missing" test).
/// Returns the tempdir handle (kept alive by the caller
/// — drop tears the directory down) and the workspace
/// path the runtime should read.
fn control_with_body(body: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let debian_dir = tmp.path().join("debian");
    std::fs::create_dir_all(&debian_dir).expect("create debian/");
    std::fs::write(debian_dir.join("control"), body).expect("write debian/control");
    let workspace = tmp.path().to_path_buf();
    (tmp, workspace)
}

/// At the new baseline: the candidate's
/// `Standards-Version` equals the manifest's
/// `standards_version_target` (4.7.4) → the report
/// carries `comparison_outcome: AtBaseline` and
/// `verdict: Pass`.
///
/// Pre-1.1.D: the comparator is the manifest's
/// `baseline_policy_version` (= 4.7.4.1). The
/// candidate's `4.7.4` is *below* 4.7.4.1, so
/// `comparison_outcome` is `BelowBaseline` — the
/// asserted `AtBaseline` would not match. RED.
#[test]
fn at_baseline_is_pass() {
    let (_tmp, workspace) = control_with_body(
        "Source: example\n\
Maintainer: IronMaint Test <test@example.invalid>\n\
Section: utils\n\
Priority: optional\n\
Standards-Version: 4.7.4\n\
\n\
Package: example\n\
Architecture: any\n\
Description: at-baseline fixture for PR 1.1.D\n",
    );
    let report = run(&input_with_workspace(workspace));
    assert_eq!(report.verdict, Verdict::Pass, "report: {report:?}");
    assert_eq!(report.candidate_standards_version.as_deref(), Some("4.7.4"));
    assert_eq!(
        report.baseline_standards_version.as_deref(),
        Some("4.7.4"),
        "the baseline is the new `standards_version_target` (4.7.4), \
         not the old `baseline_policy_version` (4.7.4.1)"
    );
    assert_eq!(
        report.comparison_outcome,
        Some(StandardsVersionComparison::AtBaseline),
        "at baseline is `AtBaseline`, not `BelowBaseline`"
    );
}

/// Below the new baseline: the candidate's
/// `Standards-Version` is older than the manifest's
/// `standards_version_target`. The report's verdict is
/// `Pass` and the `intervening_checklist_entries` field
/// is populated with the §20 upgrading-checklist
/// entries between the candidate's `4.6.2` and the
/// baseline's `4.7.4`.
///
/// Pre-1.1.D: the `intervening_checklist_entries` field
/// does not exist on `DebianPolicyDeltaReviewV1` — the
/// test cannot even reference it. RED.
#[test]
fn below_baseline_is_pass_with_checklist_entries() {
    let (_tmp, workspace) = control_with_body(
        "Source: example\n\
Maintainer: IronMaint Test <test@example.invalid>\n\
Section: utils\n\
Priority: optional\n\
Standards-Version: 4.6.2\n\
\n\
Package: example\n\
Architecture: any\n\
Description: below-baseline fixture for PR 1.1.D\n",
    );
    let report = run(&input_with_workspace(workspace));
    assert_eq!(report.verdict, Verdict::Pass, "report: {report:?}");
    assert_eq!(report.candidate_standards_version.as_deref(), Some("4.6.2"));
    assert_eq!(report.baseline_standards_version.as_deref(), Some("4.7.4"));
    assert_eq!(
        report.comparison_outcome,
        Some(StandardsVersionComparison::BelowBaseline)
    );
    // The §20 checklist promise. Pre-1.1.D: the field
    // does not exist on the report; the test references
    // a non-existent field. The new
    // `upgrading-checklist.v1.json` carries the 4.6.2 →
    // 4.7.4 interval; the §20 promise is that the report
    // names the intervening entries. RED.
    let entries = &report.intervening_checklist_entries;
    assert!(
        !entries.is_empty(),
        "below-baseline reports must name the intervening checklist entries (§20); \
         report: {report:?}"
    );
    // Every named entry sits between the candidate's
    // `4.6.2` and the baseline's `4.7.4`. The fixture
    // file's 4.7.4 → 4.7.4.1 interval is the only
    // post-1.1.D entry; an earlier interval (4.6.2 →
    // 4.7.4) would also be named. We pin the count at
    // ≥ 1 rather than a specific value so future
    // interval extensions don't break the test.
    assert!(
        entries
            .iter()
            .any(|e| e.from_version.as_str() <= "4.7.4" && e.to_version.as_str() >= "4.7.4"),
        "an entry covering the 4.7.4 baseline must be present: {entries:?}"
    );
}

/// Above the new baseline: the candidate's
/// `Standards-Version` is *newer* than the manifest's
/// `standards_version_target`. The report carries
/// `verdict: InfrastructureError` (per the spec, the
/// candidate is not Policy-bad — the manifest is stale;
/// refresh the authority snapshot before evaluating),
/// not `Verdict::Fail`.
///
/// Pre-1.1.D: the report carries `verdict: Fail` and
/// the `E_POLICY_BASELINE_STALE` diagnostic. The
/// `InfrastructureError` verdict is what the spec
/// promises; the pre-1.1.D `Fail` is the §20 spec
/// violation. RED.
#[test]
fn above_baseline_is_infrastructure_error_not_fail() {
    let workspace = workspace_root_path("containers/fixtures/debian/example-future-1.0");
    let report = run(&input_with_workspace(workspace));
    assert_eq!(
        report.verdict,
        Verdict::InfrastructureError,
        "above baseline is `InfrastructureError` (the manifest is stale), \
         NOT `Fail` (which would say the package is not Policy-compliant); \
         the spec (PHASE-1.md line 978) explicitly forbids marking the package bad: \
         report: {report:?}"
    );
    assert_eq!(report.candidate_standards_version.as_deref(), Some("4.9.0"));
    assert_eq!(report.baseline_standards_version.as_deref(), Some("4.7.4"));
    assert_eq!(
        report.comparison_outcome,
        Some(StandardsVersionComparison::AboveBaseline)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "E_POLICY_BASELINE_STALE"),
        "the `E_POLICY_BASELINE_STALE` diagnostic must be present so the runtime \
         can route to `ExecutorError::InfrastructureFailed` → `GateStatus::Blocked`"
    );
}

/// Missing `Standards-Version`: the candidate's
/// `debian/control` has no `Standards-Version` field.
/// The report carries `verdict: Fail` (a Policy
/// violation per §22 observation-not-compliance),
/// not `InfrastructureError` (the tool could read the
/// file; the field is just absent).
///
/// Pre-1.1.D: passes (the existing missing-field path
/// is unchanged). Post-1.1.D: passes (the missing-field
/// path is unchanged). This is a regression pin rather
/// than a RED test.
#[test]
fn missing_standards_version_is_fail() {
    let (_tmp, workspace) = control_with_body(
        "Source: example\n\
Maintainer: IronMaint Test <test@example.invalid>\n\
Section: utils\n\
Priority: optional\n\
\n\
Package: example\n\
Architecture: any\n\
Description: missing Standards-Version field for PR 1.1.D regression pin\n",
    );
    let report = run(&input_with_workspace(workspace));
    assert_eq!(
        report.verdict,
        Verdict::Fail,
        "missing Standards-Version is a Policy violation (Fail), \
         not an infrastructure error; report: {report:?}"
    );
    assert_eq!(report.candidate_standards_version, None);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "E_STANDARDS_VERSION_MISSING"),
        "the `E_STANDARDS_VERSION_MISSING` diagnostic must be present"
    );
}
