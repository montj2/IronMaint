//! `ironmaint-debian-tool` — the production Debian inspection
//! tool binary.
//!
//! PHASE-1.md §13 / §16 / §17 / §29. Subcommand enum
//! (`source-preparation`, `source-analysis`,
//! `maintenance-context`, `policy-delta`, and
//! `bts-snapshot` are all wired; no stubs remain):
//!
//!   source-preparation    (1C.1) — wired
//!   source-analysis       (1C.2) — wired
//!   maintenance-context   (1C.3) — wired
//!   policy-delta          (1D.2) — wired
//!   bts-snapshot          (1E.2) — wired
//!
//! Wire format per subcommand is the same:
//!
//! - Read `ironmaint.candidate_input.v2` from stdin
//!   (one document, EOF-terminated, deserialised into
//!   the subcommand's input struct).
//! - Write the versioned JSON report to stdout (one
//!   document, EOF-terminated).
//! - Diagnostics and warnings go to stderr.
//! - Exit code is 0 on a normal report emission (the
//!   report's `verdict` carries Pass / Fail /
//!   InfrastructureError; the exit code is not the
//!   verdict). Non-zero exit is reserved for "the
//!   tool itself could not run" (e.g. malformed
//!   input, IO error reading stdin).

use std::io::{self, Read, Write};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;

use ironmaint_debian_tool::bts_snapshot::{self as bts_snapshot_run, CandidateInput as BtsInput};
use ironmaint_debian_tool::maintenance_context::{
    self as maintenance_context_run, CandidateInput as MaintenanceContextInput,
};
use ironmaint_debian_tool::policy_delta::{
    self as policy_delta_run, CandidateInput as PolicyDeltaInput,
};
use ironmaint_debian_tool::report::{
    BTS_SNAPSHOT_REPORT_KIND, BTS_SNAPSHOT_SCHEMA_VERSION, BugSummary, DebianBuildReportV1,
    IssueVerdict,
};
use ironmaint_debian_tool::source_analysis::{
    self as source_analysis_run, CandidateInput as AnalysisInput,
};
use ironmaint_debian_tool::source_preparation::{self, CandidateInput};

/// CLI entry point. The clap derive is sufficient — the
/// tool is invoked by the executor with one positional
/// subcommand, and the executor passes any other args
/// (e.g. `--workspace-root` for hermetic test harnesses)
/// via `fixed_args`.
#[derive(Debug, Parser)]
#[command(
    name = "ironmaint-debian-tool",
    version,
    about = "IronMaint real Debian inspection tool (PHASE-1 §13, §16, §17, §29)."
)]
struct Cli {
    #[command(subcommand)]
    subcommand: Sub,
}

