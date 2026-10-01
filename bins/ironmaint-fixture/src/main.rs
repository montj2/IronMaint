//! `ironmaint-fixture` — synthetic tool subprocess.
//!
//! This binary is a control-flow shell, not domain code: the
//! panic-policy lints don't apply.

#![allow(clippy::expect_used, clippy::unwrap_used)]
//!
//! Behaviour is keyed off the first positional argv (the
//! tool_key). The binary does not read stdin — `ProcessExecutor`
//! invokes it as a normal subprocess (`executable() +
//! fixed_args()`); the registered tool's `fixed_args` carries the
//! key.
//!
//! Supported keys:
//!
//! - `synthetic.build.validate`: exit 0, stdout `BUILD OK\n`.
//! - `synthetic.qa.lintian`: exit 0, stdout `lintian: 0 warnings\n`.
//! - `synthetic.build.fail`: exit 1, stdout `build failed: missing dep\n`.
//! - `synthetic.qa.fail`: exit 1, stdout `lintian: E: syntax-error\n`.
//! - `synthetic.policy.validate`: exit 0, stdout `policy: no violations\n`.
//! - `synthetic.policy.fail`: exit 1, stdout `policy: E: source-not-maintained\n`.
//! - `synthetic.build.truncate`: exit 0, stdout 256 KiB of `A`.
//! - `synthetic.build.timeout`: sleeps 120s, then exit 0. The
//!   executor's `tokio::time::timeout` is expected to kill it
//!   well before then.
//! - `synthetic.build.interrupt`: raises `SIGINT` (exit 130).
//! - `synthetic.build.infra_fail`: exit 127 (POSIX "command not
//!   found" — mapped by `outcome_from_record` to
//!   `InfrastructureFailed`).
//!
//! Any other key: exit 2, stderr `unknown synthetic tool: <key>`.
//!
//! The fixture is what the §101 E2E scenario calls; the
//! `ToolRegistry` registration and `ResultNormalizer` for it
//! live in `ironmaint-executor` so they can be reused without
//! depending on this binary.

use std::io::Write;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

const TRUNCATE_BYTES: usize = 256 * 1024;
const TIMEOUT_SLEEP: Duration = Duration::from_secs(120);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let tool_key = args.get(1).map(String::as_str).unwrap_or("");

    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();

    match tool_key {
        "synthetic.build.validate" => {
            let _ = writeln!(stdout, "BUILD OK");
            ExitCode::from(0)
        }
        "synthetic.qa.lintian" => {
            let _ = writeln!(stdout, "lintian: 0 warnings");
            ExitCode::from(0)
        }
        "synthetic.build.fail" => {
            let _ = writeln!(stdout, "build failed: missing dep");
            ExitCode::from(1)
        }
        "synthetic.qa.fail" => {
            let _ = writeln!(stdout, "lintian: E: syntax-error");
            ExitCode::from(1)
        }
        "synthetic.policy.validate" => {
            let _ = writeln!(stdout, "policy: no violations");
            ExitCode::from(0)
        }
        "synthetic.policy.fail" => {
            let _ = writeln!(stdout, "policy: E: source-not-maintained");
            ExitCode::from(1)
        }
        "synthetic.build.truncate" => {
            let chunk = [b'A'; 4096];
            let mut written = 0usize;
            while written < TRUNCATE_BYTES {
                let n = chunk.len().min(TRUNCATE_BYTES - written);
                if stdout.write_all(&chunk[..n]).is_err() {
                    break;
                }
                written += n;
            }
            let _ = stdout.flush();
            ExitCode::from(0)
        }
        "synthetic.build.timeout" => {
            // Sleep longer than any reasonable timeout. The
            // executor kills us before we wake up; the OS still
            // returns 0 if we ever do.
            thread::sleep(TIMEOUT_SLEEP);
            ExitCode::from(0)
        }
        "synthetic.build.interrupt" => {
            // POSIX SIGINT exit convention: 128 + signal = 130.
            // Whether we actually receive SIGINT or simply exit
            // with that status, the executor sees exit_code 130
            // and `outcome_from_record` maps it to
            // `Outcome::Interrupted`. The workspace forbids
            // `unsafe`, so we use `process::exit(130)` rather
            // than `libc::raise(SIGINT)`.
            ExitCode::from(130)
        }
        "synthetic.build.infra_fail" => ExitCode::from(127),
        other => {
            let _ = writeln!(stderr, "unknown synthetic tool: {other}");
            ExitCode::from(2)
        }
    }
}
