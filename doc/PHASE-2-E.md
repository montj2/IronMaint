# PHASE-2 PR 2.0-E — readiness guardrails

> Final readiness PR before Phase 2A. Three independent issues converge here:
> 1. **Gate aggregation is silently lossy.** A stage with two mandatory
>    `GateDefinition`s only enforces the first.
> 2. **Stubs advertise capabilities with no production caller.** Drop
>    the advertised-only set; the enum stays, the stubs just stop
>    over-promising.
> 3. **`source.import` is a footgun.** A `Vec<String>` `starts_with`
>    allowlist matches `https://github.com/debian/` to
>    `https://github.com/debian-mirror/foo.git`. Replace with a
>    parsed `SourceImportPolicy` and pin repeated-import behaviour
>    with a test.

PR: `phase-2/2.0-e-readiness-guardrails` (cut from `develop`)

## User decisions (this plan)

| Question | Decision |
|---|---|
| Repeated `source.import` for the same `job_id` | **Re-clone** (today's behaviour, add a test pinning the second-call semantics — same `workspace_handle`, `HEAD^{tree}` matches the new remote). |
| Stub capability cleanup | **Remove all 7 stub-only capabilities**: `FunctionalTestPlanning`, `UpgradeTestPlanning`, `ReproducibilityPlanning`, `IssueWrite`, `ReleaseMetadata`, `PublicationPlanning` (both stubs) and `UpstreamDiscovery` (fedora-stub only). |
| URL policy placement | **`SourceImportPolicy` in `ironmaint-core`** (modelled on `RepositoryRef`); `McpRuntime` carries `Vec<SourceImportPolicy>`; the wire `SourceImportInput::url` stays `String`; the dispatcher parses via `url::Url` and matches. |

## The three slices

### Slice 1 — gate aggregation fix

`crates/ironmaint-state/src/engine.rs:392-435` iterates
`for &stage in requirements.gates` and uses
`.find(|d| d.stage == stage && d.candidate == *fp && d.mandatory)`
(line 397). When a stage has two mandatory `GateDefinition`s for the
same candidate, only the first is enforced; the rest are silently
ignored. `evaluate_gate` in
`crates/ironmaint-runtime/src/check.rs:215-261` already aggregates
correctly across N checks bound to one `gate_id` — the gap is in the
engine loop, not the runtime aggregation.

**Fix (one file, `ironmaint-state`):** replace the `.find(...)` with
`.filter(|d| d.stage == stage && d.candidate == *fp && d.mandatory).cloned().collect::<Vec<_>>()`.
Iterate the vec; for each match, run the same `match result.status`
arm logic (Pass / NotApplicable / NotEvaluated|Blocked /
Fail|ReviewRequired). Empty filter (no mandatory `GateDefinition` at
the required stage) keeps today's "MissingGate once per stage"
behaviour.

**Regression test:** `s72_every_mandatory_check_at_a_stage_is_enforced`
in `crates/ironmaint-state/tests/engine_scenarios.rs`. Inserts two
mandatory `GateDefinition`s for the same `(stage, candidate)` via a
new `make_two_mandatory_gates` helper. Three sub-cases:

1. `gate1` = `Pass`, `gate2` absent → transition blocked
   (`IncompleteGate(gate2.id)`).
2. `gate1` = `Pass`, `gate2` = `Pass` → transition allowed.
3. `gate1` = `Pass`, `gate2` = `Fail` → transition blocked
   (`FailedGate(gate2.id)`).

**No change to:** `TransitionRequirements`,
`plan_to_materialised`, `persist_materialised`, `evaluate_gate`,
`TransitionContext`, `TransitionBlocker` variants. The fix is local
to `check_gates`.

### Slice 2 — stub capability cleanup

The real production callers of `DistributionAdapter` are exactly
three: `adapter.build()` (`crates/ironmaint-runtime/src/service.rs:2064`),
`adapter.inspection()` (`service.rs:2075`), `adapter.policy()`
(`service.rs:2103, 3094`). The stubs at
`adapters/debian-stub/src/descriptor.rs:21-32` and
`adapters/fedora-stub/src/descriptor.rs:17-29` advertise 11 and 12
of 13 capabilities, including `FunctionalTestPlanning`,
`UpgradeTestPlanning`, `ReproducibilityPlanning`, `IssueWrite`,
`ReleaseMetadata`, `PublicationPlanning` (and `UpstreamDiscovery` on
fedora-stub) — none of which has a production caller, and
PHASE-2.md §226 forbids automatic BTS/signing/upload in Phase 2.
The S6 seam check (`xtask/src/seams/adapters.rs:69-78`) already
enforces the production-side invariant; the stubs are out of
compliance.

**Fix (3 files):**

- `adapters/debian-stub/src/descriptor.rs:21-32` — drop
  `BuildPlanning`, `FunctionalTestPlanning`, `UpgradeTestPlanning`,
  `ReproducibilityPlanning`, `IssueWrite`, `ReleaseMetadata`,
  `PublicationPlanning` from the `caps` set. (Re-adding
  `BuildPlanning` is 2A.2's call.) Update the module docstring at
  lines 1-9 to reflect the trimmed set.
- `adapters/fedora-stub/src/descriptor.rs:17-29` — same drops, plus
  `UpstreamDiscovery`. Update the module docstring at lines 1-6.
- `crates/ironmaint-testkit/src/conformance.rs:97-140` — the
  `for cap in d.capabilities.iter() { match cap { ... } }` arm keeps
  its shape; the `_ => {}` catch-all absorbs the now-unmatched
  arms. Verify the conformance suite still passes the trimmed stub
  descriptors.
- `adapters/debian-stub/tests/` and `adapters/fedora-stub/tests/` —
  update any unit tests that asserted the now-removed capabilities
  (search for `AdapterCapability::FunctionalTestPlanning` etc. in
  each stub's test dir; remove the assertions).

**No change to:** `ironmaint-adapter-api/src/descriptor.rs` (the
enum stays the same), `adapters/debian/src/descriptor.rs`
(production already has the §11 expected set per 1B.3 GREEN), the
S6 seam check's `PHASE_2_CAPABILITIES` allowlist (it only
constrains production adapters, not stubs).

### Slice 3 — parsed `SourceImportPolicy` + repeated-import test

`crates/ironmaint-mcp/src/dispatch.rs:640-674` matches URLs against
`Vec<String>` with `String::starts_with` (line 655). An entry of
`https://github.com/debian/` silently accepts
`https://github.com/debian-mirror/foo.git`. The conformance
comment at `crates/ironmaint-testkit/src/conformance.rs:128-134`
already calls out the analogous `UpstreamDiscovery` debt.
Repeated `source.import` calls for the same `job_id` silently
re-clone (no test pins this). The 2.0-E work is the parsed-policy
+ tests before treating `source.import` as production ingress.

**New type** (`crates/ironmaint-core/src/source_import.rs`):

```rust
pub struct SourceImportPolicy {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path_prefix: Option<String>,
}
```

With `Serialize + Deserialize + JsonSchema + Debug + Clone + PartialEq + Eq`.
Methods:

- `parse(s: &str) -> Result<Self, SourceImportPolicyError>` —
  parses `scheme://host[:port][/path-prefix]` strings.
- `matches(&self, url: &url::Url) -> bool` — case-insensitive on
  host, exact on scheme, exact on port (or `None` matches default),
  prefix on path.
- `Default` (deny-all).

**Wiring (5 files):**

- `crates/ironmaint-core/src/lib.rs` — re-export.
- `crates/ironmaint-mcp/src/dispatch.rs:41` — change
  `McpRuntime::source_import_allowlist: Vec<String>` to
  `Vec<SourceImportPolicy>`. Update `Clone`, `new`,
  `with_source_import_allowlist`.
- `crates/ironmaint-mcp/src/dispatch.rs:640-674` — replace
  `starts_with` with `policy.matches(&url::Url::parse(&input.url)?)`.
- `bins/ironmaintd/src/config.rs:116` —
  `RuntimeConfig::source_import_allowlist: Vec<SourceImportPolicy>`.
  CLI flag and env var both parse via `SourceImportPolicy::parse`.
- `bins/ironmaintd/src/main.rs:500-502` — call site keeps its shape.
- `bins/ironmaintd/tests/startup_shutdown.rs:128-153` — helper
  signature changes to `start_with_allowlist(Vec<SourceImportPolicy>)`.

**No change to:** `WorkspaceManager::import_git_remote`. The URL is
still passed straight through to `git clone`; the policy is
enforced at the MCP layer. The `SourceImportInput` wire shape stays
`url: String` (no MCP tool rename, no schema renumbering).

## Reuse from the existing tree

- **`RepositoryRef` pattern**
  (`crates/ironmaint-core/src/repository.rs:35-68`) — established
  shape for a core-owned `url::Url`-backed value with serde +
  JsonSchema + `validate_url`. `SourceImportPolicy` is modelled on
  this.
- **`evaluate_gate` aggregation**
  (`crates/ironmaint-runtime/src/check.rs:215-261`) — already
  correct; 2.0-E fix is upstream in the engine.
- **`url::Url` plumbing** — already a workspace dep
  (`Cargo.toml:108`).
- **`make_gate_pair` fixture**
  (`crates/ironmaint-state/tests/engine_scenarios.rs:~888-925`) —
  extended (not replaced) by `make_two_mandatory_gates`.

## RED / GREEN pattern

- **RED:** spec doc lands; slice 1 test fails (engine only checks
  the first mandatory); slice 2 conformance test fails (stubs
  advertise the removed capabilities); slice 3 unit tests for the
  new type pass in isolation but the integration test fixture
  references `Vec<SourceImportPolicy>` before the dispatcher
  rewires (compile failure is the clean RED).
- **GREEN:** the three fixes land; the §97 gate passes.

## §106 invariants preserved

- **State machine:** the engine loop changes shape (more blockers
  possible for the same stage); the public
  `TransitionEngine::evaluate` API is unchanged. `TransitionBlocker`
  variants are unchanged. `TransitionRequirements` is unchanged.
- **Persistence:** no new store rows.
- **Executor:** unchanged.
- **Candidate identity:** unchanged.
- **Evidence:** unchanged. `evaluate_gate` aggregation rules are
  unchanged; the rank table is unchanged.
- **State ownership:** unchanged.
- **MCP boundary:** the `source.import` tool's wire shape is
  unchanged (`url: String`, `ref_name: Option<String>`,
  `job_id: JobId`). The internal `McpRuntime` field type changes
  from `Vec<String>` to `Vec<SourceImportPolicy>`. Tool count:
  still 15.
- **IronClaw integration:** unchanged.

## §97 gate result (target)

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
- A new MCP tool. (None needed — `source.import` is the same 15th
  tool.)
- A `BuildPlan` that carries a `BuildInputDescriptor` field.
  (2A.2's call.)
- A read-only bind-mount of the candidate. (2A.3's hardening.)
- A new `EvidenceKind` or `GateStage`. (None needed.)
- Changing the daemon's source-import CLI shape beyond the parse
  function. (The flag stays `--source-import-allowlist`; the env
  var stays `IRONMAINT_SOURCE_IMPORT_ALLOWLIST`; the comma-
  separated form is preserved.)
- A new `AdapterCapability` variant. (The enum stays; the stubs
  just stop advertising the ones with no production path.)
- Wiring `issues()` or `release()` into a production caller. (Out
  of scope; PHASE-2.md §226 forbids it.)
- The Fedora parallel. (Phase 2 follow-on.)

## Next steps (per `doc/phases/PHASE-2.md`)

- **2A.1** — first real-build code, the `debian.build.sbuild`
  `ToolDefinition`. Consumes `BuildInputHandle` from 2.0-D.
- **2A.2** — `BuildCapability` impl with a real `BuildPlan`;
  carries the `BuildTarget` (2.0-C) and the `BuildInputDescriptor`
  (2.0-D). Re-enables `BuildPlanning` on the production Debian
  adapter (S6 allowlist flip).
- **2A.3** — real `sbuild` invocation against the pinned-real
  fixture. Hardens the read-only mount and the actual `dpkg-source
  -b` step.
- **2A.4** — second tool (`debian.qa.lintian`).
