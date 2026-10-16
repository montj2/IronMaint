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
/// entries between the candidate's `4.6.0` and the
/// baseline's `4.7.4`.
///
/// Pre-1.1.D: the `intervening_checklist_entries` field
/// does not exist on `DebianPolicyDeltaReviewV1` — the
/// test cannot even reference it. RED.
///
/// PHASE-1.1 PR 1.1.J (policy delta evidence): the test
/// is tightened to assert the *exact* subset of §20
/// entries the report carries. The bundled
/// `upgrading-checklist.v1.json` (post-1.1.J) lists
/// four intervals: 4.5.0→4.6.0, 4.6.0→4.7.0,
/// 4.7.0→4.7.4, 4.7.4→4.7.4.1. With a candidate of
/// `4.6.0` and a baseline of `4.7.4`, the *containment*
/// predicate (`from >= candidate && to <= baseline`)
/// returns exactly the two middle intervals: the
/// 4.5.0→4.6.0 entry is below the candidate; the
/// 4.7.4→4.7.4.1 entry is above the baseline. The
/// pre-1.1.J overlap predicate returned
/// `[4.7.0→4.7.4, 4.7.4→4.7.4.1]` (the last two —
/// wrong: the 4.7.4→4.7.4.1 entry is entirely above
/// the baseline).
#[test]
fn below_baseline_is_pass_with_checklist_entries() {
    let (_tmp, workspace) = control_with_body(
        "Source: example\n\
Maintainer: IronMaint Test <test@example.invalid>\n\
Section: utils\n\
Priority: optional\n\
Standards-Version: 4.6.0\n\
\n\
Package: example\n\
Architecture: any\n\
Description: below-baseline fixture for PR 1.1.D\n",
    );
    let report = run(&input_with_workspace(workspace));
    assert_eq!(report.verdict, Verdict::Pass, "report: {report:?}");
    assert_eq!(report.candidate_standards_version.as_deref(), Some("4.6.0"));
    assert_eq!(report.baseline_standards_version.as_deref(), Some("4.7.4"));
    assert_eq!(
        report.comparison_outcome,
        Some(StandardsVersionComparison::BelowBaseline)
    );
    // The §20 checklist promise. The bundled
    // `upgrading-checklist.v1.json` carries four
    // intervals (4.5.0→4.6.0, 4.6.0→4.7.0,
    // 4.7.0→4.7.4, 4.7.4→4.7.4.1). With
    // candidate=4.6.0 and baseline=4.7.4, the
    // *containment* predicate
    // (`from >= 4.6.0 && to <= 4.7.4`) returns
    // exactly the two middle intervals, in JSON
    // order. The pre-1.1.J overlap predicate
    // (`to >= 4.6.0 && from <= 4.7.4`) returned
    // the last two — wrong: the 4.7.4→4.7.4.1
    // entry is entirely *above* the baseline.
    let entries = &report.intervening_checklist_entries;
    let summaries: Vec<&str> = entries.iter().map(|e| e.summary.as_str()).collect();
    assert_eq!(
        summaries,
        vec![
            "4.6.0 → 4.7.0: tightened the §4.6 build-dependency rules and the §12.3 binary-control fields; the §22 gate now flags 4.6.0→4.7.0 transitions explicitly.",
            "4.7.0 → 4.7.4: updated the §10.1 architecture list and the §11.8.4 dpkg-statoverride note; the §22 gate names the 4.7.0 → 4.7.4 interval on a candidate below the baseline.",
        ],
        "candidate=4.6.0, baseline=4.7.4: the report must name the two entries \
         whose interval is on the path from the candidate to the baseline \
         (containment). The 4.5.0→4.6.0 entry is below the candidate; the \
         4.7.4→4.7.4.1 entry is above the baseline. Pre-1.1.J the overlap \
         predicate returned the wrong subset (last two instead of middle two); \
         report: {report:?}"
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
