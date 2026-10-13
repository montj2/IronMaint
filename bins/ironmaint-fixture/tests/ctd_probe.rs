//! Probe binary target for `cargo_target_dir_overrides_the_workspace_default`.
//!
//! This is not a test of anything in its own right — it exists so the
//! resolver's unit test can start a *child* `cargo test` with
//! `CARGO_TARGET_DIR` set and read back what the child resolved.
//! Process-wide environment mutation is not safe inside a test binary,
//! so the assertion has to happen out-of-process.
//!
//! The parent sets `CARGO_TARGET_DIR` to a path outside the workspace
//! and greps this file's output for `PROBE_TARGET_DIR=<that path>`.
//! If cargo stops re-exporting the variable to test processes, the
//! child's resolver falls back to `<workspace>/target`, the printed
//! value does not match, and the parent fails.
//!
//! **This calls [`ironmaint_fixture::target_dir`] rather than
//! recomputing it.** The first version of this file reimplemented the
//! resolution, and the teeth-check caught exactly what that predicts:
//! breaking the real `target_dir()` left the test green, because the
//! test was asserting on its own copy. A copy is the defect this crate
//! exists to end.

#[test]
fn probe_prints_the_target_dir_it_resolved() {
    let dir = ironmaint_fixture::target_dir();
    println!("PROBE_TARGET_DIR={}", dir.display());
}
