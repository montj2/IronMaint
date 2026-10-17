# PHASE-2 PR 2A.1 — define the build contract (`debian.build.sbuild`)

> First real-build code in IronMaint. Per
> `doc/phases/PHASE-2.md` §82–96: a versioned, structured
> sbuild report, a normalizer that classifies outcomes
> (`Pass` / `PackageBuildFailed` / `InfrastructureFailed` /
> `Malformed` / `MissingTool` / `Unknown`), and a hermetic
> `ToolDefinition` for `debian.build.sbuild` registered in
> the daemon's tool registry. **No real `sbuild` invocation
> lands in 2A.1** (that is 2A.3); **no production
> `BuildCapability` impl** (2A.2). 2A.1 is the report
> type, the normalizer, the tool registration, and
> hermetic tests.

PR: `phase-2/2a-1-build-contract` (cut from `develop`)

## Context

Up through 2.0-E, every tool that runs is either a
synthetic `synthetic.*` fixture (under
`ironmaint-fixture`) or one of the five read-only
`debian.inspect.*` / `debian.issue.bts_snapshot` checks.
None of them changes the world. 2A.1 is the seam that
opens the door to running real `sbuild`, and the spec
(`doc/phases/PHASE-2.md` §82–96) insists the door opens
with a **versioned, structured report** and a
**classified outcome** so that:

- the candidate binding (fingerprint, commit, tree)
  survives the subprocess boundary,
- exit codes / log snippets can never be silently
  re-read as "package defect" when they are actually
  "infrastructure failure" or "tool missing",
- 2A.3 (real `sbuild`) can drop the report type in
  unchanged, and
- 2A.4 (MCP end-to-end) can `check.run` against a
  hermetic fixture and get back a typed
  `DebianBuildReportV1` whose `outcome` advances the
  gate to `BuildValidation`.

### What the spec says, verbatim

From `doc/phases/PHASE-2.md` §82–96:

> ### PR 2A.1 — Define build contract
>
> Add a versioned, structured sbuild report and
> normalizer.
>
> The report should contain:
>
> - Candidate fingerprint, commit, and tree.
> - Source package name and version.
> - Target suite and architecture.
> - sbuild version and build-environment identity.
> - Start/end timestamps, exit information, and
>   classified outcome.
> - References and SHA-256 digests for the build log
>   and generated artifacts.
> - Stable diagnostics for package failures and
>   infrastructure failures.
>
> Pin the report schema and add tests for successful
> builds, package build failures, infrastructure
> failures, malformed output, and missing tools.
> Unknown failure modes must not be silently
> classified as package defects.

### What 2A.1 consumes from 2.0-C / 2.0-D

- `ironmaint_adapter_api::BuildTarget`
  (`crates/ironmaint-adapter-api/src/build_target.rs:8`)
  — `{ suite, arch }`. The report's `target` field
  reuses this shape (not a newtype, since the report
  is the wire format).
- `ironmaint_workspace::build_input::BuildInputHandle`
  (`crates/ironmaint-workspace/src/build_input.rs`)
  — exposes `tree_sha()`, `commit()`, and the
  descriptor's candidate fingerprint. 2A.1's
  **fixture** carries these; 2A.3's **real wrapper**
  will too.
- `EvidenceKind::Build` already exists
  (`crates/ironmaint-evidence/src/evidence.rs:142`).
- `GateStage::BuildValidation` already exists
  (`crates/ironmaint-evidence/src/gate.rs:31`); the
  runtime's
  `gate_stage_for(EvidenceKind::Build) → Some(GateStage::BuildValidation)`
  is already wired
  (`crates/ironmaint-runtime/src/check.rs:40`).

### What 2A.1 deliberately does NOT do

- No real `sbuild` invocation. (PR 2A.3.)
- No `BuildCapability` impl on the production Debian
  adapter (`adapters/debian/src/descriptor.rs`). (PR
  2A.2; the S6 seam check at
  `xtask/src/seams/adapters.rs:69-78` still refuses
  `BuildPlanning` on non-stub adapters.)
