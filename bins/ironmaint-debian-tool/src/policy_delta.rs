//! `debian.policy.delta_review` — 1D.2 real check.
//!
//! PHASE-1.md §13 / §19 / §22. The 1D.2 check evaluates
//! the candidate's `debian/control` `Standards-Version`
//! against the §19 manifest's
//! `standards_version_target` (currently `"4.7.4"`).
//! A candidate whose `Standards-Version` is *newer*
//! than the baseline triggers the §22 gate:
//!
//! > "IronMaint's Debian Policy knowledge is stale;
//! >  refresh the authority snapshot before evaluating
//! >  this candidate."
//!
//! The spec is explicit (PHASE-1.md line 978) that
//! a candidate above baseline "must not be marked
//! bad": the manifest is stale, not the package. The
//! verdict is therefore `Verdict::InfrastructureError`
//! (which the normalizer routes to
//! `ExecutorError::InfrastructureFailed` and the
//! runtime surfaces as `GateStatus::Blocked`),
//! not `Verdict::Fail`.
//!
//! A candidate below baseline yields
//! `Verdict::Pass` *with* the §20
//! `intervening_checklist_entries` field populated:
//! the operator's remediation is the named entries
//! from `policy/debian/upgrading-checklist.v1.json`.
//!
//! The wire format follows the 1C.1 / 1C.2 / 1C.3
//! precedent:
//!
//! - Read `ironmaint.candidate_input.v3` from stdin
//!   (one document, EOF-terminated, deserialised into
//!   the `CandidateInput` struct). 1D.2 reuses 1C.3's
//!   v3 shape — no schema bump is needed.
//! - Write the versioned JSON report
//!   (`DebianPolicyDeltaReviewV1`) to stdout (one
//!   document, EOF-terminated).
//!
//! PHASE-1.1 PR 1.1.D (policy baseline correctness):
//! the pre-1.1.D comparator was the manifest's
//! `baseline_policy_version` (= `"4.7.4.1"`, the
//! *Policy Manual document* version). The new
//! comparator is the manifest's
//! `standards_version_target` (= `"4.7.4"`, the
//! `Standards-Version` target). The
//! `AboveBaseline` verdict is flipped from `Fail`
//! to `InfrastructureError`, and the
//! `intervening_checklist_entries` field on the
//! report is populated from
//! `policy/debian/upgrading-checklist.v1.json`.

use std::path::PathBuf;
use std::str::FromStr;

use deb822_lossless::Deb822;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::report::{
    ChecklistEntry, DebianPolicyDeltaReviewV1, Diagnostic, POLICY_DELTA_REPORT_KIND,
    POLICY_DELTA_SCHEMA_VERSION, Provenance, StandardsVersionComparison, Verdict,
};

/// The runtime's `ironmaint.candidate_input.v3` payload
/// (PHASE-1.md §7). 1D.2 reuses 1C.3's v3 shape.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct CandidateInput {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub job_id: String,
    #[serde(default)]
    pub check_id: String,
    pub package: Package,
    #[serde(default)]
    pub workspace_path: Option<PathBuf>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub tree: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Package {
    pub name: String,
    pub version: String,
}

/// The on-disk shape of
/// `policy/debian/upgrading-checklist.v1.json`.
///
/// PHASE-1.1 PR 1.1.D (policy baseline correctness):
/// the JSON is a parallel data file to
/// `authorities.v1.json`, carrying the §20
/// narrative of each `Standards-Version`
/// interval. The 1.1.D-bundled file carries
/// only the 4.7.4 → 4.7.4.1 interval; future
/// PRs can extend the JSON to cover more
/// intervals.
///
/// `pub` so the `xtask verify-schemas` tool
/// can call `schemars::schema_for!` for the
/// §97 schema-snapshot pin. The re-exports in
/// `lib.rs` expose the loader functions, not
/// the struct.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpgradingChecklistManifestV1 {
    pub schema_version: u32,
    pub manifest_version: String,
    pub distribution: String,
    pub intervals: Vec<ChecklistEntry>,
}

