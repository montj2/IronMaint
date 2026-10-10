//! `ironmaint-debian-tool` library surface.
//!
//! The crate has two targets: a `[[bin]]` (`ironmaint-debian-tool`,
//! the production tool the executor spawns) and a `[lib]` (this
//! file). The lib is what the daemon and the integration tests
//! import:
//!
//! - [`DebianSourcePreparationNormalizer`][]: the
//!   `ironmaint_executor::ResultNormalizer` that classifies the
//!   tool's stdout.
//! - [`binary_path`]: a test helper that returns the absolute
//!   path to the built binary, mirroring the
//!   `ironmaint_fixture::fixture_binary_path()` pattern.
//! - The `report` and `source_preparation` modules are re-exported
//!   so the normalizer (which lives in a separate module) can
//!   refer to them through the crate root.

use std::path::PathBuf;

pub mod normalizer;
pub mod report;
pub mod source_analysis;
pub mod source_preparation;

pub use normalizer::{DebianSourceAnalysisNormalizer, DebianSourcePreparationNormalizer};
pub use report::{DebianSourcePreparationV1, DebianSourceReportV1, Verdict};
pub use source_analysis::run as run_source_analysis;
pub use source_preparation::run as run_source_preparation;

const BINARY_NAME: &str = "ironmaint-debian-tool";

/// Return the absolute path to the built `ironmaint-debian-tool`
/// binary, mirroring the `ironmaint-fixture` pattern
/// (`bins/ironmaint-fixture/src/lib.rs::fixture_binary_path`).
///
/// Two resolution strategies are tried, in order:
///
/// 1. The `CARGO_BIN_EXE_ironmaint-debian-tool` env var,
///    set by Cargo at build time when the test runs in a
///    context where the binary is a sibling target (the
///    unit tests in this crate, the binary's own
///    integration tests). This is the most precise
///    answer — Cargo resolves `CARGO_TARGET_DIR`,
///    `--profile`, and per-target artifact directories.
/// 2. A walk up to the workspace root and into
///    `target/{debug,release}/`. This is what other
///    crates in the workspace (e.g. `ironmaintd`'s
///    integration tests) need, because their build
///    context does not set the env var.
#[must_use]
pub fn binary_path() -> PathBuf {
    if let Some(p) = std::env::var_os(format!("CARGO_BIN_EXE_{BINARY_NAME}")) {
        return PathBuf::from(p);
    }
    target_dir_walk()
}

fn target_dir_walk() -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_root().join("target"), PathBuf::from);
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join(BINARY_NAME);
        if candidate.is_file() {
            return candidate;
        }
    }
    // Fall back to the most common location even if it
    // does not exist; the caller (the executor's
    // `check_launchable`) will produce a precise error
    // message identifying the missing path.
    target_dir.join("debug").join(BINARY_NAME)
}

/// Walk up from this crate's manifest directory to the
/// workspace root. The lib lives at
/// `bins/ironmaint-debian-tool/src/lib.rs`, so the
/// manifest is at `bins/ironmaint-debian-tool/`, two
/// levels below the workspace root.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut p = manifest_dir;
    for _ in 0..2 {
        if !p.pop() {
            break;
        }
    }
    p
}
