//! `ironmaint-debian-tool` — the production Debian inspection
//! tool binary.
//!
//! PHASE-1.md §13 / §16 / §17 / §29. Subcommand enum
//! (1C.1's `source-preparation` is the only real subcommand;
//! the others are 1C.2 / 1C.3 / 1D.2 / 1E.x forward pointers
//! and return a clear "not implemented" with a non-zero
//! exit code):
//!
//!   source-preparation    (1C.1)  — wired in this PR
//!   source-analysis       (1C.2)  — stub
//!   maintenance-context   (1C.3)  — stub
//!   policy-delta          (1D.2)  — stub
//!   bts-snapshot          (1E.x)  — stub
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
    /// The 1E.x forward pointer. Returns exit 1 with
    /// a diagnostic on stderr; 1E.x lands the real
    /// implementation (separate binary, the
    /// `python3-debianbts` shell-out per §27).
    #[command(name = "bts-snapshot")]
    BtsSnapshot,
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
        Sub::SourceAnalysis => not_yet_implemented("source-analysis", "1C.2"),
        Sub::MaintenanceContext => not_yet_implemented("maintenance-context", "1C.3"),
        Sub::PolicyDelta => not_yet_implemented("policy-delta", "1D.2"),
        Sub::BtsSnapshot => not_yet_implemented("bts-snapshot", "1E.x"),
    }
}

/// Emit the "not yet implemented" diagnostic on stderr and
/// return exit 1. The executor's pre-flight `check_launchable`
/// runs even on exit-1; the tool's exit code is what the
/// normalizer + exit-code classification read.
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
