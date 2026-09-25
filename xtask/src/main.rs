//! `cargo xtask` — workspace automation for IronMaint.
//!
//! Subcommands:
//! - `verify-architecture`: walk the workspace and assert the Phase 0A
//!   dependency graph (PHASE-0A.md §5, §71).
//! - `verify-schemas`: regenerate JSON schemas and diff against committed
//!   snapshots in `schemas/` (PHASE-0A.md §61).
//!
//! Phase 0A.1 ships the skeleton only; both subcommands are no-ops that
//! confirm they wire up correctly. They become real in 0A.5 / 0A.6.

#![forbid(unsafe_code)]

use std::process::ExitCode;

mod architecture;
mod schemas;

enum Command {
    VerifyArchitecture,
    VerifySchemas { write: bool },
    Help,
}

fn parse_args() -> Command {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("verify-architecture") => Command::VerifyArchitecture,
        Some("verify-schemas") => {
            let write = args.any(|a| a == "--write" || a == "-w");
            Command::VerifySchemas { write }
        }
        Some("--help") | Some("-h") | None => Command::Help,
        Some(other) => {
            eprintln!("error: unknown subcommand `{other}`");
            eprintln!("hint: try `cargo xtask --help`");
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "cargo xtask — IronMaint workspace automation\n\n\
         USAGE:\n    \
             cargo xtask <SUBCOMMAND>\n\n\
         SUBCOMMANDS:\n    \
             verify-architecture    Assert Phase 0A dependency graph.\n    \
             verify-schemas [--write]\n                              \
                                  Regenerate JSON schemas; with --write, write\n                                  \
                                  them under doc/schemas/.\n    \
             -h, --help             Print this help.\n"
    );
}

fn main() -> ExitCode {
    match parse_args() {
        Command::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Command::VerifyArchitecture => match architecture::run() {
            Ok(report) => {
                println!("{report}");
                if report.is_clean() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(err) => {
                eprintln!("verify-architecture failed: {err}");
                ExitCode::FAILURE
            }
        },
        Command::VerifySchemas { write } => match schemas::run(write) {
            Ok(report) => {
                println!("{report}");
                if report.is_clean() {
                    ExitCode::SUCCESS
                } else {
                    eprintln!(
                        "verify-schemas: {} mismatch(es); run with --write to accept.",
                        report.mismatches.len()
                    );
                    ExitCode::FAILURE
                }
            }
            Err(err) => {
                eprintln!("verify-schemas failed: {err}");
                ExitCode::FAILURE
            }
        },
    }
}
