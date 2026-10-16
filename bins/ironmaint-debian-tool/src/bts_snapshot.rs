//! `debian.issue.bts_snapshot` — the 1E.2 BTS read-only
//! snapshot check.
//!
//! PHASE-1.md §23, §24, §25. The 1E.1 `bts-helper.py` shim
//! (in the `ironmaint/debian-tools:0.1` container) is the
//! read-only transport that talks to bugs.debian.org and
//! emits canonical JSON. This module is the
//! `ironmaint-debian-tool` binary's `bts-snapshot`
//! subcommand: it reads the runtime's v3 input, shells out
//! to the `bts` shim (in live mode), converts the shim's
//! canonical-JSON output into [`DebianIssueReportV1`], and
//! emits the report on stdout.
//!
//! ## Modes
//!
//! - **Hermetic / self-test** (1E.2 RED + GREEN): the
//!   subcommand emits a hard-coded fixture directly,
//!   without shelling out. The fixture's `verdict` is
//!   `pass`; the §23 fields (`source_package`,
//!   `snapshot_at`, `bugs`, `summary`) are populated
//!   from a small in-memory table. This is the mode the
//!   §97 gate exercises — it never touches the network
//!   and never shells out.
//!
//! - **Live / network** (opt-in, not in §97): the
//!   subcommand shells out to the `bts` shim with
//!   `--self-test <source_package>` for hermetic tests
//!   in the `ironmaint/debian-tools:0.1` container, or
//!   to `bts <source_package>` for real network access.
//!   The live mode is the §27 / §29 opt-in surface; the
//!   §97 gate is hermetic.
//!
//! ## Verdict semantics (§24)
//!
//! §24 says "A package having 50 open bugs is not a tool
//! failure." The snapshot's `verdict` is therefore
//! binary — `pass` or `infrastructure_error`. A snapshot
//! with 1000 open bugs is still `pass` (the bug list is
//! observed). Only transport-level failures (DNS / TLS /
//! timeout / SOAP / malformed service response) yield
//! `infrastructure_error`.
//!
//! ## Why pure-Rust (not a Python shell-out)
//!
//! The 1E.1 `bts` shim is the SOAP client; the 1E.2
//! subcommand is the *consumer* of the shim's output.
//! The subcommand does not need to know about the
//! BTS wire format — that's the shim's job. The
//! subcommand's job is "given a `source_package`,
//! produce a `DebianIssueReportV1`." The hermetic mode
//! does that from a hard-coded fixture; the live mode
//! would shell out to the shim.

use std::io;
use std::process::{Command, Stdio};

use serde::Deserialize;
use thiserror::Error;

use crate::report::{
    BTS_SNAPSHOT_REPORT_KIND, BTS_SNAPSHOT_SCHEMA_VERSION, BugRecord, BugSummary, IssueVerdict,
};

/// The runtime's `ironmaint.candidate_input.v3` payload
/// (PHASE-1.md §7). The bts-snapshot subcommand uses
/// only the candidate's `source_name` to look up the
/// BTS bug list; the rest of the v3 fields are
/// tolerated (serde's `deny_unknown_fields` is off) so
/// the v2 / v3 payload continues to parse.
#[derive(Debug, Deserialize)]
pub struct CandidateInput {
    #[allow(dead_code)]
    pub schema: String,
    #[allow(dead_code)]
    pub job_id: String,
    #[allow(dead_code)]
    pub check_id: String,
    pub package: Package,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    #[allow(dead_code)]
    pub version: String,
}

/// The fixed RFC3339 UTC timestamp the hermetic fixture
/// reports. A real snapshot would carry the actual
/// retrieval time; the hermetic mode reports a stable
/// constant so the `assert_eq!` on the integration
/// test's `snapshot_at` field is deterministic.
pub const HERMETIC_SNAPSHOT_AT: &str = "2026-10-09T00:00:00Z";

