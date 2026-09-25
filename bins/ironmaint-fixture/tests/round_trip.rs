//! Round-trip test for the fixture binary.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Stdio;

#[tokio::test]
async fn fixture_emits_record_for_synthetic_build_validate() {
    let bin = env!("CARGO_BIN_EXE_ironmaint-fixture");
    let request = serde_json::json!({
        "tool_key": "synthetic.build.validate",
        "retry_class": "idempotent",
        "input": {},
        "env_overrides": {}
    });
    let mut child = tokio::process::Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .env("LC_ALL", "C.UTF-8")
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .await
        .expect("write stdin");
    drop(child.stdin.take());
    let out = child.wait_with_output().await.expect("wait");
    assert!(out.status.success(), "fixture exited {:?}", out.status);
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse");
    assert_eq!(value["tool_key"], "synthetic.build.validate");
    assert_eq!(value["exit_code"], 0);
    assert!(value["stdout"].as_str().unwrap().contains("BUILD OK"));
}

use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn fixture_emits_nonzero_for_fail_key() {
    let bin = env!("CARGO_BIN_EXE_ironmaint-fixture");
    let request = serde_json::json!({
        "tool_key": "synthetic.build.fail",
        "retry_class": "idempotent",
        "input": {},
        "env_overrides": {}
    });
    let mut child = tokio::process::Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .env("LC_ALL", "C.UTF-8")
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .await
        .expect("write stdin");
    drop(child.stdin.take());
    let out = child.wait_with_output().await.expect("wait");
    assert!(!out.status.success(), "fixture must fail");
    assert_eq!(out.status.code(), Some(1));
}
