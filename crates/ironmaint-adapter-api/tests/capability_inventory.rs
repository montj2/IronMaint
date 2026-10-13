//! S3 — every planned capability resolves to a registered tool, or is
//! on a list that says who will supply it.
//!
//! The seam this closes is between the **adapter layer** and the
//! **executor layer**, and it is open in a way the daemon already
//! reports honestly at runtime: a captured `debian` job materialises
//! gates whose capability keys have no registered tool, and
//! `check.run` returns `no tool registered for capability
//! debian.build.sbuild`.
//!
//! That is *correct* in Phase 0 — the adapters are stubs and only
//! `synthetic.*` tools exist. The problem is where the fact lives. A
//! runtime error message is a fact you learn by running a job, at the
//! moment an agent is waiting on you. This turns it into a build-time
//! statement, and gives it a second form: for every unresolved key, a
//! **reason naming the phase that will supply it**. That reason is the
//! part an adapter author actually needs, and it is the part a runtime
//! error cannot carry.
//!
//! ## Why a test and not a source scan
//!
//! The set of capabilities an adapter can plan is a *runtime value* —
//! it comes out of `build_plan` and `qa_plan`, and a real adapter will
//! compute it from the package, the release and the policy. Reading the
//! source for string literals would pass on every stub and fail on the
//! first conditional one. This drives the real `DistributionAdapter`
//! and compares against a registry the daemon's own tool set builds,
//! so a new stub is covered the day it lands and a newly registered
//! tool empties its own allowlist entry without anyone editing a list.
//!
//! ## The allowlist is a claim, and claims rot
//!
//! Every entry says which phase supplies the tool, and
//! `every_allowlist_entry_is_still_needed` runs in the other
//! direction too: an entry for a key no adapter plans, or for a key
//! that has since gained a registered tool, is a stale claim rather
//! than a plan, and a reader has no way to tell the difference.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use debian_stub::DebianStubAdapter;
use fedora_stub::FedoraStubAdapter;
use ironmaint_adapter_api::{DistributionAdapter, ToolCapabilityKey};
use ironmaint_executor::ToolRegistry;
use ironmaint_testkit::{fixture_binary_path, fixtures, register_fixture_tools};

/// Capabilities that have no registered tool, and the phase that
/// supplies each one.
///
/// **Every entry names a phase.** An entry that cannot name one does
/// not belong here — it belongs in `doc/DEBT.md`, where it is a debt
/// rather than a plan, and where it will be argued with.
const ALLOWLIST: &[(&str, &str)] = &[
    (
        "debian.build.sbuild",
        "Phase 1 — `doc/CONTAINER-IMAGES.md` §4.3. The build image does not \
         exist yet; sbuild runs in the container that §4.3 specifies.",
    ),
    (
        "debian.qa.lintian",
        "Phase 1 — same image as `debian.build.sbuild`. Lintian runs against \
         the built package, so it cannot ship before the build image does.",
    ),
    (
        "debian.qa.piuparts",
        "Phase 1 — piuparts exercises the *source* package, so unlike lintian \
         it does not need the build image; it needs the checkout, which the \
         capture boundary already produces. It is the first of these that can \
         be unblocked without §4.3.",
    ),
    (
        "debian.test.autopkgtest",
        "Phase 1 — needs the autopkgtest chroot, which is the second half of \
         `doc/CONTAINER-IMAGES.md` §4.3 and cannot be built without the build \
         image.",
    ),
    (
        "fedora.build.mock",
        "Phase 1 — Mock chroot build. Same shape as the Debian build image and \
         the same §4.3 work.",
    ),
    (
        "fedora.qa.rpmlint",
        "Phase 1 — runs against the built RPM, so it follows \
         `fedora.build.mock`.",
    ),
    (
        "fedora.qa.spectool",
        "Phase 1 — runs against the built RPM, so it follows \
         `fedora.build.mock`.",
    ),
    (
        "fedora.test.functional",
        "Phase 1 — needs a booted guest; this is the one entry on this list \
         that is not a container image but a test harness.",
    ),
];

/// The set the daemon actually holds, built by the same call
/// `bins/ironmaintd/src/main.rs` makes.
///
/// Not a list of literals: if the daemon gains a tool, this set gains
/// it in the same build, and any allowlist entry for that key becomes
/// a stale claim that this test can then catch.
fn daemon_registered_tools() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_fixture_tools(&mut registry, &fixture_binary_path());
    registry
}