/// The env var that names the `bts` shim the
/// bts-snapshot subcommand shells out to. Default
/// behaviour: a hermetic Rust-side fixture (the
/// pre-1.1.C path) so the §97 gate does not need the
/// helper to be installed. PHASE-1.1 PR 1.1.C
/// (real BTS integration): the runtime sets this
/// to `/usr/local/bin/bts` (or the container's
/// `bts-helper.py` shim path) when the live
/// transport is desired; the integration test in
/// `tests/bts_snapshot_live.rs` sets it to a
/// per-test fake-bts script.
pub const BTS_HELPER_ENV: &str = "BTS_HELPER";

/// Errors the live BTS transport can produce.
///
/// PHASE-1.1 PR 1.1.C (real BTS integration): the
/// `bts_snapshot` subcommand shells out to the
/// `bts-helper.py` shim (path controlled by the
/// `BTS_HELPER` env var). The three failure modes
/// — `Transport` (could not spawn the helper),
/// `HelperExit` (helper ran but exited non-zero),
/// `MalformedJson` (helper exited 0 but its stdout
/// is not a valid `DebianIssueReportV1`) — are the
/// three variants. The dispatcher in `main.rs` maps
/// all three to `DebianIssueReportV1 { verdict:
/// InfrastructureError, ... }` (per §24, transport
/// failures are infrastructure errors, not tool
/// failures).
#[derive(Debug, Error)]
pub enum BtsError {
    /// The helper could not be spawned (PATH lookup
    /// failure, permission denied, etc.) or its
    /// stdout / stderr could not be read.
    #[error("BTS helper transport failed: {0}")]
    Transport(#[source] io::Error),
    /// The helper exited with a non-zero exit code.
    /// The `stderr` field carries whatever the helper
    /// wrote to stderr, which is the diagnostic
    /// surface the runtime surfaces.
    #[error("BTS helper exited {code}: {stderr}")]
    HelperExit { code: i32, stderr: String },
    /// The helper exited 0 but its stdout is not a
    /// valid `DebianIssueReportV1`. The §24 "malformed
    /// service response" case.
    #[error("BTS helper produced malformed JSON: {0}")]
    MalformedJson(#[source] serde_json::Error),
}

/// Run the 1E.2 check for the candidate whose
/// `source_name` is `input.package.name`.
///
/// PHASE-1.1 PR 1.1.C (real BTS integration): the
/// pre-1.1.C body returned a hard-coded three-bug
/// fixture regardless of the candidate's source
/// package, so the live BTS transport was never
/// exercised. The new body shells out to the
/// `bts-helper.py` shim (path controlled by the
/// `BTS_HELPER` env var; default: a hermetic
/// Rust-side fallback so the §97 gate does not need
/// the helper to be installed).
///
/// The §23 wire format on stdout is unchanged: the
/// helper emits a `DebianIssueReportV1` and the
/// dispatcher writes that JSON to stdout. The
/// `verdict` tri-state is preserved — a successful
/// subprocess yields `verdict: pass`; a transport /
/// helper-exit / malformed-JSON failure yields
/// `verdict: infrastructure_error` (the dispatcher's
/// error-to-verdict mapping, not the helper's).
pub fn run(input: &CandidateInput) -> Result<crate::report::DebianIssueReportV1, BtsError> {
    // The pre-1.1.C test surface was the in-process
    // hermetic fixture; preserving that path is what
    // keeps the §97 gate hermetic (the gate's tests
    // do not set `BTS_HELPER`). The new test surface
    // (PHASE-1.1 PR 1.1.C) sets `BTS_HELPER` to a
    // fake-bts script and exercises the live path.
    match std::env::var(BTS_HELPER_ENV) {
        Ok(helper_path) if !helper_path.is_empty() => run_live(input, &helper_path),
        _ => Ok(hermetic_report(input)),
    }
}

/// The live path: shell out to the helper, parse its
/// stdout as a `DebianIssueReportV1`, return the
/// report. The helper is invoked as `<helper>
/// <package_name>` (no `--self-test`); the
/// `bts-helper.py` shim supports both modes and
/// defaults to live mode (network). A `BtsError` is
/// returned for any of the three failure modes; the
/// dispatcher in `main.rs` maps the error to a
/// `verdict: infrastructure_error` report.
fn run_live(
    input: &CandidateInput,
    helper_path: &str,
) -> Result<crate::report::DebianIssueReportV1, BtsError> {
    // PHASE-1.1 PR 1.1.I: seal the env boundary. The helper
    // script must not see the ironmaintd daemon's ambient
    // env (`IRONMAINT_*`, `AWS_*`, `LD_*`, proxy vars,
    // ...). Without `env_clear()` every var the parent
    // process was started with leaks through to the
    // `bts-helper.py` subprocess. The 1.1.I fix on the
    // executor side (cmd.env_clear() before per-tool env) is
    // the *outer* boundary — but `ironmaint-debian-tool` is
    // a separate binary spawned by the executor, and the
    // bts-snapshot subcommand shells out to the helper as a
    // *second* subprocess on top of that. The inner spawn
    // here is therefore sealed independently, with the same
    // minimal allow-list the executor uses. The Python
    // helper needs no env to do its job.
    let output = Command::new(helper_path)
        .arg(&input.package.name)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .map_err(BtsError::Transport)?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(BtsError::HelperExit { code, stderr });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: crate::report::DebianIssueReportV1 =
        serde_json::from_str(stdout.as_ref()).map_err(BtsError::MalformedJson)?;
    Ok(report)
}

/// The pre-1.1.C hermetic fixture, preserved as a
/// fallback for the §97 gate (which does not set
/// `BTS_HELPER` and therefore does not need the
/// helper to be installed). The fixture is a
/// three-bug table that mirrors the 1E.1
/// `bts-helper.py --self-test` shape.
fn hermetic_report(input: &CandidateInput) -> crate::report::DebianIssueReportV1 {
    let source_package = input.package.name.clone();
    let bugs = hermetic_bugs();
    let summary = bug_summary(&bugs);
    crate::report::DebianIssueReportV1 {
        schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
        report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
        verdict: IssueVerdict::Pass,
        source_package,
        snapshot_at: HERMETIC_SNAPSHOT_AT.to_string(),
        bugs,
        summary,
    }
}

/// The hermetic fixture's bug list. Three bugs:
/// one open/serious (with a patch), one
/// open/forwarded (upstream), one done. Mirrors
/// the 1E.1 `bts-helper.py` `--self-test` fixture.
fn hermetic_bugs() -> Vec<BugRecord> {
    vec![
        BugRecord {
            id: 1_000_001,
            severity: "serious".to_string(),
            status: "open".to_string(),
            subject: "example: FTBFS on s390x (mock-related)".to_string(),
            tags: vec!["patch".to_string()],
            forwarded: false,
            patch_tagged: true,
        },
        BugRecord {
            id: 1_000_002,
            severity: "normal".to_string(),
            status: "open".to_string(),
            subject: "example: upstream has new release 1.1.0".to_string(),
            tags: vec!["upstream".to_string()],
            forwarded: true,
            patch_tagged: false,
        },
        BugRecord {
            id: 1_000_003,
            severity: "wishlist".to_string(),
            status: "done".to_string(),
            subject: "example: please add a man page".to_string(),
            tags: Vec::new(),
            forwarded: false,
            patch_tagged: false,
        },
    ]
}

/// Compute the §23 summary from a bug list. The
/// summary is `open`, `serious`, `patch_tagged`,
/// `forwarded` counts.
fn bug_summary(bugs: &[BugRecord]) -> BugSummary {
    let mut summary = BugSummary::default();
    for bug in bugs {
        if bug.status == "open" {
            summary.open = summary.open.saturating_add(1);
            if matches!(bug.severity.as_str(), "critical" | "grave" | "serious") {
                summary.serious = summary.serious.saturating_add(1);
            }
        }
        if bug.patch_tagged {
            summary.patch_tagged = summary.patch_tagged.saturating_add(1);
        }
        if bug.forwarded {
            summary.forwarded = summary.forwarded.saturating_add(1);
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{
        BTS_SNAPSHOT_REPORT_KIND, BTS_SNAPSHOT_SCHEMA_VERSION, DebianIssueReportV1, IssueVerdict,
    };

    /// The hermetic fixture: a successful snapshot
    /// (`verdict: pass`) regardless of which source
    /// package the candidate names. §24 says the
    /// snapshot either succeeded or the transport
    /// failed; a populated bug list is *not* a failure.
    ///
    /// The test does *not* set `BTS_HELPER`, so the
    /// `run` function falls through to the
    /// in-process hermetic fixture. The live-path
    /// tests are in `tests/bts_snapshot_live.rs`.
    #[test]
    fn hermetic_fixture_emits_pass_verdict() {
        let input = CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192…".to_string(),
            check_id: "0192…".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "1.0.0-1".to_string(),
            },
        };
        let report = run(&input).expect("hermetic fixture does not error");
        assert_eq!(report.verdict, IssueVerdict::Pass);
        assert_eq!(report.report_kind, BTS_SNAPSHOT_REPORT_KIND);
        assert_eq!(report.schema_version, BTS_SNAPSHOT_SCHEMA_VERSION);
    }

    /// The hermetic fixture's source package is the
    /// candidate's `package.name`. The wire format
    /// surfaces the lookup key so a §23 consumer can
    /// correlate the bug list to the candidate.
    #[test]
    fn hermetic_fixture_populates_source_package_from_candidate() {
        let input = CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192…".to_string(),
            check_id: "0192…".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "1.0.0-1".to_string(),
            },
        };
        let report = run(&input).expect("hermetic fixture does not error");
        assert_eq!(report.source_package, "example");
    }

