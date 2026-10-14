//! End-to-end tests for PHASE-1.1 PR 1.1.C (real BTS
//! integration).
//!
//! The pre-1.1.C `bts_snapshot::run` returned a hard-coded
//! three-bug fixture regardless of the candidate's source
//! package; the live BTS transport was never exercised, so
//! `bugs.debian.org` (or its test stand-in) was never
//! consulted. PR 1.1.C replaces the hard-coded fixture
//! with a subprocess call to the `bts-helper.py` shim
//! (path controlled by the `BTS_HELPER` env var).
//!
//! The tests below use a *fake-BTS* shell script (a POSIX
//! `sh` script on the test's tempdir) to control the
//! helper's output. The script is created per-test; the
//! test sets `BTS_HELPER` to its absolute path and invokes
//! the `ironmaint-debian-tool bts-snapshot` subcommand as
//! a subprocess. The normalizer's verdict on the
//! subprocess's stdout is the assertion.
//!
//! ## Test surface
//!
//! - `fake_bts_returns_malformed_json_is_infrastructure_error` —
//!   the fake-BTS prints `{"this is not": "the expected
//!   shape"}` (a JSON object that is *not* a valid
//!   `DebianIssueReportV1`). The bts-snapshot subcommand
//!   parses the helper's stdout; the parse failure
//!   surfaces as `verdict: infrastructure_error` (per
//!   §24), and the normalizer maps the report to
//!   `Unclassifiable` (→ `ExecutorError::InfrastructureFailed`).
//!   RED: the bts-snapshot subcommand returns the
//!   hard-coded hermetic fixture, so the fake-BTS is
//!   never invoked and the normalizer returns `Pass`.
//!   The test fails because the asserted `Unclassifiable`
//!   is missing.
//!
//! - `fake_bts_returns_a_known_bug_shape_is_pass` — the
//!   fake-BTS prints a valid `DebianIssueReportV1` (the
//!   same three-bug shape the pre-1.1.C hermetic fixture
//!   emitted). The bts-snapshot subcommand parses the
//!   helper's stdout, returns the report, and the
//!   normalizer maps it to `Pass`. RED: the subcommand
//!   returns the hard-coded fixture regardless of the
//!   helper, so the assertion that the report's bug list
//!   matches what the fake-BTS emitted fails.
//!   (Both bugs-list paths would be 3 entries, so a
//!   stronger test is the report's `snapshot_at` field:
//!   the hermetic fixture's constant is
//!   `2026-10-09T00:00:00Z`, and the fake-BTS emits the
//!   helper's "now" timestamp. The RED state reports the
//!   constant; the GREEN state reports the helper's
//!   "now" timestamp; the assertion pins the GREEN
//!   behaviour.)

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaint_debian_tool::DebianBtsSnapshotNormalizer;
use ironmaint_debian_tool::binary_path;
use ironmaint_executor::{ExecutionRecord, NormalizationError, ResultNormalizer, RetryClass};
use time::macros::datetime;

/// Build a `CandidateInput` whose `package` is `example`
/// `1.0.0-1`. The `version` field is the candidate's
/// `source_version`; the bts-snapshot subcommand uses
/// only the `package.name` (the BTS lookup key), so the
/// version is a stable placeholder.
fn good_input_json() -> String {
    serde_json::json!({
        "schema": "ironmaint.candidate_input.v3",
        "job_id": "0192beef-0000-7000-8000-000000000001",
        "check_id": "0192beef-0000-7000-8000-000000000002",
        "package": {
            "name": "example",
            "version": "1.0.0-1",
        }
    })
    .to_string()
}

fn make_record(stdout: String, stderr: String, exit_code: i32) -> ExecutionRecord {
    ExecutionRecord {
        tool_key: ToolCapabilityKey::new("debian.issue.bts_snapshot").unwrap(),
        retry_class: RetryClass::Safe,
        started_at: datetime!(2026-01-01 00:00:00 UTC),
        finished_at: datetime!(2026-01-01 00:00:01 UTC),
        exit_code,
        stdout,
        stderr,
        retries_exhausted: false,
        truncated: false,
        artifacts: Vec::new(),
        artifacts_dropped: Vec::new(),
    }
}

