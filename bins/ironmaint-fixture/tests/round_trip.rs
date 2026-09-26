//! Round-trip tests for the fixture binary.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Stdio;

fn fixture() -> &'static str {
    env!("CARGO_BIN_EXE_ironmaint-fixture")
}

async fn run_with_arg(arg: &str) -> std::process::Output {
    tokio::process::Command::new(fixture())
        .arg(arg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .env("LC_ALL", "C.UTF-8")
        .spawn()
        .expect("spawn")
        .wait_with_output()
        .await
        .expect("wait")
}

#[tokio::test]
async fn fixture_emits_record_for_synthetic_build_validate() {
    let out = run_with_arg("synthetic.build.validate").await;
    assert!(out.status.success(), "fixture exited {:?}", out.status);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("BUILD OK"), "stdout was: {text}");
}

#[tokio::test]
async fn fixture_emits_nonzero_for_fail_key() {
    let out = run_with_arg("synthetic.build.fail").await;
    assert!(!out.status.success(), "fixture must fail");
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("build failed"), "stdout was: {text}");
}

#[tokio::test]
async fn fixture_truncate_emits_256_kib() {
    let out = run_with_arg("synthetic.build.truncate").await;
    assert!(out.status.success(), "fixture exited {:?}", out.status);
    assert_eq!(
        out.stdout.len(),
        256 * 1024,
        "expected exactly 256 KiB on stdout"
    );
}

#[tokio::test]
async fn fixture_infra_fail_exits_127() {
    let out = run_with_arg("synthetic.build.infra_fail").await;
    assert_eq!(out.status.code(), Some(127));
}

#[tokio::test]
async fn fixture_unknown_key_exits_2() {
    let out = run_with_arg("definitely.not.a.known.tool").await;
    assert_eq!(out.status.code(), Some(2));
}