    /// The hermetic fixture's `snapshot_at` is a
    /// stable RFC3339 UTC constant. The integration
    /// test in `bins/ironmaintd/tests/startup_shutdown.rs`
    /// asserts against this constant.
    #[test]
    fn hermetic_fixture_populates_snapshot_at_with_a_stable_timestamp() {
        let input = CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192…".to_string(),
            check_id: "0192…".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "1.0.0-1".to_string(),
            },
        };
        let report = run(&input).expect("hermetic fixture does not error");
        assert_eq!(report.snapshot_at, HERMETIC_SNAPSHOT_AT);
    }

    /// The hermetic fixture's `bugs` list has 3
    /// entries (mirroring the 1E.1
    /// `bts-helper.py --self-test` shape).
    #[test]
    fn hermetic_fixture_populates_three_bugs() {
        let input = CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192…".to_string(),
            check_id: "0192…".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "1.0.0-1".to_string(),
            },
        };
        let report = run(&input).expect("hermetic fixture does not error");
        assert_eq!(report.bugs.len(), 3);
    }

    /// The hermetic fixture's `summary` reflects the
    /// bug list: 2 open, 1 serious, 1 patch-tagged,
    /// 1 forwarded.
    #[test]
    fn hermetic_fixture_summary_matches_bug_list() {
        let input = CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192…".to_string(),
            check_id: "0192…".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "1.0.0-1".to_string(),
            },
        };
        let report = run(&input).expect("hermetic fixture does not error");
        assert_eq!(report.summary.open, 2);
        assert_eq!(report.summary.serious, 1);
        assert_eq!(report.summary.patch_tagged, 1);
        assert_eq!(report.summary.forwarded, 1);
    }

    /// The struct round-trips through the wire format
    /// without losing data. The 1E.1 `bts-helper.py` shim
    /// produces a JSON document the normalizer parses;
    /// a round-trip through `serde_json` is the
    /// structural-equality test.
    #[test]
    fn comprehensive_report_round_trips_through_json() {
        let report = DebianIssueReportV1 {
            schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
            report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
            verdict: IssueVerdict::Pass,
            source_package: "example".to_string(),
            snapshot_at: HERMETIC_SNAPSHOT_AT.to_string(),
            bugs: Vec::new(),
            summary: BugSummary::default(),
        };
        let json = serde_json::to_string(&report).unwrap();
        let round: DebianIssueReportV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round, report);
    }
}