/// The dispatch enum. The subcommand names are exactly the
/// `fixed_args` strings `bins/ironmaintd/src/main.rs` passes
/// when registering the `ToolDefinitionRecord` — the two
/// sides have to agree on the literal string.
#[derive(Debug, Subcommand)]
enum Sub {
    /// The 1C.1 real implementation. Reads
    /// `ironmaint.candidate_input.v2` on stdin, emits
    /// `DebianSourcePreparationV1` on stdout.
    #[command(name = "source-preparation")]
    SourcePreparation,
    /// The 1C.2 forward pointer. Returns exit 1 with
    /// a diagnostic on stderr; 1C.2 lands the real
    /// implementation.
    #[command(name = "source-analysis")]
    SourceAnalysis,
    /// The 1C.3 forward pointer. Returns exit 1 with
    /// a diagnostic on stderr; 1C.3 lands the real
    /// implementation.
    #[command(name = "maintenance-context")]
    MaintenanceContext,
    /// The 1D.2 forward pointer. Returns exit 1 with
    /// a diagnostic on stderr; 1D.2 lands the real
    /// implementation.
    #[command(name = "policy-delta")]
    PolicyDelta,
    /// The 1E.2 real implementation. Reads
    /// `ironmaint.candidate_input.v3` on stdin, emits
    /// `DebianIssueReportV1` on stdout. The 1E.1
    /// `bts-helper.py` shim is the read-only transport;
    /// this subcommand is the consumer (the hermetic
    /// mode emits a hard-coded fixture; the live mode
    /// would shell out to the shim, opt-in only — not
    /// in the §97 gate).
    #[command(name = "bts-snapshot")]
    BtsSnapshot,
    /// PHASE-2 PR 2A.1. The 2A.1 build subcommand.
    /// The body is a stub that emits a hard-coded
    /// `DebianBuildReportV1 { outcome: Pass, .. }`.
    /// 2A.3 replaces this body with a real `sbuild`
    /// invocation. The subcommand literal `"build"`
    /// and the `BUILD_*` env-var contract (read but
    /// not yet consumed by the stub) are what 2A.3
    /// must honour.
    #[command(name = "build")]
    Build,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.subcommand {
        Sub::SourcePreparation => match run_source_preparation() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                // The `Err` arm is reserved for "the tool
                // itself could not run" — malformed input,
                // IO error on stdin, etc. The verdict
                // tri-state is inside the report; the
                // `Verdict::InfrastructureError` case is a
                // valid *report*, not a tool failure.
                eprintln!("ironmaint-debian-tool: source-preparation: {err:#}");
                ExitCode::FAILURE
            }
        },
        Sub::SourceAnalysis => match run_source_analysis() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ironmaint-debian-tool: source-analysis: {err:#}");
                ExitCode::FAILURE
            }
        },
        Sub::MaintenanceContext => match run_maintenance_context_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ironmaint-debian-tool: maintenance-context: {err:#}");
                ExitCode::FAILURE
            }
        },
        Sub::PolicyDelta => match run_policy_delta_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ironmaint-debian-tool: policy-delta: {err:#}");
                ExitCode::FAILURE
            }
        },
        Sub::BtsSnapshot => match run_bts_snapshot_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ironmaint-debian-tool: bts-snapshot: {err:#}");
                ExitCode::FAILURE
            }
        },
        Sub::Build => match run_build_cmd() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ironmaint-debian-tool: build: {err:#}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Emit the "not yet implemented" diagnostic on stderr and
/// return exit 1. The executor's pre-flight `check_launchable`
/// runs even on exit-1; the tool's exit code is what the
/// normalizer + exit-code classification read.
#[allow(dead_code)]
fn not_yet_implemented(subcommand: &str, pr: &str) -> ExitCode {
    eprintln!("ironmaint-debian-tool: {subcommand} is not yet implemented; will land in PR {pr}.");
    ExitCode::FAILURE
}

/// Run the `source-preparation` subcommand. Reads
/// `ironmaint.candidate_input.v2` from stdin, walks the
/// source tree, emits `DebianSourcePreparationV1` on stdout.
fn run_source_preparation() -> Result<()> {
    let input: CandidateInput =
        read_stdin_json().context("reading ironmaint.candidate_input.v2 from stdin")?;
    let report = source_preparation::run(&input);
    write_stdout_json(&report).context("writing DebianSourcePreparationV1 to stdout")?;
    Ok(())
}

/// Run the `source-analysis` subcommand. Reads
/// `ironmaint.candidate_input.v3` from stdin, walks the
/// source tree, emits the comprehensive
/// `DebianSourceReportV1` on stdout.
fn run_source_analysis() -> Result<()> {
    let input: AnalysisInput =
        read_stdin_json().context("reading ironmaint.candidate_input.v3 from stdin")?;
    let report = source_analysis_run::run(&input);
    write_stdout_json(&report).context("writing DebianSourceReportV1 to stdout")?;
    Ok(())
}

/// Run the `maintenance-context` subcommand. Reads
/// `ironmaint.candidate_input.v3` from stdin (no schema
/// bump from 1C.2), walks the source tree, emits
/// `DebianMaintenanceContextV1` on stdout.
fn run_maintenance_context_cmd() -> Result<()> {
    let input: MaintenanceContextInput =
        read_stdin_json().context("reading ironmaint.candidate_input.v3 from stdin")?;
    let report = maintenance_context_run::run(&input);
    write_stdout_json(&report).context("writing DebianMaintenanceContextV1 to stdout")?;
    Ok(())
}

/// Run the `policy-delta` subcommand. Reads
/// `ironmaint.candidate_input.v3` from stdin (no schema
/// bump from 1C.2 / 1C.3), walks the source tree,
/// emits `DebianPolicyDeltaReviewV1` on stdout.
fn run_policy_delta_cmd() -> Result<()> {
    let input: PolicyDeltaInput =
        read_stdin_json().context("reading ironmaint.candidate_input.v3 from stdin")?;
    let report = policy_delta_run::run(&input);
    write_stdout_json(&report).context("writing DebianPolicyDeltaReviewV1 to stdout")?;
    Ok(())
}

/// Run the `bts-snapshot` subcommand. Reads
/// `ironmaint.candidate_input.v3` from stdin (no schema
/// bump from 1C.2 / 1C.3 / 1D.2), emits
/// `DebianIssueReportV1` on stdout.
///
/// PHASE-1.1 PR 1.1.C (real BTS integration): the
/// pre-1.1.C body always returned a hard-coded
/// three-bug fixture regardless of the candidate's
/// source package, so the live BTS transport was never
/// exercised. The new body shells out to the
/// `bts-helper.py` shim (path controlled by the
/// `BTS_HELPER` env var; default: a hermetic Rust-side
/// fallback so the §97 gate does not need the helper
/// to be installed).
///
/// A `BtsError` (transport / helper-exit / malformed
/// JSON) is converted to a `verdict:
/// infrastructure_error` report. The §24 "transport
/// failures are infrastructure errors, not tool
/// failures" mapping is preserved: the binary exits 0
/// (it has produced a valid report) and the
/// `verdict: infrastructure_error` field is what the
/// normalizer reads.
fn run_bts_snapshot_cmd() -> Result<()> {
    let input: BtsInput =
        read_stdin_json().context("reading ironmaint.candidate_input.v3 from stdin")?;
    let source_package = input.package.name.clone();
    let report = match bts_snapshot_run::run(&input) {
        Ok(report) => report,
        Err(err) => {
            // The helper could not be spawned / exited
            // non-zero / emitted malformed JSON. Emit a
            // valid `DebianIssueReportV1` with
            // `verdict: infrastructure_error` so the
            // normalizer can map it to `Unclassifiable`
            // (→ `ExecutorError::InfrastructureFailed` →
            // `GateStatus::Blocked` per §24). The
            // diagnostic message is the source of
            // truth for the runtime; the exit code
            // remains 0 (a valid report was emitted).
            eprintln!("ironmaint-debian-tool: bts-snapshot: {err}");
            ironmaint_debian_tool::report::DebianIssueReportV1 {
                schema_version: BTS_SNAPSHOT_SCHEMA_VERSION,
                report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
                verdict: IssueVerdict::InfrastructureError,
                source_package,
                snapshot_at: String::new(),
                bugs: Vec::new(),
                summary: BugSummary::default(),
            }
        }
    };
    write_stdout_json(&report).context("writing DebianIssueReportV1 to stdout")?;
    Ok(())
}

/// Run the `build` subcommand (PHASE-2 PR 2A.1).
///
/// 2A.1's body is a stub: it reads the v3 candidate
/// input from stdin, reads the `BUILD_*` env vars (which
/// the runtime will inject per-call in 2A.2), and emits
/// a hard-coded `DebianBuildReportV1 { outcome: Pass, .. }`.
/// 2A.3 replaces the body with a real `sbuild`
/// invocation. The subcommand literal `"build"` and
/// the env-var contract are what 2A.3 must honour.
///
/// The env vars read (but currently unused by the stub):
///
/// - `BUILD_INPUT_DIR` — the staged source root.
/// - `BUILD_OUTPUT_DIR` — the sibling output dir.
/// - `BUILD_TARGET_SUITE` — the build target's suite.
/// - `BUILD_TARGET_ARCH` — the build target's arch.
/// - `BUILD_FINGERPRINT` — the candidate's BLAKE3 fingerprint.
/// - `BUILD_COMMIT` — the candidate's Git commit OID.
/// - `BUILD_TREE_SHA` — the candidate's Git tree OID.
fn run_build_cmd() -> Result<()> {
    // The 2A.1 stub reads (and discards) the candidate
    // input so the executor's `JsonStdin` wiring is
    // exercised end-to-end. 2A.3 will parse the input
    // and use it to seed the sbuild invocation.
    let _input: BtsInput =
        read_stdin_json().context("reading ironmaint.candidate_input.v3 from stdin")?;

    // Touch each `BUILD_*` env var so the contract is
    // exercised. The stub does not use the values; 2A.3
    // populates the report's `provenance`, `target`,
    // `source_package`, `started_at`, and `finished_at`
    // from these.
    for key in [
        "BUILD_INPUT_DIR",
        "BUILD_OUTPUT_DIR",
        "BUILD_TARGET_SUITE",
        "BUILD_TARGET_ARCH",
        "BUILD_FINGERPRINT",
        "BUILD_COMMIT",
        "BUILD_TREE_SHA",
    ] {
        let _ = std::env::var(key);
    }

    // The 2A.1 stub emits a hard-coded pass report. 2A.3
    // replaces this with a real sbuild invocation and
    // populates every field from real sbuild output.
    let report = DebianBuildReportV1 {
        schema_version: ironmaint_debian_tool::report::BUILD_REPORT_SCHEMA_VERSION,
        report_kind: ironmaint_debian_tool::report::BUILD_REPORT_KIND.to_string(),
        outcome: ironmaint_debian_tool::report::BuildOutcome::Pass,
        producer: ironmaint_debian_tool::report::BuildProducer {
            name: "ironmaint-debian-tool".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        },
        exit_code: 0,
        ..Default::default()
    };
    write_stdout_json(&report).context("writing DebianBuildReportV1 to stdout")?;
    Ok(())
}

/// Read one JSON document from stdin. The input is a
/// single object terminated by EOF (no trailing-newline
/// requirement; serde_json tolerates both).
fn read_stdin_json<T: for<'de> Deserialize<'de>>() -> Result<T> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    serde_json::from_str(&buf).context("parsing stdin JSON")
}