/// Write a POSIX `sh` script to `dir/name` with the given
/// body (everything after the `#!/bin/sh` shebang) and
/// `0o755` permissions. The script is the fake-BTS the
/// test will use; the body is a `printf` or `cat` of the
/// malformed / well-formed output.
fn write_fake_bts(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).expect("create fake-bts script");
    f.write_all(b"#!/bin/sh\n").expect("write shebang");
    f.write_all(body.as_bytes()).expect("write body");
    drop(f);
    let mut perms = std::fs::metadata(&path)
        .expect("stat fake-bts")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod fake-bts");
    path
}

/// Run the bts-snapshot subcommand with `BTS_HELPER` set
/// to `helper_path` and the given JSON `input_json` on
/// stdin. Returns `(exit_code, stdout, stderr)`.
fn run_bts_snapshot_subprocess(helper_path: &str, input_json: &str) -> (i32, String, String) {
    let bin = binary_path();
    let mut child = std::process::Command::new(&bin)
        .arg("bts-snapshot")
        .env("BTS_HELPER", helper_path)
        // Reset PATH so the subprocess only finds the
        // fake-BTS via the explicit `BTS_HELPER` env var.
        .env(
            "PATH",
            helper_path
                .rsplit_once('/')
                .map(|(dir, _)| format!("{dir}:/usr/bin:/bin"))
                .unwrap_or_else(|| "/usr/bin:/bin".to_string()),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ironmaint-debian-tool bts-snapshot");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input_json.as_bytes())
        .expect("write input JSON to stdin");
    let out = child.wait_with_output().expect("wait for bts-snapshot");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The fake-BTS returns malformed JSON. The bts-snapshot
/// subcommand must parse the helper's stdout; the parse
/// failure surfaces as `verdict: infrastructure_error` (per
/// §24), and the normalizer maps the report to
/// `Unclassifiable` (→ `ExecutorError::InfrastructureFailed`).
///
/// Pre-1.1.C: the bts-snapshot subcommand returns the
/// hard-coded hermetic fixture (`verdict: pass`, three
/// bugs), so the fake-BTS is never invoked and the
/// normalizer returns `Pass`. The test fails because the
/// asserted `Unclassifiable` is missing.
#[test]
fn fake_bts_returns_malformed_json_is_infrastructure_error() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let _fake = write_fake_bts(
        tmp.path(),
        "fake-bts",
        // The body emits a JSON object that is not a
        // valid `DebianIssueReportV1` (no `verdict`, no
        // `report_kind`, no `schema_version`). The
        // bts-snapshot subcommand's `serde_json::from_str`
        // rejects this, surfacing as
        // `verdict: infrastructure_error`.
        "printf '{\"this is not\": \"the expected shape\"}'\n",
    );

    let helper_path = tmp.path().join("fake-bts");
    let helper_path_str = helper_path.to_str().expect("fake-bts path is utf-8");
    let (exit_code, stdout, stderr) =
        run_bts_snapshot_subprocess(helper_path_str, &good_input_json());
    eprintln!("fake-bts exit_code={exit_code}");
    eprintln!("fake-bts stdout={stdout}");
    eprintln!("fake-bts stderr={stderr}");

    let result =
        DebianBtsSnapshotNormalizer::new().normalize(&make_record(stdout, stderr, exit_code));
    match result {
        Err(NormalizationError::Unclassifiable(msg)) => {
            assert!(
                msg.contains("debian.issue.bts_snapshot"),
                "the unclassifiable message names the routing key: {msg}"
            );
        }
        Ok(normalised) => panic!(
            "expected Unclassifiable (BTS transport failed), got {:?}; \
             the bts-snapshot subcommand is not shelling out to the helper",
            normalised.evidence_status
        ),
        Err(other) => panic!("expected Unclassifiable, got {other:?}"),
    }
}

/// The fake-BTS returns a valid `DebianIssueReportV1`
/// (the same three-bug shape the pre-1.1.C hermetic
/// fixture emitted). The bts-snapshot subcommand parses
/// the helper's stdout, returns the report, and the
/// normalizer maps it to `Pass`.
///
/// The assertion pins the GREEN behaviour: the report's
/// `snapshot_at` field is the fake-BTS's "now" timestamp
/// (here: `2099-01-01T00:00:00Z`), not the pre-1.1.C
/// hermetic constant `2026-10-09T00:00:00Z`. The test
/// also asserts the report's `bugs` list is the one the
/// fake-BTS emitted (so the subcommand's parse is
/// actually consuming the helper's output, not the
/// in-memory hermetic fixture).
///
/// Pre-1.1.C: the bts-snapshot subcommand returns the
/// hard-coded hermetic fixture regardless of the helper,
/// so the asserted `snapshot_at` and bug-shape fail.
#[test]
fn fake_bts_returns_a_known_bug_shape_is_pass() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    // A well-formed report the fake-BTS emits. The
    // `snapshot_at` is a future date that the
    // pre-1.1.C hermetic fixture's constant
    // (`2026-10-09T00:00:00Z`) cannot accidentally match.
    // The `bugs` list is the same three-bug shape the
    // pre-1.1.C hermetic fixture emitted, so a test that
    // only checks `bugs.len() == 3` would pass in both
    // states — the `snapshot_at` assertion is what makes
    // the test a GREEN-state-only pin.
    let report_json = serde_json::json!({
        "schema_version": 1,
        "report_kind": "debian.issue.bts_snapshot_v1",
        "verdict": "pass",
        "source_package": "example",
        "snapshot_at": "2099-01-01T00:00:00Z",
        "bugs": [
            {
                "id": 200_001,
                "severity": "serious",
                "status": "open",
                "subject": "fake-bts: serious open bug",
                "tags": ["patch"],
                "forwarded": false,
                "patch_tagged": true,
            },
            {
                "id": 200_002,
                "severity": "normal",
                "status": "open",
                "subject": "fake-bts: open bug, forwarded upstream",
                "tags": ["upstream"],
                "forwarded": true,
                "patch_tagged": false,
            }
        ],
        "summary": {
            "open": 2,
            "serious": 1,
            "patch_tagged": 1,
            "forwarded": 1,
        }
    })
    .to_string();
    let body = format!("printf '%s' '{report_json}'\n");
    let _fake = write_fake_bts(tmp.path(), "fake-bts", &body);

    let helper_path = tmp.path().join("fake-bts");
    let helper_path_str = helper_path.to_str().expect("fake-bts path is utf-8");
    let (exit_code, stdout, stderr) =
        run_bts_snapshot_subprocess(helper_path_str, &good_input_json());
    eprintln!("fake-bts exit_code={exit_code}");
    eprintln!("fake-bts stdout={stdout}");
    eprintln!("fake-bts stderr={stderr}");

    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("bts-snapshot stdout is not valid JSON: {e}; stderr: {stderr}"));
    assert_eq!(
        report["verdict"], "pass",
        "the helper emitted a `pass` verdict; the bts-snapshot subcommand must surface it"
    );
    assert_eq!(
        report["snapshot_at"], "2099-01-01T00:00:00Z",
        "the helper's `snapshot_at` is the source of truth; \
         the pre-1.1.C hermetic constant `2026-10-09T00:00:00Z` would mean the subcommand \
         ignored the helper"
    );
    assert_eq!(
        report["bugs"].as_array().map(Vec::len),
        Some(2),
        "the helper emitted a 2-bug list; the subcommand must surface it (not the 3-bug \
         pre-1.1.C hermetic fixture)"
    );

    // The normalizer maps the well-formed `pass` report
    // to `EvidenceStatus::Pass`. This is a belt-and-braces
    // pin against the normalizer's mapping; the
    // pre-1.1.C `verdict: pass` mapping is unchanged.
    let result =
        DebianBtsSnapshotNormalizer::new().normalize(&make_record(stdout, stderr, exit_code));
    let normalised = result
        .unwrap_or_else(|e| panic!("expected Pass, got {e:?}; the bts-snapshot subcommand must emit a well-formed report when the helper succeeds"));
    assert_eq!(
        format!("{:?}", normalised.evidence_status),
        "Pass",
        "the §24 `pass` verdict is `EvidenceStatus::Pass`"
    );
}
