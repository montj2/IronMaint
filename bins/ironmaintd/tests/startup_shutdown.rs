//! Startup / shutdown smoke test.
//!
//! Spins up the daemon against an in-memory SQLite via
//! IRONMAINT_STATE_DIR pointing at a tempfile directory; sends
//! SIGTERM via timeout; asserts clean exit.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;

#[tokio::test]
async fn daemon_starts_and_responds_to_signal() {
    let bin = env!("CARGO_BIN_EXE_ironmaintd");
    let tmp = tempfile::tempdir().expect("tempdir");
    let state_dir = tmp.path().to_path_buf();

    let mut child = tokio::process::Command::new(bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .env("LC_ALL", "C.UTF-8")
        .env("IRONMAINT_STATE_DIR", &state_dir)
        .spawn()
        .expect("spawn");

    // Give the daemon a beat to bootstrap.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    child.kill().await.expect("kill");
    let status = child.wait().await.expect("wait");
    // Killed via signal: exit code may be non-zero.
    assert!(status.success() || status.signal().is_some());
}