/// Write one JSON document to stdout. Pretty-printed so
/// the §30 oracle diff is human-readable; the §97
/// snapshots are exact-byte stable.
fn write_stdout_json<T: serde::Serialize>(value: &T) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value).context("serialising JSON to stdout")?;
    out.write_all(b"\n").context("writing trailing newline")?;
    out.flush().context("flushing stdout")?;
    Ok(())
}

// ---------------------------------------------------------------------
// Unit tests for the binary entry point. The actual check logic is
// tested in `source_preparation::tests`; these tests pin the
// subcommand dispatch and the wire-format conventions.
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_debian_tool::report::{
        DebianSourcePreparationV1, Diagnostic, REPORT_KIND, SCHEMA_VERSION,
        SourcePreparationFindings, Verdict,
    };

    /// The subcommand enum's clap derive must produce
    /// exactly the literal strings the daemon's
    /// `fixed_args` expects. A typo here is a silent
    /// wiring failure: the tool registers correctly, the
    /// executor spawns the binary, but the binary's
    /// `clap` parser fails to find the subcommand and
    /// exits 2 with "unrecognised subcommand" on stderr.
    #[test]
    fn subcommand_names_match_the_daemon_fixed_args() {
        // The clap derive is type-erased at runtime, so
        // the only way to assert this is via a parse round-trip.
        // `Cli::try_parse_from(["ironmaint-debian-tool", "source-preparation"])`
        // would return `Err` for the wrong name.
        let cli = Cli::try_parse_from(["ironmaint-debian-tool", "source-preparation"]).unwrap();
        assert!(matches!(cli.subcommand, Sub::SourcePreparation));

        let cli = Cli::try_parse_from(["ironmaint-debian-tool", "source-analysis"]).unwrap();
        assert!(matches!(cli.subcommand, Sub::SourceAnalysis));

        let cli = Cli::try_parse_from(["ironmaint-debian-tool", "maintenance-context"]).unwrap();
        assert!(matches!(cli.subcommand, Sub::MaintenanceContext));

        let cli = Cli::try_parse_from(["ironmaint-debian-tool", "policy-delta"]).unwrap();
        assert!(matches!(cli.subcommand, Sub::PolicyDelta));

        let cli = Cli::try_parse_from(["ironmaint-debian-tool", "bts-snapshot"]).unwrap();
        assert!(matches!(cli.subcommand, Sub::BtsSnapshot));
    }

    /// The subcommand's report's `report_kind` discriminator
    /// is the literal string the verifier (and the 1A.4
    /// `evidence.artifact.read` tool) reads to know which
    /// schema to apply. A drift here is a wire-format
    /// regression: the report would still serialise, but
    /// downstream readers would reject the discriminator.
    #[test]
    fn report_kind_is_stable() {
        // The constant lives in the lib; this test is
        // a no-op at the binary level. The cross-binary
        // assertion is the `cargo run -p xtask -- verify-schemas`
        // snapshot.
        assert_eq!(REPORT_KIND, "debian.source_preparation");
        assert_eq!(SCHEMA_VERSION, 1);
    }

    /// A round-trip through the wire format: serialise a
    /// `DebianSourcePreparationV1`, deserialise it back,
    /// and assert the result is structurally equal. This
    /// is the test the §30 oracle uses to compare against
    /// `dpkg-parsechangelog`'s output.
    #[test]
    fn report_round_trips_through_json() {
        let report = DebianSourcePreparationV1 {
            schema_version: SCHEMA_VERSION,
            report_kind: REPORT_KIND.to_string(),
            candidate_fingerprint: "blake3:test".to_string(),
            verdict: Verdict::Pass,
            diagnostics: vec![Diagnostic {
                code: "E_TEST".to_string(),
                message: "test diagnostic".to_string(),
                path: None,
            }],
            findings: SourcePreparationFindings {
                debian_directory_present: true,
                control_parsable: true,
                control_source_name: Some("example".to_string()),
                control_source_name_matches_candidate: true,
                changelog_parsable: true,
                changelog_source_name: Some("example".to_string()),
                changelog_source_name_matches_candidate: true,
                changelog_version: Some("1.0.0-1".to_string()),
                changelog_version_matches_candidate: true,
                format_identifiable: true,
                source_format: Some("3.0 (quilt)".to_string()),
                rules_executable: true,
            },
            identity: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        let round: DebianSourcePreparationV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round, report);
    }
}
