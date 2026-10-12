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

use crate::report::{BTS_SNAPSHOT_REPORT_KIND, BTS_SNAPSHOT_SCHEMA_VERSION, IssueVerdict};

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

/// Run the 1E.2 check for the candidate whose
/// `source_name` is `input.package.name`.
///
/// 1E.2 RED: emits a minimal `DebianIssueReportV1` with
/// only `schema_version`, `report_kind`, and `verdict`.
/// The `verdict` is `pass` (the hermetic fixture is
/// a successful snapshot). The comprehensive §23
/// fields (`source_package`, `snapshot_at`, `bugs`,
/// `summary`) are added in the GREEN commit.
///
/// Public so `tests/bts_snapshot.rs` and the daemon's
/// integration test can drive the logic without a real
/// subprocess.
#[must_use]
pub fn run(input: &CandidateInput) -> crate::report::DebianIssueReportV1 {
    // The hermetic fixture: a successful snapshot. The
    // `verdict` is `pass` regardless of bug count (§24);
    // the 1E.1 shim's `--self-test` fixture has 3 bugs
    // (one open/serious, one open/forwarded, one done),
    // but the 1E.2 RED report is the *minimal* shape
    // (no bug list). The GREEN commit adds the
    // comprehensive shape.
    let _ = input.package.name.as_str();
    crate::report::DebianIssueReportV1 {
        schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
        report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
        verdict: IssueVerdict::Pass,
    }
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

    /// The struct round-trips through the wire format
    /// without losing data. The 1E.1 `bts-helper.py` shim
    /// produces a JSON document the normalizer parses;
    /// a round-trip through `serde_json` is the
    /// structural-equality test.
    #[test]
    fn minimal_report_round_trips_through_json() {
        let report = DebianIssueReportV1 {
            schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
            report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
            verdict: IssueVerdict::Pass,
        };
        let json = serde_json::to_string(&report).unwrap();
        let round: DebianIssueReportV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round, report);
    }
}
