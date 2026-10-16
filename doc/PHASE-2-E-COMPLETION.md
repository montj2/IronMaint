# PHASE-2 PR 2.0-E — completion record

PR: `phase-2/2.0-e-readiness-guardrails`
Branch: `feature/phase-2/2.0-e-readiness-guardrails` (cut from `develop`)
Status: GREEN

This PR is the final readiness guardrail before Phase 2A. It
fixes three latent issues the 2A PRs would otherwise re-discover
the hard way:

1. The state machine's gate-aggregation loop silently dropped
   every mandatory check after the first.
2. The `debian-stub` and `fedora-stub` adapters advertised
   capabilities with no production caller.
3. The `source.import` URL allowlist was a `Vec<String>` with
   `starts_with`, which accepted `https://github.com/debian-`
   against an `https://github.com/debian/` policy.

It also pins the repeated-`source.import` behaviour and
uncovered a real macOS bug in the workspace manager's import
cleanup that the new repeated-import test exercises.

## What 2.0-E delivered

### Slice 1 — gate aggregation fix

`crates/ironmaint-state/src/engine.rs:392-435` (`check_gates`)
iterated `for &stage in requirements.gates` and used
`.find(|d| d.stage == stage && d.candidate == *fp && d.mandatory)`.
If a stage had two mandatory `GateDefinition`s for the same
candidate, only the first was enforced; the rest were silently
ignored. `evaluate_gate` in
`crates/ironmaint-runtime/src/check.rs:215-261` already
aggregated correctly across N checks bound to one `gate_id` —
the gap was in the engine loop, not the runtime aggregation.

The fix is local to `check_gates`:

```rust
let definitions: Vec<_> = ctx
    .gate_definitions
    .values()
    .filter(|d| d.stage == stage && d.candidate == *fp && d.mandatory)
    .cloned()
    .collect();
if definitions.is_empty() {
    blockers.push(TransitionBlocker::MissingGate(missing_gate_sentinel()));
    continue;
}

for def in definitions {
    // ... existing match-on-status logic, but for every definition
}
```

`TransitionRequirements`, `TransitionContext`, `TransitionBlocker`,
`plan_to_materialised`, `persist_materialised`, and
`evaluate_gate` are all unchanged. The fix is upstream in the
engine.

**New test:** `s72_every_mandatory_check_at_a_stage_is_enforced`
in `crates/ironmaint-state/tests/engine_scenarios.rs` seeds
two mandatory `GateDefinition`s for the same `(stage, candidate)`
and asserts three sub-cases:

1. `gate1` = `Pass`, `gate2` absent → transition blocked
   (`MissingGate(gate2.id)`).
2. `gate1` = `Pass`, `gate2` = `Pass` → transition allowed.
3. `gate1` = `Pass`, `gate2` = `Fail` → transition blocked
   (`FailedGate(gate2.id)`).

### Slice 2 — stub capability cleanup

The `DistributionAdapter` trait has three production callers in
`ironmaint-runtime`: `adapter.build()` (`service.rs:2064`),
`adapter.inspection()` (`service.rs:2075`), and `adapter.policy()`
(`service.rs:2103, 3094`). The stubs advertised 11 and 12 of
13 capabilities, including `FunctionalTestPlanning`,
`UpgradeTestPlanning`, `ReproducibilityPlanning`, `IssueWrite`,
`ReleaseMetadata`, `PublicationPlanning`, and (fedora-only)
`UpstreamDiscovery` — none of which has a production caller,
and `doc/phases/PHASE-2.md` §226 forbids automatic BTS /
signing / upload in Phase 2.

The S6 seam check (`xtask/src/seams/adapters.rs:69-78`) already
enforced the production-side invariant; the stubs were out of
compliance. 2.0-E brings them in line:

- `adapters/debian-stub/src/descriptor.rs` — trimmed to
  `VersionComparison`, `PolicyDerivation`, `PackageQaPlanning`,
  `IssueRead`. The dropped variants:
  `BuildPlanning`, `FunctionalTestPlanning`,
  `UpgradeTestPlanning`, `ReproducibilityPlanning`,
  `IssueWrite`, `ReleaseMetadata`, `PublicationPlanning`.
  `BuildPlanning` is intentionally deferred to 2A.2; the others
  are 2.0-E.
