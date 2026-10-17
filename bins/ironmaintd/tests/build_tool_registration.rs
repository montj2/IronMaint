//! PHASE-2 PR 2A.1 — verify that the daemon's tool
//! registry contains `debian.build.sbuild`.
//!
//! The `BuildValidation` gate's
//! `evidence.debian.build.sbuild` check requires the
//! key to be registered before `check.run` will
//! dispatch it. The test calls the
//! `register_debian_tools` helper (which the GREEN
//! commit extracts from `main.rs` and adds the
//! `debian.build.sbuild` `ToolDefinitionRecord` to)
//! and asserts the key is present.
//!
//! RED: `ironmaintd::register_debian_tools` does not
//! exist yet. The test fails to compile. Once the
//! GREEN commit lands the helper and the new
//! registration, the test compiles and passes.

use ironmaint_adapter_api::ToolCapabilityKey;
use ironmaintd::register_debian_tools;
use std::path::PathBuf;

#[test]
fn build_sbuild_tool_is_registered() {
    // PHASE-2 PR 2A.1: the daemon must register
    // `debian.build.sbuild` so the BuildValidation gate
    // can dispatch `check.run` for it. The
    // `register_debian_tools` helper builds a fresh
    // `ToolRegistry` with the same six Debian tools
    // (the five existing `debian.inspect.*` /
    // `debian.policy.delta_review` /
    // `debian.issue.bts_snapshot` plus the new
    // `debian.build.sbuild`).
    let mut registry = ironmaint_executor::ToolRegistry::new();
    let debian_tool_bin = PathBuf::from("/nonexistent/ironmaint-debian-tool");
    register_debian_tools(&mut registry, &debian_tool_bin, None)
        .expect("register_debian_tools must succeed");

    let build_key = ToolCapabilityKey::new("debian.build.sbuild")
        .expect("`debian.build.sbuild` is a valid capability key");
    assert!(
        registry.get(&build_key).is_some(),
        "ToolRegistry must contain `debian.build.sbuild` (the BuildValidation gate's build check)"
    );
}