- No Fedora parallel. (Phase 2.4.)
- No new MCP tool, no new `EvidenceKind`, no new
  `GateStage`, no new `AdapterCapability`. (All exist;
  2A.1 is additive.)
- No read-only bind-mount. (`chmod -R a-w` from 2.0-D
  is sufficient; bind-mount is 2A.3's hardening.)
- No daemon-level end-to-end MCP test in 2A.1. (2A.4.)
- No new SQLite migration. The `Evidence` schema is
  unchanged; the report is delivered as the existing
  `evidence.note` payload (or, in 2A.3, an artifact
  referenced via `EvidenceScope::Candidate(...)`).
- No `ironmaint-fixture` extension. (The fixture's
  `debian.build.sbuild` scenarios are 2A.4's job —
  2A.1's tests are hermetic and construct
  `ExecutionRecord`s directly.)

## Decisions (this PR)

### 1. Where the report type lives

**Decision.** The `DebianBuildReportV1` struct, the
`BuildOutcome` enum, the `ArtifactRef` struct, and the
schema-version constants all live in
`bins/ironmaint-debian-tool/src/report.rs` — the same
module that already carries the five existing Debian
reports
(`DebianSourcePreparationV1`, `DebianSourceReportV1`,
`DebianMaintenanceContextV1`,
`DebianPolicyDeltaReviewV1`, `DebianIssueReportV1`).
The normalizer goes in
`bins/ironmaint-debian-tool/src/normalizer.rs`. The
`report.rs` module is the wire-1 surface; 2A.1
extends it with one more report and one more
verdict tri-state.

### 2. The `BuildOutcome` enum (4 variants) + 2 normalizer-only cases

**Decision.** `BuildOutcome` is a 4-variant enum that
the tool sets in its report:

- `Pass` — the build succeeded. Normalizer returns
  `EvidenceStatus::Pass`.
- `PackageBuildFailed` — the build failed for a
  package-defect reason (compile error, missing
  build-dep, etc.). Normalizer returns
  `EvidenceStatus::Fail` with the report's
  `diagnostics` as observations.
- `InfrastructureFailed` — the build failed for an
  infrastructure reason (sbuild chroot missing,
  network down, etc.). Normalizer returns
  `NormalizationError::Unclassifiable(...)`, which
  the runtime maps to
  `Outcome::InfrastructureFailed` →
  `GateStatus::Blocked`.
- `Unknown(String)` — catch-all. The tool saw a
  failure it could not classify. Normalizer returns
  `NormalizationError::Unclassifiable(...)` —
  **never `Fail`**, per the spec's "Unknown failure
  modes must not be silently classified as package
  defects."

In addition, the normalizer recognises two
**normalizer-only** cases that no real tool ever
emits in the `outcome` field:

- **Malformed** — the tool's stdout did not parse as
  `DebianBuildReportV1`. Normalizer returns
  `NormalizationError::Malformed(...)` (the runtime
  maps this to `Outcome::Fail` — a defect, not an
  outage). The type-level distinction from
  `Unclassifiable` lets a reader tell "the tool
  produced garbage" specifically.
- **MissingTool** — the executor's `check_launchable`
  returned `Err` or the record carries
  `exit_code == -1 && stdout.is_empty() &&
  stderr.is_empty()`. Normalizer returns
  `NormalizationError::Unclassifiable(...)` (the
  runtime maps this to
  `Outcome::InfrastructureFailed` →
  `GateStatus::Blocked`).

Together the 4 enum variants and the 2 normalizer
cases cover every scenario the spec names
("successful builds, package build failures,
infrastructure failures, malformed output, and
missing tools") plus the catch-all `Unknown` that
the spec explicitly demands. The 5 hermetic tests
in `bins/ironmaint-debian-tool/tests/build_report.rs`
cover all 6: `Pass`, `PackageBuildFailed`,
`InfrastructureFailed`, the `Malformed` normalizer
case, the `MissingTool` normalizer case, and (via
the `Unknown(_)` arm of the normalizer's match) the
catch-all.

### 3. The `Malformed` mapping

**Decision.** `Malformed` stdout (does not parse to
`DebianBuildReportV1`) maps to
`NormalizationError::Malformed(...)`, which the
runtime maps to `Outcome::Fail` — a defect (the tool
produced wrong output), not an outage. This is
deliberately different from `Unclassifiable` (which
maps to `Outcome::InfrastructureFailed → Blocked`),
even though both `Err` arms of the normalizer funnel
into the same `ExecutorError::InfrastructureFailed`
in the §30 runtime. The type-level distinction lets a
§24-aware reader tell "the tool produced garbage"
(specifically a defect) from "the BTS service
returned garbage" (a service-side outage). The
spec's "Unknown failure modes must not be silently
classified as package defects" requirement is met by
the `Unknown` variant's `Unclassifiable` mapping —
the failure mode is observable, not silently `Fail`.

### 4. The tool's `command` and `fixed_args`

**Decision.** The `ToolDefinition` for
`debian.build.sbuild` is registered with:

- `executable = config.debian_tool_bin` (the
  `ironmaint-debian-tool` binary — same as the
  five existing Debian inspection tools).
- `fixed_args = ["build"]` — the new `Sub::Build`
  clap subcommand.
- `class = ExecutionClass::Check`.
- `limits = { timeout: 30min, stdout_max_bytes:
  8MiB, stderr_max_bytes: 8MiB }` — conservative
  defaults; 2A.3 tunes these against the
  `pinned-real` fixture.
- `input_mode = ToolInputMode::JsonStdin` — the
  runtime writes the v3 candidate input to stdin.
- `with_normalizer(Arc::new(DebianBuildReportNormalizer::new()))`.
- `tool.env()` is empty; the `BUILD_*` env vars are
  injected per-call by the runtime in 2A.2 via
  `ExecutionRequest::env_overrides`.

### 5. The env-var contract for the build subcommand

**Decision.** For 2A.1, the `Sub::Build` body is a
stub that emits a hard-coded
`DebianBuildReportV1 { outcome: Pass, … }`. The body
reads the following env vars (which the runtime
will inject in 2A.2; for 2A.1 the env is empty and
the stub uses placeholder values):

- `BUILD_INPUT_DIR` — the staged source root
  (`BuildInputHandle::root()`).
- `BUILD_OUTPUT_DIR` — the sibling output dir
  (`BuildInputHandle::output_dir()`).
- `BUILD_TARGET_SUITE` — `BuildTarget::suite`.
- `BUILD_TARGET_ARCH` — `BuildTarget::arch`.
- `BUILD_FINGERPRINT` — `BuildInputHandle::candidate().fingerprint()`.
- `BUILD_COMMIT` — `BuildInputHandle::commit()`.
- `BUILD_TREE_SHA` — `BuildInputHandle::tree_sha()`.

2A.3 will replace the stub body with a real
`sbuild` invocation that consumes the same env vars
and the same stdin JSON. The env-var contract is
the same shape; the body is what changes.

### 6. The registry helper extraction

**Decision.** `bins/ironmaintd/src/main.rs`'s
in-line `tools.register(...)` calls for the five
existing Debian inspection tools are left in place
unchanged; the new `debian.build.sbuild`
registration is added as a sixth in-line call
following the same shape (no helper extraction — the
shape is already stable, and 2A.4's e2e test will
assert the key is registered against the existing
in-line registration). The 2A.1 test
(`bins/ironmaintd/tests/build_tool_registration.rs`)
calls a new `register_debian_tools(...)` helper
extracted from `main.rs` that builds a fresh
`ToolRegistry` with the same six Debian tools
registered and asserts the `debian.build.sbuild`
key is present.

## Files

### RED commit

| Path | Kind | Purpose |
|---|---|---|
| `doc/PHASE-2-A-1.md` | new | This design doc. |
| `bins/ironmaint-debian-tool/tests/build_report.rs` | new | 5 hermetic normalizer tests; will fail to compile until GREEN. |
| `bins/ironmaintd/tests/build_tool_registration.rs` | new | 1 registry test; will fail to compile until GREEN. |

### GREEN commit

| Path | Kind | Purpose |
|---|---|---|
| `bins/ironmaint-debian-tool/src/report.rs` | edit | `BUILD_REPORT_KIND`, `BUILD_REPORT_SCHEMA_VERSION`, `BuildOutcome`, `ArtifactRef`, `DebianBuildReportV1`. |
| `bins/ironmaint-debian-tool/src/normalizer.rs` | edit | `DebianBuildReportNormalizer` + 5 hermetic tests in the `build_report_tests` submodule. |
| `bins/ironmaint-debian-tool/src/lib.rs` | edit | Re-export `DebianBuildReportV1`, `BuildOutcome`, `DebianBuildReportNormalizer`, `DebianBuildReportNormalizer::new`. |
| `bins/ironmaint-debian-tool/src/main.rs` | edit | `Sub::Build` clap variant, `run_build_cmd()` stub, dispatch arm. |
| `bins/ironmaintd/src/main.rs` | edit | Register `debian.build.sbuild` `ToolDefinitionRecord`; add `register_debian_tools` helper extracted from in-line code. |
| `bins/ironmaintd/src/lib.rs` | edit | Expose `register_debian_tools` for the test. |
| `schemas/DebianBuildReportV1.json` | new (generated) | Re-emitted by `cargo xtask verify-schemas --write`. |
| `crates/ironmaint-adapter-api/tests/capability_inventory.rs` | edit | Remove the `("debian.build.sbuild", "Phase 1 …")` ALLOWLIST entry — the tool is now registered. |
| `doc/PHASE-2-A-1-COMPLETION.md` | new | PR record. |
| `doc/DEBT.md` | edit | File a new D-27 row (2A.1 capability pinning), closed by 2A.1 GREEN. |

## Reuse from the existing tree

- **ToolDefinition registration pattern** —
  `bins/ironmaintd/src/main.rs:214–424` builds the
  `ToolRegistry` and registers each tool via
  `ToolDefinitionRecord::new(...).with_input_mode(JsonStdin).with_normalizer(Arc::new(...Normalizer))`.
  2A.1's `debian.build.sbuild` is a sixth Debian
  tool alongside the existing five.
- **ResultNormalizer pattern** —
  `bins/ironmaint-debian-tool/src/normalizer.rs:90–677`
  (the existing normalizer tests and the
  `DebianBtsSnapshotNormalizer::normalize` impl) is
  the template. 2A.1's
  `DebianBuildReportNormalizer` follows the exact
  shape: a `new()` constructor and a
  `normalize(&self, &ExecutionRecord) -> Result<NormalizedResult,
  NormalizationError>` method.
- **Hermetic-test pattern** —
  `bins/ironmaint-debian-tool/src/normalizer.rs:90–280`
  (existing normalizer tests) construct canned
  `ExecutionRecord`s and call the normalizer. No
  subprocess spawn. 2A.1's
  `tests/build_report.rs` follows the same style.
- **JSON-schema regeneration** — `cargo xtask
  verify-schemas --write` (mirroring how
  `schemas/DebianPolicyDeltaReviewV1.json` is
  committed).
- **BuildTarget reuse** — the report's `target`
  field is the same shape as
  `BuildTarget::suite / arch`. The report is the
  wire format; no newtype, no enum (avoids
  renaming churn on schema bumps).
- **`ToolCapabilityKey::new("debian.build.sbuild")`**
  — exists and is tested at
  `crates/ironmaint-adapter-api/src/tool_key.rs:127–190`;
  the inventory test at
  `crates/ironmaint-adapter-api/tests/capability_inventory.rs:55-60`
  already lists it. 2A.1 removes the
  `("debian.build.sbuild", "Phase 1 …")` entry from
  the `ALLOWLIST`.
- **2.0-D's `chmod -R a-w`** — the staged root is
  already read-only at stage time; the executor's
  precondition (tested in
  `crates/ironmaint-workspace/src/build_input.rs`
  and `crates/ironmaint-workspace/src/manager.rs`)
  already covers it. 2A.1 inherits this for free.

## RED / GREEN

### RED commit

Three things land in RED:

1. **This design doc** (`doc/PHASE-2-A-1.md`).
2. **Five failing normalizer tests** at
   `bins/ironmaint-debian-tool/tests/build_report.rs` —
   RED because `DebianBuildReportNormalizer`,
   `DebianBuildReportV1`, and `BuildOutcome` don't
   exist yet. The test file imports
   `ironmaint_debian_tool::DebianBuildReportNormalizer`
   which is a compile error until GREEN.
3. **One registry test** at
   `bins/ironmaintd/tests/build_tool_registration.rs` —
   RED because `ironmaintd::register_debian_tools` does
   not exist yet. The test imports
   `ironmaintd::register_debian_tools` which is a
   compile error until GREEN.

The §97 gate `cargo test --workspace --locked` will
fail to compile either test crate; the failure is
the RED state.

### GREEN commit

Code lands to make RED green:

1. `bins/ironmaint-debian-tool/src/report.rs` —
   `BuildOutcome`, `ArtifactRef`, `DebianBuildReportV1`,
   constants.
2. `bins/ironmaint-debian-tool/src/normalizer.rs` —
   `DebianBuildReportNormalizer` + 5 tests.
3. `bins/ironmaint-debian-tool/src/lib.rs` — re-exports.
4. `bins/ironmaint-debian-tool/src/main.rs` — `Sub::Build`
   clap variant, `run_build_cmd()` stub.
5. `bins/ironmaintd/src/main.rs` — register
   `debian.build.sbuild`; extract `register_debian_tools`.
6. `bins/ironmaintd/src/lib.rs` — expose the helper.
7. `schemas/DebianBuildReportV1.json` — regenerated
   by `cargo xtask verify-schemas --write`.
8. `crates/ironmaint-adapter-api/tests/capability_inventory.rs` —
   remove the `debian.build.sbuild` ALLOWLIST entry.
9. `doc/PHASE-2-A-1-COMPLETION.md` — PR record.
10. `doc/DEBT.md` — D-27 filed and closed.

## §106 invariants preserved

`CLAUDE.md` cites §106 as the bar Phase 0 closed; the
do-not-redesign list is **runtime, persistence,
executor, candidate identity, evidence, state
ownership, MCP boundary, IronClaw integration**. 2A.1
touches **none** of these:

| Invariant | Why 2A.1 preserves it |
|---|---|
| **State machine** | `TransitionEngine::evaluate` is unchanged. `TransitionBlocker` variants are unchanged. `gate_stage_for(EvidenceKind::Build) → Some(GateStage::BuildValidation)` already exists (`crates/ironmaint-runtime/src/check.rs:40`). |
| **Persistence** | No new store rows. `Evidence` schema is unchanged; the report is delivered as `evidence.note` (or, in 2A.3, an artifact referenced via `EvidenceScope::Candidate(...)`). No new SQLite migration. |
| **Executor** | `ToolDefinition`, `ToolRegistry`, `ProcessExecutor`, `ToolDefinitionRecord`, `ResultNormalizer` — all unchanged. 2A.1 only registers an instance. |
| **Candidate identity** | `SourceCandidate` / `FrozenWorktree::tree_sha()` / BLAKE3 fingerprint are unchanged. The candidate's identifiers flow from `BuildInputHandle::candidate()` into the report's `provenance` fields. |
| **Evidence** | `EvidenceKind::Build` already exists. The normalizer emits `NormalizedResult { evidence_status, output_truncated, observations, invalidations }` — the same shape every normalizer emits. |
| **State ownership** | No new gate definitions. `GateStage::BuildValidation` already exists. |
| **MCP boundary** | No new MCP tool. `check.run` is the surface (15 existing tools, unchanged count). |
| **IronClaw integration** | Unchanged. The orchestrator's `run_check` call is unmodified; it receives a `DebianBuildReportV1` typed payload as `evidence.note`. |

## §97 gate result (target)

```text
cargo fmt --check                                                                clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings    clean
cargo run -p xtask --locked -- verify-architecture                               no violations
cargo run -p xtask --locked -- verify-schemas                                    no drift (after --write)
cargo run -p xtask --locked -- verify-migrations                                 clean
cargo run -p xtask --locked -- verify-mcp-schemas                                no drift (15 tools)
cargo run -p xtask --locked -- verify-seams                                      no violations
cargo test --workspace --locked                                                  all pass
cargo test --workspace --features integration --locked                           all pass
```

`verify-seams` is unchanged: production Debian adapter
still does not advertise `BuildPlanning`; the S6
allowlist at `xtask/src/seams/adapters.rs:69-78` is
unchanged. `verify-schemas` regenerates
`schemas/DebianBuildReportV1.json` (new) and
`schemas/mcp/check.run.json` (unchanged) — the 15-tool
MCP count is preserved.

## What 2A.1 deliberately did NOT do

- **No real `sbuild` invocation.** (2A.3.)
- **No `BuildCapability` impl on the production
  Debian adapter.** (2A.2.)
- **No Fedora parallel.** (Phase 2.4.)
- **No new MCP tool.** (None needed.)
- **No new `EvidenceKind`.**
  (`EvidenceKind::Build` exists.)
- **No new `GateStage`.**
  (`GateStage::BuildValidation` exists.)
- **No read-only bind-mount.** (`chmod -R a-w` from
  2.0-D is sufficient; bind-mount is 2A.3.)
- **No new artifact-store CAS keyed by BLAKE3.**
  (2.0-D doc defers; artifact store is SHA-256 only.)
- **No new `AdapterCapability`.**
  (`BuildPlanning` exists; the production adapter
  doesn't advertise it yet — 2A.2's S6 allowlist
  flip.)
- **No new SQLite migration.**
- **No change to daemon CLI surface.**
  `--debian-tool-bin` is reused; no new flag for
  `BUILD_*` env vars (2A.2's job).
- **No daemon-level end-to-end test in 2A.1.** The
  hermetic integration test for `debian.build.sbuild`
  is 2A.4's job.
- **No `ironmaint-fixture` extension.** (2A.4's
  job — the fixture's `debian.build.sbuild`
  scenarios drive the e2e MCP test.)

## Next steps (per `doc/phases/PHASE-2.md`)

- **2A.2** — `BuildCapability` impl on the production
  Debian adapter; one mandatory check
  `debian.build.sbuild` / `EvidenceKind::Build` /
  `GateStage::BuildValidation`; S6 allowlist flip to
  permit `BuildPlanning` on the production adapter;
  runtime injects the per-call `BUILD_*` env.
- **2A.3** — replace `run_build_cmd()` body with a
  real `sbuild` invocation; pinned source + verified
  hashes; buildroot provisioning; capture
  `.buildinfo` / `.changes` / binary artifacts;
  artifact-store digest verification; bind-mount
  hardening.
- **2A.4** — end-to-end MCP acceptance scenario
  (`check.run` against `debian.build.sbuild`; job
  advances to `BuildValidation`; structured report
  read back through `evidence.artifact.read`).
- **2A.5+** — second tool (`debian.qa.lintian`);
  Fedora parallel (mock / rpmlint).
