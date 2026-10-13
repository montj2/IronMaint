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

use serde::Deserialize;

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

/// Run the 1E.2 check for the candidate whose
/// `source_name` is `input.package.name`.
///
/// 1E.2 GREEN: emits the comprehensive
/// `DebianIssueReportV1` (PHASE-1.md §23). The
/// `verdict` is `pass` (the hermetic fixture is a
/// successful snapshot); the §23 fields
/// (`source_package`, `snapshot_at`, `bugs`,
/// `summary`) are populated from a small in-memory
/// table that mirrors the 1E.1 `bts-helper.py`
/// `--self-test` fixture.
///
/// The fixture has three bugs: one open/serious,
/// one open/forwarded, one done. The summary is
/// computed from the bug list: `open: 2`, `serious: 1`,
/// `patch_tagged: 1`, `forwarded: 1`.
///
/// Public so `tests/bts_snapshot.rs` and the daemon's
/// integration test can drive the logic without a real
/// subprocess.
#[must_use]
pub fn run(input: &CandidateInput) -> crate::report::DebianIssueReportV1 {
    // The hermetic fixture: a successful snapshot
    // against the candidate's source package. The
    // `verdict` is `pass` regardless of bug count (§24);
    // the 1E.1 shim's `--self-test` fixture has 3 bugs,
    // and the 1E.2 GREEN report carries the same 3
    // bugs in the comprehensive shape.
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
        let report = run(&input);
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
        let report = run(&input);
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
        let report = run(&input);
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
        let report = run(&input);
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
        let report = run(&input);
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
        let input = CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192…".to_string(),
            check_id: "0192…".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "1.0.0-1".to_string(),
            },
        };
        let report = run(&input);
        let json = serde_json::to_string(&report).unwrap();
        let round: DebianIssueReportV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round, report);
    }
}