/// Load the §20 upgrading-checklist entries
/// bundled at compile time via `include_str!`.
///
/// PHASE-1.1 PR 1.1.D: this loader is the
/// data path for the new
/// `intervening_checklist_entries` field on
/// `DebianPolicyDeltaReviewV1`. The bundled
/// JSON carries only the 4.7.4 → 4.7.4.1
/// interval today; the loader returns
/// whatever intervals the JSON declares so
/// future extensions are zero-cost.
#[allow(clippy::expect_used)] // The manifest is bundled via `include_str!`
// at compile time; a deserialiser error
// here is a build-time defect, not a
// runtime concern. The `expect` is the
// right tool for a precondition violation.
fn load_upgrading_checklist() -> Vec<ChecklistEntry> {
    let raw = include_str!("../../../policy/debian/upgrading-checklist.v1.json");
    let manifest: UpgradingChecklistManifestV1 = serde_json::from_str(raw)
        .expect("policy/debian/upgrading-checklist.v1.json must be a valid manifest");
    manifest.intervals
}

/// The §22 Standards-Version comparison result.
///
/// `above_baseline` is the gate-triggering case:
/// the candidate declares a `Standards-Version`
/// newer than the manifest's
/// `standards_version_target`, so the candidate
/// cannot be evaluated against the adapter's
/// known Policy. `at_or_below_baseline` is the
/// pass-path case (the candidate's declared
/// version is at or below the manifest's known
/// Policy). `missing_candidate_standards_version`
/// records the absence of a `Standards-Version`
/// field on `debian/control` — a Policy violation
/// (§22 observation-not-compliance rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StandardsDelta {
    /// The candidate's `Standards-Version` is
    /// strictly newer than the manifest's
    /// `standards_version_target`. The §22 gate
    /// is blocked; the verdict is
    /// `InfrastructureError` (the manifest is
    /// stale, not the package).
    AboveBaseline,
    /// The candidate's `Standards-Version` is
    /// at or below the manifest's baseline. The
    /// §22 gate passes.
    AtOrBelowBaseline,
    /// The candidate's `debian/control` is
    /// missing the `Standards-Version` field.
    MissingCandidate,
}

/// Compare the candidate's declared
/// `Standards-Version` against the manifest's
/// `standards_version_target` using `debversion`'s
/// dpkg §5.6.12 lex-order.
///
/// `candidate_standards_version` is the value
/// extracted from `debian/control`. `baseline` is
/// `policy/debian/authorities.v1.json`'s
/// `standards_version_target` (= `"4.7.4"`).
fn compare_standards_version(
    candidate_standards_version: Option<&str>,
    baseline: &str,
) -> StandardsDelta {
    let Some(candidate) = candidate_standards_version else {
        return StandardsDelta::MissingCandidate;
    };
    let Ok(candidate_v) = debversion::Version::from_str(candidate) else {
        // A candidate with an unparsable
        // Standards-Version is treated as
        // missing (the §22 gate is "is the
        // declared version known?", and a
        // non-dpkg-parseable value is "not
        // known" — the adapter cannot rank it
        // against the baseline).
        return StandardsDelta::MissingCandidate;
    };
    let Ok(baseline_v) = debversion::Version::from_str(baseline) else {
        // The manifest's baseline is
        // unparseable — internal data error;
        // fail-closed to missing.
        return StandardsDelta::MissingCandidate;
    };
    if candidate_v > baseline_v {
        StandardsDelta::AboveBaseline
    } else {
        StandardsDelta::AtOrBelowBaseline
    }
}

/// Return the §20 upgrading-checklist entries
/// whose interval lies *on* the path from
/// `candidate` to `baseline`. An entry
/// `(from, to)` qualifies when
/// `from >= candidate` AND `to <= baseline`
/// (containment — the entry's interval is
/// entirely inside the candidate→baseline
/// range). The entries are returned in the
/// order they appear in the JSON file.
///
/// `candidate` and `baseline` are `&str`
/// `Standards-Version` values; unparseable
/// values yield an empty `Vec` (the report's
/// `intervening_checklist_entries` is empty
/// for the at-or-above cases).
///
/// PHASE-1.1 PR 1.1.J (policy delta evidence):
/// the pre-1.1.J predicate was
/// `to >= candidate && from <= baseline`
/// (overlap, not containment). That match
/// returned entries whose interval was
/// entirely *above* the candidate (e.g. the
/// bundled `4.7.4 → 4.7.4.1` entry when the
/// candidate is `4.6.0` and the baseline is
/// `4.7.4`), which is wrong: the entry is
/// on a path the candidate has not yet
/// reached. The new predicate is containment
/// (see `intervening_entries_in` for the
/// testable filter and the inline teeth-check).
fn intervening_entries(candidate: Option<&str>, baseline: &str) -> Vec<ChecklistEntry> {
    intervening_entries_in(candidate, baseline, &load_upgrading_checklist())
}