/// Every capability key this adapter can plan, across both plan
/// methods.
///
/// `build_plan` and `qa_plan` are separate calls because that is how
/// the runtime uses them, and a key that only one of them can return is
/// exactly the kind that slips through a check that only looks at one.
fn planned_capabilities(adapter: &dyn DistributionAdapter) -> BTreeSet<String> {
    let descriptor = adapter.descriptor();
    let build = adapter.build().expect(
        "both stubs advertise the build capability, and a \
                 DistributionAdapter that does not is a different check",
    );

    let release = fixtures::release("test");
    let candidate = fixtures::source_candidate(descriptor.family.clone(), release);
    let ctx = fixtures::candidate_context(&candidate);

    let build_plan = build
        .build_plan(&ctx)
        .expect("build.build_plan must succeed");
    let qa_plan = build.qa_plan(&ctx).expect("build.qa_plan must succeed");

    build_plan
        .checks
        .iter()
        .chain(qa_plan.checks.iter())
        .map(|c| c.key.as_str().to_string())
        .collect()
}

fn assert_capabilities_resolve(label: &str, planned: BTreeSet<String>, registry: &ToolRegistry) {
    let allowlisted: BTreeSet<&str> = ALLOWLIST.iter().map(|(k, _)| *k).collect();

    // Collect rather than panic on the first. A new stub arriving with
    // three unowned capabilities should get one message naming three,
    // not three rounds of one.
    let unresolved: Vec<&String> = planned
        .iter()
        .filter(|key| {
            let registered = ToolCapabilityKey::new(key.as_str())
                .map(|k| registry.get(&k).is_some())
                .unwrap_or(false);
            !registered && !allowlisted.contains(key.as_str())
        })
        .collect();

    if unresolved.is_empty() {
        return;
    }

    // The failure has to name the two ways out, because "add a tool or
    // add a reason" is only useful if both are visible.
    panic!(
        "{label} plans {} capabilit{} with no registered tool and no \
         allowlist entry:\n  {}\n\n\
         For each, one of two things is true.\n\n  \
         The tool exists and the daemon does not register it -- add it \
         to `bins/ironmaintd`'s tool set, and this test resolves the key \
         on its own with no list edit.\n  \
         The tool does not exist -- add it to `ALLOWLIST` with a reason \
         naming the phase that will supply it.\n\n\
         A reason that cannot name a phase is not a plan; file it in \
         `doc/DEBT.md` instead.",
        unresolved.len(),
        if unresolved.len() == 1 { "y" } else { "ies" },
        unresolved
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("\n  "),
    );
}

#[test]
fn debian_stub_capabilities_all_resolve() {
    let adapter = DebianStubAdapter::new();
    assert_capabilities_resolve(
        "DebianStubAdapter",
        planned_capabilities(&adapter),
        &daemon_registered_tools(),
    );
}

#[test]
fn fedora_stub_capabilities_all_resolve() {
    let adapter = FedoraStubAdapter::new();
    assert_capabilities_resolve(
        "FedoraStubAdapter",
        planned_capabilities(&adapter),
        &daemon_registered_tools(),
    );
}

#[test]
fn every_allowlist_entry_is_still_needed() {
    let registry = daemon_registered_tools();
    let planned: BTreeSet<String> = planned_capabilities(&DebianStubAdapter::new())
        .into_iter()
        .chain(planned_capabilities(&FedoraStubAdapter::new()))
        .collect();

    for (key, reason) in ALLOWLIST {
        let is_registered = ToolCapabilityKey::new(*key)
            .map(|k| registry.get(&k).is_some())
            .unwrap_or(false);

        assert!(
            planned.contains(*key),
            "allowlist entry `{key}` names a capability no adapter plans. \
             Either the adapter stopped planning it -- delete the entry -- or \
             the key was renamed. Either way an entry for a key nothing \
             plans is a claim about work nobody is waiting for. Its stated \
             reason was: {reason}"
        );
        assert!(
            !is_registered,
            "allowlist entry `{key}` now HAS a registered tool. Remove it \
             from the list: an entry for a resolved key reads as 'not done \
             yet' and would be read that way forever. Its stated reason was: \
             {reason}"
        );
    }
}
