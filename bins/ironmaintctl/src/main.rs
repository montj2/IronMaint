//! `ironmaintctl` — operator CLI for IronMaint.
//!
//! Phase 0B ships a single subcommand, `rebuild-projections`
//! (PHASE-0B.md §36). The runtime command surface lands in 0B.13.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::ExitCode;

use ironmaint_core::JobId;

mod rebuild;

#[derive(Debug)]
struct Args {
    state_dir: Option<PathBuf>,
    sub: Option<Subcommand>,
}

#[derive(Debug)]
enum Subcommand {
    RebuildProjections { job: Option<JobId> },
    Help,
}

const HELP: &str = "\
ironmaintctl — operator CLI for IronMaint (Phase 0B)

USAGE:
    ironmaintctl --state-dir <PATH> <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    rebuild-projections [--job <ID>]
                          Rebuild every projection (or one job) by
                          replaying the SQLite event log.
    -h, --help           Print this help.
";

fn parse_args() -> Result<Args, String> {
    let mut raw = std::env::args().skip(1);
    let mut state_dir: Option<PathBuf> = None;
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "--state-dir" => {
                state_dir = Some(PathBuf::from(
                    raw.next()
                        .ok_or_else(|| "--state-dir requires a path".to_string())?,
                ));
            }
            s if s.starts_with("--state-dir=") => {
                state_dir = Some(PathBuf::from(&s["--state-dir=".len()..]));
            }
            "-h" | "--help" => {
                return Ok(Args {
                    state_dir,
                    sub: Some(Subcommand::Help),
                });
            }
            "rebuild-projections" => {
                let mut job: Option<JobId> = None;
                while let Some(flag) = raw.next() {
                    match flag.as_str() {
                        "--job" => {
                            let id = raw
                                .next()
                                .ok_or_else(|| "--job requires a value".to_string())?;
                            job = Some(
                                id.parse::<JobId>()
                                    .map_err(|e| format!("invalid job id `{id}`: {e}"))?,
                            );
                        }
                        other => return Err(format!("unexpected flag `{other}`")),
                    }
                }
                return Ok(Args {
                    state_dir,
                    sub: Some(Subcommand::RebuildProjections { job }),
                });
            }
            other => return Err(format!("unknown subcommand or flag `{other}`")),
        }
    }
    Ok(Args {
        state_dir,
        sub: None,
    })
}

fn run() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("\n{HELP}");
            return ExitCode::from(2);
        }
    };
    let sub = match args.sub {
        Some(s) => s,
        None => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
    };
    let state_dir = match args.state_dir {
        Some(d) => d,
        None => {
            eprintln!("error: --state-dir is required");
            eprintln!("\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match sub {
        Subcommand::Help => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Subcommand::RebuildProjections { job } => {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("error: could not start tokio runtime: {e}");
                    return ExitCode::FAILURE;
                }
            };
            runtime.block_on(async move {
                match rebuild::run(&state_dir, job).await {
                    Ok(report) => {
                        println!("{report}");
                        ExitCode::SUCCESS
                    }
                    Err(err) => {
                        eprintln!("rebuild-projections failed: {err}");
                        ExitCode::FAILURE
                    }
                }
            })
        }
    }
}

fn main() -> ExitCode {
    run()
}