/// The testable filter (PHASE-1.1 PR 1.1.J). Takes the
/// checklist as a parameter so the predicate can be
/// exercised against a synthetic manifest with multiple
/// intervals; the production `intervening_entries` calls
/// this with the bundled `load_upgrading_checklist()`.
///
/// `candidate` and `baseline` are `&str`
/// `Standards-Version` values; unparseable
/// values yield an empty `Vec` (the report's
/// `intervening_checklist_entries` is empty
/// for the at-or-above cases).
fn intervening_entries_in(
    candidate: Option<&str>,
    baseline: &str,
    checklist: &[ChecklistEntry],
) -> Vec<ChecklistEntry> {
    let Some(candidate) = candidate else {
        return Vec::new();
    };
    let Ok(candidate_v) = debversion::Version::from_str(candidate) else {
        return Vec::new();
    };
    let Ok(baseline_v) = debversion::Version::from_str(baseline) else {
        return Vec::new();
    };
    checklist
        .iter()
        .filter(|e| {
            // CONTAINMENT (PHASE-1.1 PR 1.1.J): the
            // entry's interval must lie ON the path
            // from `candidate` to `baseline` —
            // i.e. `from >= candidate` and `to
            // <= baseline`. The previous *overlap*
            // test (`to >= candidate && from <=
            // baseline`) matched entries whose
            // interval was entirely above the
            // candidate (e.g. the bundled
            // `4.7.4 → 4.7.4.1` entry when the
            // candidate is 4.6.0 and the baseline
            // is 4.7.4), which is wrong: the entry
            // is on a path the candidate has not
            // yet reached. The new test in
            // `mod tests` pins the containment
            // shape against a synthetic
            // 4-interval manifest.
            let Ok(from_v) = debversion::Version::from_str(&e.from_version) else {
                return false;
            };
            let Ok(to_v) = debversion::Version::from_str(&e.to_version) else {
                return false;
            };
            from_v >= candidate_v && to_v <= baseline_v
        })
        .cloned()
        .collect()
}

