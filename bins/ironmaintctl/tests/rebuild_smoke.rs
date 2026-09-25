//! `ironmaintctl rebuild-projections` smoke test:
//! opens a temp state directory, attempts a rebuild against a
//! job that has no events, and asserts the CLI surfaces a
//! "0 rebuilt" report without panicking.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

#[test]
fn rebuild_smoke_no_events() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = Command::new(env!("CARGO_BIN_EXE_ironmaintctl"))
        .args([
            "--state-dir",
            tmp.path().to_str().expect("utf8 path"),
            "rebuild-projections",
        ])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "ironmaintctl rebuild-projections failed: stderr=`{}`",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("rebuild-projections"));
    assert!(stdout.contains("rebuilt: 0 job(s)"));
}