- `adapters/fedora-stub/src/descriptor.rs` — trimmed to
  `SourceInspection`, `VersionComparison`,
  `PackageQaPlanning`. The dropped variants:
  `BuildPlanning`, `FunctionalTestPlanning`,
  `UpgradeTestPlanning`, `ReproducibilityPlanning`,
  `IssueWrite`, `ReleaseMetadata`, `PublicationPlanning`,
  `UpstreamDiscovery`, `PolicyDerivation`, `IssueRead`.
- New tests in each stub assert the removed variants are NOT
  advertised (`capability_set_does_not_advertise_phase_2_no_callers`).

The `AdapterCapability` enum is unchanged. The conformance
arm `assert_descriptor_is_valid` in
`crates/ironmaint-testkit/src/conformance.rs` continues to
accept the trimmed descriptors — its match arm's `_ => {}`
catch-all absorbs the now-unmatched variants.

### Slice 3 — parsed `SourceImportPolicy`

`crates/ironmaint-core/src/source_import.rs` (NEW) defines:

```rust
pub struct SourceImportPolicy {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path_prefix: Option<String>,
}
```

with `Serialize + Deserialize + JsonSchema + Debug + Clone +
PartialEq + Eq + Hash`. The struct is modelled on the
established `RepositoryRef` pattern
(`crates/ironmaint-core/src/repository.rs:35-68`).

Methods:

- `parse(s: &str) -> Result<Self, SourceImportPolicyError>` —
  parses `scheme://host[:port][/path-prefix]` strings via
  `url::Url::parse`. The error enum carries the offending
  input for the dispatcher's diagnostic.
- `matches(&self, url: &url::Url) -> bool` — case-insensitive
  on scheme and host (after `url::Url::host_str()`), exact on
  port (`None` matches the URL's default for the scheme, set
  via the helper `default_port`), and a literal anchored prefix
  on path. Trailing-slash on `path_prefix` is preserved: a
  `https://github.com/debian/` policy matches
  `https://github.com/debian/foo` (directory prefix) but not
  `https://github.com/debian-mirror/foo` (sibling host). A
  `https://github.com/debian` policy (no trailing slash)
  matches the broader prefix.
- `Display` — re-emits the policy in
  `scheme://host[:port][/path]` form so it round-trips through
  `parse`. The CLI / env-var surface relies on this; the
  round-trip is unit-tested.
- `Default` — a deny-all sentinel (`https://` with no host).
  An empty `Vec<SourceImportPolicy>` is also deny-all.

**Wiring:**

- `crates/ironmaint-mcp/src/dispatch.rs` —
  `McpRuntime::source_import_allowlist: Vec<SourceImportPolicy>`
  (was `Vec<String>`). The `dispatch_source_import` body parses
  `input.url` via `url::Url::parse` and matches against the
  configured policies with `policy.matches(&parsed_url)`. The
  deny message names the configured policies and the rejected
  URL.
- `bins/ironmaintd/src/config.rs` — `RuntimeConfig::source_import_allowlist:
  Vec<SourceImportPolicy>`. The CLI flag
  `--source-import-allowlist` (repeatable) parses each entry
  through `SourceImportPolicy::parse`. The env var
  `IRONMAINT_SOURCE_IMPORT_ALLOWLIST` (comma-separated) does
  the same. Failure to parse is a daemon-startup error, not a
  per-call error.
- `bins/ironmaintd/Cargo.toml` — `ironmaint-core` is now an
  explicit dep so the daemon can use `SourceImportPolicy` in
  its config and tests.

The wire shape of `SourceImportInput` /
`SourceImportOutput` is unchanged. The tool count is still
15. The `source.import` JSON schema is unchanged
(`schemas/mcp/source.import.json` regenerates identically).

### Slice 4 — repeated-import pin (and a real macOS bug)