/// Run the 1D.2 policy-delta check.
///
///   1. Reads `policy/debian/authorities.v1.json`
///      `standards_version_target` (= `"4.7.4"`).
///   2. Parses `debian/control` source paragraph
///      for the `Standards-Version` field.
///   3. Calls `compare_standards_version` and
///      emits `Verdict::InfrastructureError` with
///      the `E_POLICY_BASELINE_STALE` diagnostic
///      when the candidate is above baseline.
///   4. Emits `Verdict::Fail` with
///      `E_STANDARDS_VERSION_MISSING` when the
///      candidate's `debian/control` is missing
///      the field.
///   5. Populates `intervening_checklist_entries`
///      with the §20 narrative entries that
///      cover the candidate-to-baseline path.
pub fn run(input: &CandidateInput) -> DebianPolicyDeltaReviewV1 {
    let baseline = debian::authority_manifest::standards_version_target();

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut verdict = Verdict::Pass;

    // The candidate's `Standards-Version` is
    // the source paragraph's `Standards-Version`
    // field. `None` when the file is missing,
    // empty, unparsable, or the field is absent.
    let candidate_standards_version: Option<String> = match input.workspace_path.as_ref() {
        None => {
            diagnostics.push(Diagnostic {
                code: "E_WORKSPACE_PATH_MISSING".to_string(),
                message: "candidate input did not include workspace_path".to_string(),
                path: None,
            });
            verdict = Verdict::InfrastructureError;
            None
        }
        Some(p) if !p.is_dir() => {
            diagnostics.push(Diagnostic {
                code: "E_WORKSPACE_NOT_DIR".to_string(),
                message: format!("workspace_path {} is not a directory", p.display()),
                path: Some(p.clone()),
            });
            verdict = Verdict::InfrastructureError;
            None
        }
        Some(p) => {
            let control_path = p.join("debian").join("control");
            read_standards_version(&control_path)
        }
    };

    // The §22 comparison. The tri-state
    // AboveBaseline / AtOrBelowBaseline /
    // MissingCandidate is wired into the
    // verdict + diagnostics.
    //
    // PHASE-1.1 PR 1.1.D: the `AboveBaseline`
    // arm emits `Verdict::InfrastructureError`
    // (manifest is stale), not `Verdict::Fail`
    // (package is bad). The spec is explicit
    // (line 978) that a candidate above
    // baseline "must not be marked bad".
    let comparison_outcome: Option<StandardsVersionComparison> =
        match compare_standards_version(candidate_standards_version.as_deref(), baseline) {
            StandardsDelta::AboveBaseline => {
                diagnostics.push(Diagnostic {
                    code: "E_POLICY_BASELINE_STALE".to_string(),
                    message: format!(
                        "candidate declares Standards-Version \
                         {} but the manifest's standards_version_target is {}; \
                         the §22 gate is blocked until the \
                         authority snapshot is refreshed",
                        candidate_standards_version.as_deref().unwrap_or(""),
                        baseline
                    ),
                    path: None,
                });
                verdict = Verdict::InfrastructureError;
                Some(StandardsVersionComparison::AboveBaseline)
            }
            StandardsDelta::AtOrBelowBaseline => Some(
                if candidate_standards_version.as_deref() == Some(baseline) {
                    StandardsVersionComparison::AtBaseline
                } else {
                    StandardsVersionComparison::BelowBaseline
                },
            ),
            StandardsDelta::MissingCandidate => {
                if matches!(verdict, Verdict::Pass) {
                    diagnostics.push(Diagnostic {
                        code: "E_STANDARDS_VERSION_MISSING".to_string(),
                        message: "debian/control has no Standards-Version \
                             field; the §22 gate cannot evaluate \
                             the candidate's policy posture"
                            .to_string(),
                        path: None,
                    });
                    verdict = Verdict::Fail;
                }
                None
            }
        };

    // The §20 upgrading-checklist entries. The
    // pre-1.1.D report did not carry the §20
    // promise; the 1.1.D-corrected report
    // populates this field for `BelowBaseline`
    // cases. Other cases get an empty list.
    let intervening_checklist_entries: Vec<ChecklistEntry> = match comparison_outcome {
        Some(StandardsVersionComparison::BelowBaseline) => {
            intervening_entries(candidate_standards_version.as_deref(), baseline)
        }
        _ => Vec::new(),
    };

    DebianPolicyDeltaReviewV1 {
        schema_version: POLICY_DELTA_SCHEMA_VERSION,
        report_kind: POLICY_DELTA_REPORT_KIND.to_string(),
        provenance: Provenance {
            candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
            candidate_commit: input.commit.clone().unwrap_or_default(),
            candidate_tree: input.tree.clone().unwrap_or_default(),
            inspector_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        verdict,
        diagnostics,
        candidate_standards_version,
        baseline_standards_version: Some(baseline.to_string()),
        comparison_outcome,
        intervening_checklist_entries,
    }
}

/// Read the `Standards-Version` field from
/// `debian/control`'s source paragraph.
///
/// `None` when:
///   - The file does not exist.
///   - The file is empty.
///   - The deb822 parse fails.
///   - The source paragraph is missing the
///     `Standards-Version` field.
///   - The field is present but empty.
fn read_standards_version(control_path: &std::path::Path) -> Option<String> {
    let body = std::fs::read_to_string(control_path).ok()?;
    let parsed = Deb822::from_str(&body).ok()?;
    let source = parsed.paragraphs().next()?;
    let value = source.get("Standards-Version")?.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_input() -> CandidateInput {
        CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192a000-0000-0000-0000-000000000001".to_string(),
            check_id: "0192a000-0000-0000-0000-000000000002".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "1.0.0-1".to_string(),
            },
            workspace_path: Some(fixture_workspace()),
            fingerprint: Some("blake3:test".to_string()),
            commit: Some("a".repeat(40)),
            tree: Some("b".repeat(40)),
        }
    }

    fn fixture_workspace() -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        workspace_root.join("containers/fixtures/debian/example-1.0")
    }

    /// The §22 pass path: a candidate whose
    /// `Standards-Version` is at or below the
    /// manifest baseline yields
    /// `Verdict::Pass`. The example-1.0
    /// fixture's `Standards-Version: 4.6.2`
    /// is below the manifest's `4.7.4`
    /// baseline; the comparison outcome is
    /// `BelowBaseline` and the §20
    /// `intervening_checklist_entries` is
    /// populated.
    #[test]
    fn good_fixture_below_baseline_yields_pass() {
        let report = run(&good_input());
        assert_eq!(
            report.verdict,
            Verdict::Pass,
            "the well-formed hermetic fixture's verdict: {report:?}"
        );
        assert_eq!(report.candidate_standards_version.as_deref(), Some("4.6.2"));
        assert_eq!(report.baseline_standards_version.as_deref(), Some("4.7.4"));
        assert_eq!(
            report.comparison_outcome,
            Some(StandardsVersionComparison::BelowBaseline)
        );
    }

    /// The §22 gate path: a candidate whose
    /// `Standards-Version` is strictly above
    /// the manifest baseline yields
    /// `Verdict::InfrastructureError` with the
    /// `E_POLICY_BASELINE_STALE` diagnostic.
    /// The example-future-1.0 fixture's
    /// `Standards-Version: 4.9.0` is above
    /// the manifest's `4.7.4` baseline; the
    /// comparison outcome is `AboveBaseline`.
    ///
    /// PHASE-1.1 PR 1.1.D: the pre-1.1.D
    /// verdict was `Fail`; the post-1.1.D
    /// verdict is `InfrastructureError` (the
    /// manifest is stale, not the package).
    #[test]
    fn future_fixture_above_baseline_yields_infrastructure_error() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let future_workspace = workspace_root.join("containers/fixtures/debian/example-future-1.0");

        let mut input = good_input();
        input.workspace_path = Some(future_workspace);
        let report = run(&input);
        assert_eq!(
            report.verdict,
            Verdict::InfrastructureError,
            "the future fixture's verdict (post-1.1.D): {report:?}"
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
            "the §22 gate diagnostic must be present: {report:?}"
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
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "E_WORKSPACE_PATH_MISSING")
        );
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
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "E_WORKSPACE_NOT_DIR")
        );
    }

    /// The §17 fail path: a workspace that is
    /// not a Debian source tree yields
    /// `Verdict::Fail` and the
    /// `E_STANDARDS_VERSION_MISSING` diagnostic
    /// (the comparison outcome is `None`).
    #[test]
    fn missing_debian_directory_yields_fail_verdict() {
        let mut input = good_input();
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        input.workspace_path = Some(manifest.join("src"));
        let report = run(&input);
        assert_eq!(report.verdict, Verdict::Fail);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "E_STANDARDS_VERSION_MISSING")
        );
        assert_eq!(report.comparison_outcome, None);
    }

    /// The §22 tri-state: a candidate whose
    /// `Standards-Version` is exactly equal to
    /// the manifest baseline yields
    /// `Verdict::Pass` and the comparison
    /// outcome is `AtBaseline`. The example-1.0
    /// fixture's `4.6.2` is below baseline
    /// (covered by `good_fixture_below_baseline_yields_pass`);
    /// this test exercises the equality case
    /// by constructing a synthetic `control`
    /// file.
    #[test]
    fn at_baseline_yields_pass_with_at_baseline_outcome() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let debian_dir = tmp.path().join("debian");
        std::fs::create_dir_all(&debian_dir).expect("mkdir debian");
        std::fs::write(
            debian_dir.join("control"),
            "Source: example\nStandards-Version: 4.7.4\n",
        )
        .expect("write control");

        let mut input = good_input();
        input.workspace_path = Some(tmp.path().to_path_buf());
        let report = run(&input);
        assert_eq!(report.verdict, Verdict::Pass);
        assert_eq!(report.candidate_standards_version.as_deref(), Some("4.7.4"));
        assert_eq!(
            report.comparison_outcome,
            Some(StandardsVersionComparison::AtBaseline)
        );
    }

    /// The §22 unit test for the comparator:
    /// a candidate version strictly below
    /// baseline yields `AtOrBelowBaseline`; at
    /// baseline yields `AtOrBelowBaseline`; above
    /// yields `AboveBaseline`; missing yields
    /// `MissingCandidate`.
    ///
    /// Note: `debversion::Version::from_str` is
    /// permissive — an all-letters string like
    /// `"not-a-version"` parses as a valid
    /// dpkg-ordered version (it sorts above
    /// `"4.7.4"` per the dpkg §5.6.12
    /// lex-order because `'n' > '4'`). The
    /// `MissingCandidate` arm only fires when
    /// the candidate field is absent or
    /// `Some("")`.
    #[test]
    fn compare_standards_version_classifies_all_three_outcomes() {
        assert_eq!(
            compare_standards_version(Some("4.6.2"), "4.7.4"),
            StandardsDelta::AtOrBelowBaseline
        );
        assert_eq!(
            compare_standards_version(Some("4.7.4"), "4.7.4"),
            StandardsDelta::AtOrBelowBaseline
        );
        assert_eq!(
            compare_standards_version(Some("4.9.0"), "4.7.4"),
            StandardsDelta::AboveBaseline
        );
        assert_eq!(
            compare_standards_version(None, "4.7.4"),
            StandardsDelta::MissingCandidate
        );
        assert_eq!(
            compare_standards_version(Some(""), "4.7.4"),
            StandardsDelta::MissingCandidate
        );
    }

    /// PHASE-1.1 PR 1.1.J teeth-check (RED): the
    /// `intervening_entries` filter is supposed to return
    /// the entries whose interval lies on the path from
    /// the candidate to the baseline (containment), not
    /// the entries whose interval merely overlaps that
    /// path. With a synthetic 4-interval manifest this
    /// test pins the containment shape; today the
    /// predicate is overlap and the test fails.
    ///
    /// The synthetic manifest covers the 4.5.0 → 4.7.4.1
    /// path in 4 equal-ish intervals. With a candidate of
    /// 4.5.0 and a baseline of 4.7.4, the *correct*
    /// subset is the first three intervals (the path
    /// from 4.5.0 to 4.7.4); the 4.7.4 → 4.7.4.1 entry
    /// is entirely *above* the baseline, so it must be
    /// excluded. With a candidate of 4.6.0 the first
    /// interval (4.5.0 → 4.6.0) is entirely *below* the
    /// candidate and the 4.7.4 → 4.7.4.1 entry is above,
    /// so only the middle two are in the subset.
    #[test]
    fn intervening_entries_reports_containment_not_overlap() {
        let entries = vec![
            ChecklistEntry {
                from_version: "4.5.0".to_string(),
                to_version: "4.6.0".to_string(),
                summary: "4.5.0 → 4.6.0".to_string(),
                source: "synthetic".to_string(),
            },
            ChecklistEntry {
                from_version: "4.6.0".to_string(),
                to_version: "4.7.0".to_string(),
                summary: "4.6.0 → 4.7.0".to_string(),
                source: "synthetic".to_string(),
            },
            ChecklistEntry {
                from_version: "4.7.0".to_string(),
                to_version: "4.7.4".to_string(),
                summary: "4.7.0 → 4.7.4".to_string(),
                source: "synthetic".to_string(),
            },
            ChecklistEntry {
                from_version: "4.7.4".to_string(),
                to_version: "4.7.4.1".to_string(),
                summary: "4.7.4 → 4.7.4.1".to_string(),
                source: "synthetic".to_string(),
            },
        ];

        // Case 1: candidate at the bottom of the path.
        // Expect the first three entries; the 4.7.4 →
        // 4.7.4.1 entry is above the baseline and must
        // be excluded.
        let result = intervening_entries_in(Some("4.5.0"), "4.7.4", &entries);
        let summaries: Vec<&str> = result.iter().map(|e| &*e.summary).collect();
        assert_eq!(
            summaries,
            vec!["4.5.0 → 4.6.0", "4.6.0 → 4.7.0", "4.7.0 → 4.7.4"],
            "candidate=4.5.0, baseline=4.7.4: the entry's interval must lie ON the path \
             (containment). The pre-1.1.J overlap predicate returns the last two entries \
             (4.7.0→4.7.4 and 4.7.4→4.7.4.1), which is wrong: the 4.7.4→4.7.4.1 entry is \
             above the baseline. The first entry (4.5.0→4.6.0) is on the path and must be \
             present."
        );

        // Case 2: candidate in the middle of the path.
        // Expect only the middle two entries; the first
        // is below the candidate and the last is above
        // the baseline.
        let result = intervening_entries_in(Some("4.6.0"), "4.7.4", &entries);
        let summaries: Vec<&str> = result.iter().map(|e| &*e.summary).collect();
        assert_eq!(
            summaries,
            vec!["4.6.0 → 4.7.0", "4.7.0 → 4.7.4"],
            "candidate=4.6.0, baseline=4.7.4: the entry's interval must lie ON the path \
             (containment). The 4.5.0→4.6.0 entry is below the candidate; the \
             4.7.4→4.7.4.1 entry is above the baseline. Only the middle two are on the path."
        );
    }
}
