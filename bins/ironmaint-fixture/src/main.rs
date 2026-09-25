//! `ironmaint-fixture` — synthetic tool subprocess.
//!
//! This binary is a control-flow shell, not domain code: the
//! panic-policy lints don't apply.

#![allow(clippy::expect_used, clippy::unwrap_used)]
//!
//! Reads an `ExecutionRequest` JSON from stdin, writes an
//! `ExecutionRecord` JSON to stdout. Behaviour is keyed off the
//! request's `tool_key`:
//!
//! - `synthetic.build.validate`: emits `exit_code=0`,
//!   stdout `BUILD OK`, stderr ``.
//! - `synthetic.qa.lintian`: emits `exit_code=0`, stdout
//!   `lintian: 0 warnings`.
//! - `synthetic.build.fail`: emits `exit_code=1` so the
//!   runtime can exercise the failure path.
//! - Any other tool: emits `exit_code=2` with stderr
//!   `unknown synthetic tool`.
//!
//! The fixture is what the §101 E2E scenario calls; the
//! `ToolRegistry` registration and `ResultNormalizer` for it
//! live in `ironmaint-executor` so they can be reused without
//! depending on this binary.

use std::io::{Read, Write};
use std::process::ExitCode;

use ironmaint_executor::{ExecutionRecord, RetryClass};
use time::OffsetDateTime;

fn main() -> ExitCode {
    let mut buf = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
        eprintln!("fixture: read stdin: {e}");
        return ExitCode::from(3);
    }
    let request: serde_json::Value = match serde_json::from_str(&buf) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("fixture: parse request: {e}");
            return ExitCode::from(3);
        }
    };
    let tool_key = request
        .get("tool_key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let retry_class = request
        .get("retry_class")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_retry_class)
        .unwrap_or(RetryClass::Idempotent);

    let now = OffsetDateTime::now_utc();
    let (exit_code, stdout, stderr) = match tool_key {
        "synthetic.build.validate" => (0, "BUILD OK\n".to_string(), String::new()),
        "synthetic.qa.lintian" => (0, "lintian: 0 warnings\n".to_string(), String::new()),
        "synthetic.build.fail" => (1, "build failed: missing dep\n".to_string(), String::new()),
        "synthetic.qa.fail" => (1, "lintian: E: syntax-error\n".to_string(), String::new()),
        other => (
            2,
            String::new(),
            format!("unknown synthetic tool: {other}\n"),
        ),
    };

    let record = ExecutionRecord {
        tool_key: ironmaint_adapter_api::ToolCapabilityKey::new(tool_key).unwrap_or_else(|_| {
            ironmaint_adapter_api::ToolCapabilityKey::new("synthetic.unknown").unwrap()
        }),
        retry_class,
        started_at: now,
        finished_at: now,
        exit_code,
        stdout,
        stderr,
        retries_exhausted: false,
    };

    let line = serde_json::to_string(&record).expect("serialize record");
    if let Err(e) = std::io::stdout().write_all(line.as_bytes()) {
        eprintln!("fixture: write stdout: {e}");
        return ExitCode::from(3);
    }
    if let Err(e) = std::io::stdout().write_all(b"\n") {
        eprintln!("fixture: write newline: {e}");
        return ExitCode::from(3);
    }
    ExitCode::from(exit_code as u8)
}

fn parse_retry_class(s: &str) -> Option<RetryClass> {
    match s {
        "idempotent" => Some(RetryClass::Idempotent),
        "side_effecting" => Some(RetryClass::SideEffecting),
        "destructive" => Some(RetryClass::Destructive),
        _ => None,
    }
}