`doc/phases/PHASE-2.md` §2.0-E asked the agent to decide what
"repeated `source.import` for the same `job_id`" means. The
user chose **Re-clone** (today's behaviour) and asked for a
test that pins the second-call semantics. The new e2e test
`source_import_repeated_call_re_clones_to_same_handle` in
`bins/ironmaintd/tests/startup_shutdown.rs` does exactly that:
two `source.import` calls with the same `job_id` and URL,
asserting identical `workspace_handle` and
`HEAD^{tree}` match between the cloned workspace and the bare
remote.

The test exposed a real macOS bug in
`crates/ironmaint-workspace/src/manager.rs:300-303` — the
import-cleanup loop called `tokio::fs::remove_dir_all` on
**every** entry, including regular files like `README.md`.
Linux's `remove_dir_all` tolerates a file leaf; macOS returns
`ENOTDIR`. The bug was latent because the §1.1.G PR's only
import test was the single-call case. The fix dispatches on
`file_type`:

```rust
let file_type = entry.file_type().await?;
let path = entry.path();
if file_type.is_dir() || file_type.is_symlink() {
    tokio::fs::remove_dir_all(&path).await?;
} else {
    tokio::fs::remove_file(&path).await?;
}
```

This is in scope for 2.0-E because the test the user asked
for cannot pass without it, and 2.0-E's stated bar is "the
runtime, persistence, executor, candidate identity, evidence,
state ownership, MCP boundary, or IronClaw integration" must
not have to be redesigned for 2A. A workspace manager that
fails on the second call to a documented MCP tool would
qualify.

### Slice 5 — host-mismatch pin

The user asked for the parsed policy + tests before treating
`source.import` as production ingress. The new e2e test
`source_import_rejects_host_mismatch` configures
`https://github.com/debian/` and calls with
`https://github.com/debian-mirror/foo.git`, asserting the
structured error names the rejected host and references the
allowlist.

The matcher rejects on host **before** the path-prefix check
gets a chance to. This is the case the old `starts_with`
allowlist silently accepted; the test pins the corrected
behaviour.

## Tests

- `crates/ironmaint-core/src/source_import.rs` — 21 unit
  tests for `parse`, `matches`, `Display`, and JSON
  round-trip. Covers scheme, host, port, path-prefix,
  trailing-slash disambiguation, case-insensitivity,
  userinfo, IDN/punycode, scp-form rejection,
  javascript: rejection, default deny, and
  `Display` → `parse` round-trip.
- `crates/ironmaint-state/tests/engine_scenarios.rs` — the
  three sub-cases of
  `s72_every_mandatory_check_at_a_stage_is_enforced`.
- `adapters/debian-stub/tests/descriptor.rs` and
  `adapters/fedora-stub/tests/descriptor.rs` — the
  `capability_set_does_not_advertise_phase_2_no_callers`
  tests in each stub.
- `bins/ironmaintd/tests/startup_shutdown.rs` — the
  existing `source_import_clones_a_real_git_remote` and
  `source_import_rejects_a_url_not_on_the_allowlist` tests
  updated for the new `Vec<SourceImportPolicy>` helper
  signature, plus the two new tests:
  `source_import_repeated_call_re_clones_to_same_handle`
  and `source_import_rejects_host_mismatch`.

## Documentation

- `doc/PHASE-2-E.md` — the spec; the three slices, the
  user decisions, the §106 invariants, and the §97 gate
  result.
- `crates/ironmaint-workspace/src/manager.rs` — explicit
  comment recording the macOS-vs-Linux dispatch on
  `file_type` so a future cleanup refactor doesn't
  re-introduce the bug.
- `crates/ironmaint-core/src/source_import.rs` — module
  docstring records the `https://github.com/debian/` vs
  `https://github.com/debian-mirror/` asymmetry as a §2.0-E
  design decision.

## Debt register

- **D-26** filed in `doc/DEBT.md`: 2.0-E capability
  reconciliation, closed by 2.0-E GREEN.

## §106 invariants preserved

- **State machine:** `check_gates` shape changes (more
  blockers possible for the same stage); the public
  `TransitionEngine::evaluate` API is unchanged.
  `TransitionBlocker` variants are unchanged.
  `TransitionRequirements` is unchanged.
- **Persistence:** no new store rows. `GateDefinition`,
  `CheckDefinition`, `WorkspaceState`, and
  `SourceImportInput` / `SourceImportOutput` schemas are
  unchanged.
- **Executor:** unchanged.
- **Candidate identity:** unchanged.
- **Evidence:** unchanged. `evaluate_gate` aggregation rules
  are unchanged; the rank table is unchanged.
- **State ownership:** unchanged. The engine still owns
  transitions; the adapter still produces plans; the runtime
  still decides.
- **MCP boundary:** the `source.import` tool's wire shape is
  unchanged (`url: String`, `ref_name: Option<String>`,
  `job_id: JobId`). The internal `McpRuntime` field type
  changes from `Vec<String>` to `Vec<SourceImportPolicy>`.
  Tool count: still 15.
- **IronClaw integration:** unchanged.

## §97 gate result (target / observed)

```text
cargo fmt --check                                                                clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings    clean
cargo run -p xtask --locked -- verify-architecture                               no violations
cargo run -p xtask --locked -- verify-schemas                                    no drift
cargo run -p xtask --locked -- verify-migrations                                 clean
cargo run -p xtask --locked -- verify-mcp-schemas                                no drift (15 tools)
cargo run -p xtask --locked -- verify-seams                                      no violations
cargo test --workspace --locked                                                  all pass
cargo test --workspace --features integration --locked                           all pass
```

## What 2.0-E deliberately did NOT do

- A real `sbuild` invocation. (PR 2A.3.)
- A `BuildCapability` impl in the real adapter. (PR 2A.2.)
- A new MCP tool. (None needed — `source.import` is the same
  15th tool.)
- A `BuildPlan` that carries a `BuildInputDescriptor` field.
  (2A.2's call.)
- A read-only bind-mount of the candidate. (2A.3's
  hardening.)
- A new `EvidenceKind` or `GateStage`. (None needed.)
- Changing the daemon's source-import CLI shape beyond the
  parse function. (The flag stays
  `--source-import-allowlist`; the env var stays
  `IRONMAINT_SOURCE_IMPORT_ALLOWLIST`; the comma-separated
  form is preserved.)
- A new `AdapterCapability` variant. (The enum stays; the
  stubs just stop advertising the ones with no production
  path.)
- Wiring `issues()` or `release()` into a production caller.
  (Out of scope; `doc/phases/PHASE-2.md` §226 forbids it.)
- The Fedora parallel. (Phase 2 follow-on.)
- A new `MockMcpRuntime` or in-process test fixture. (No
  public API change justifies one; the e2e harness covers
  the dispatcher.)
- A change to the §97 gate. (The new tests run in the
  existing `cargo test --workspace` set; the e2e tests run
  in the existing `bins/ironmaintd/tests/startup_shutdown.rs`
  set.)
- A change to `WorkspaceManager::import_git_remote` beyond
  the `file_type` dispatch fix. (The URL still flows through
  to `git clone`; the policy is enforced at the MCP layer;
  the only workspace-layer change is the file/dir dispatch
  that the new repeated-import test exposed.)

## Next steps (per `doc/phases/PHASE-2.md`)

- **2A.1** — first real-build code, the
  `debian.build.sbuild` `ToolDefinition`. Consumes
  `BuildInputHandle` from 2.0-D.
- **2A.2** — `BuildCapability` impl with a real `BuildPlan`;
  carries the `BuildTarget` (2.0-C) and the
  `BuildInputDescriptor` (2.0-D). Re-enables `BuildPlanning`
  on the production Debian adapter (S6 allowlist flip).
- **2A.3** — real `sbuild` invocation against the
  pinned-real fixture. Hardens the read-only mount and the
  actual `dpkg-source -b` step.
- **2A.4** — second tool (`debian.qa.lintian`).
