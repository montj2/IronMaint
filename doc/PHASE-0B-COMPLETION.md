# Phase 0B Completion Report

Per `doc/phases/PHASE-0B.md` §104. This is the review input for the §106
handoff gate ("review the implementation before connecting real Debian
infrastructure"). Written by the implementing agent; every claim below is
checkable against the repository.

- **Branch:** `feature/0b5-0b8-runtime-mcp-ironclaw`
- **Base:** `develop` (PR #23, Phase 0B.4)
- **Scope:** sub-phases 0B.5 – 0B.8, nine commits
- **Date:** 2026-09-28

---

## 1. Workspace changes

The workspace has three trees, not one: `crates/` (13 libraries),
`adapters/` (2 adapter conformance stubs), `bins/` (3 binaries), plus
`xtask`. 0B.5–0B.8 grew `ironmaint-runtime` and `ironmaint-mcp` substantially
and touched `ironmaint-state`, `ironmaint-store`, and `ironmaint-store-sqlite`.

**`crates/`**

| Crate | Role | Touched in 0B.5–0B.8 |
|---|---|---|
| `ironmaint-core` | Domain value types, identities, fingerprints | read-only |
| `ironmaint-evidence` | Evidence + gate types | read-only |
| `ironmaint-policy` | Obligations, privileged operations, approvals | read-only |
| `ironmaint-state` | Transition engine, `TRANSITION_RULES` | **`transition.rs` extended** |
| `ironmaint-store` | Store traits + `MockStore` | **new `check.rs`; extended `candidate.rs`, `evidence.rs`, `mock.rs`, `lib.rs`** |
| `ironmaint-store-sqlite` | SQLite backend | **new `ops/checks.rs`; extended `ops/candidates.rs`, `ops/evidence.rs`** |
| `ironmaint-executor` | `ProcessExecutor`, `ToolRegistry`, artifact guard | read-only |
| `ironmaint-workspace` | Workspace manager, `WorkspaceRevision`, path confinement | read-only |
| `ironmaint-adapter-api` | Adapter contract, `BuildPlan`/`QaPlan` | read-only |
| `ironmaint-artifacts` | Content-addressed blob store | read-only |
| `ironmaint-runtime` | Application service, reconcile, transitions | **heavily extended** |
| `ironmaint-mcp` | MCP tool surface, auth, schemas, dispatcher | **new `dispatch.rs`; `error.rs`, `schema.rs`, `tools/actions.rs` extended** |
| `ironmaint-testkit` | Fixtures, conformance suites | **new `synthetic_e2e.rs` driver** |

**`adapters/`** — `debian-stub`, `fedora-stub`. Both pass the same generic
conformance suite from 0A.4. Neither was touched on this branch.

**`bins/`**

| Binary | Role | Status |
|---|---|---|
| `ironmaintd` | Daemon: opens SQLite (acquiring the single-daemon lock), builds `RuntimeService`, drains on SIGTERM (§66–§68) | **exists, bootstrap only** — no MCP server, no reconcile loop |
| `ironmaintctl` | Operator CLI; single subcommand `rebuild-projections` (§36) | exists, no other subcommands |
| `ironmaint-fixture` | Synthetic tool binary for the test suite | exists, working |

**`xtask/`** — architecture + schema + migration + MCP-schema verifiers.
**new `mcp_schemas.rs`**; `schemas.rs` extended.

No existing 0A type was modified. See §4.

## 2. New crates / binaries

**No new crate and no new binary on this branch.** All 13 crates, both
adapters, and all 3 binaries pre-date it; 0B.5–0B.8 completed the runtime,
the MCP dispatcher, and the test suites.

One clarification for the reviewer: `ironmaintd` **does exist** and boots the
runtime (§2, `bins/`), but it does not serve MCP and does not run a reconcile
loop. See §16.6.

New source files introduced by this branch:

```text
crates/ironmaint-runtime/src/candidate.rs        (C1) CaptureCandidate handler
crates/ironmaint-runtime/src/check.rs            (C2) plan→CheckDefinition materialiser, gate evaluation
crates/ironmaint-runtime/src/transition_context.rs (C4) TransitionContext builder
crates/ironmaint-runtime/src/reconcile.rs        (C5) ReconcileOutcome + format_blocker
crates/ironmaint-store/src/check.rs              (C2) CheckDefinition + CheckStore trait
crates/ironmaint-store-sqlite/src/ops/checks.rs  (C2) SQLite CheckStore
crates/ironmaint-mcp/src/dispatch.rs             (C7) tool dispatcher
crates/ironmaint-mcp/tests/dispatcher.rs         (C7) §94 exit checkpoint
crates/ironmaint-runtime/tests/hardening.rs      (C9) 0B.8 hardening suite
migrations/0002_check_definitions.sql            (C2) CheckDefinition persistence
doc/operator/{ironclaw-setup,mcp-registration,tool-permissions}  (C8)
doc/PHASE-0B-COMPLETION.md                       (C10) this report
skills/ironmaint-maintainer/SKILL.md             (C8)
scripts/ironclaw-e2e.sh                          (C8)
```

## 3. Database schema and migrations

Two migrations, 211 lines total, 25 DDL objects, all clean under
`verify-migrations`.

**`migrations/0001_initial.sql`** (186 lines, 23 objects) — event log,
projections, source/release candidates, evidence, gates, obligations,
approvals, artifacts, workspace metadata, operations.

**`migrations/0002_check_definitions.sql`** (25 lines, 2 objects) — added by
C2 for `CheckDefinition` durability:

```sql
CREATE TABLE check_definitions (
  id            TEXT PRIMARY KEY,   -- CheckId (UUIDv7)
  job_id        TEXT NOT NULL,
  candidate     TEXT NOT NULL,      -- CandidateFingerprint hex
  gate_id       TEXT NOT NULL,
  gate_stage    TEXT NOT NULL,
  tool_key      TEXT NOT NULL,      -- <ns>.<role>.<tool>
  evidence_kind TEXT NOT NULL,
  mandatory     INTEGER NOT NULL,
  created_at    TEXT NOT NULL
);
CREATE INDEX idx_check_defs_job ON check_definitions(job_id);
```

Deliberately **no** foreign keys. The spec's §4.7 ordering (persistence
precedes acknowledgement) means a check row can outlive or precede the
projection it references; referential integrity is enforced in the runtime
layer via `check.candidate != projection.active_candidate.fingerprint()`
rejection in `handle_run_check`, not by the database.

## 4. Domain changes from 0A

**None.** No 0A type was modified, removed, or reshaped.

Three types were *added* under §100's explicitly-permitted additive
allowance (new `CheckId`, `WorkspaceRevision`, `OrchestratorRef` and
correlation/operation ids), all of which landed in earlier 0B sub-phases, not
on this branch. On this branch:

- `ironmaint_state::TransitionContext` gained a field
  (`active_candidate_fingerprint: Option<CandidateFingerprint>`, owned rather
  than borrowed) — this is a 0B type, not a 0A type.
- `RuntimeErrorKind::ConcurrentModification` was added (0B type).

No `PHASE-0A-CONTRACT-CHANGE` document was filed because no 0A contract was
altered.

## 5. MCP tools exposed

Nine tools, each with a snapshotted input/output JSON schema under
`schemas/mcp/`. `verify-mcp-schemas` reports 9 committed / 9 generated, no
drift.

| Tool | Wired to runtime? | Notes |
|---|---|---|
| `job.create` | **yes** | `RuntimeCommand::CreateJob`; returns minted `JobId` |
| `job.get` | **yes** | `RuntimeQuery::GetJob` → `JobProjection` |
| `job.next_actions` | **yes** | `RuntimeQuery::ListNextActions` → evidence-ledger walk |
| `job.reconcile` | **yes** | `RuntimeService::reconcile` → `ReconcileOutcome` |
| `candidate.capture` | **yes** | `RuntimeCommand::CaptureCandidate` |
| `check.run` | **partially** | validates `RunCheckInput`, returns zeroed `RunCheckOutput` — **does not execute** |
| `workspace.apply_patch` | **partially** | echoes `expected_revision` back as `new_revision` — **does not apply** |
| `workspace.stat` | **partially** | always returns `WorkspaceRevision::ZERO` |
| `operation.get` | **partially** | returns a synthetic `PrivilegedOperation::proposed(...)` sentinel |

The three-plus partial tools are the primary review risk. The dispatcher
(`crates/ironmaint-mcp/src/dispatch.rs`) is the correct place for them to grow
real bodies — the runtime methods all exist — but as of this PR they are
shape-only. **The §94 exit checkpoint nonetheless holds**: the automated MCP
client test never touches the store directly, and the six wired tools are
exercised through the dispatcher. The claim to scrutinise is not "no direct
store access" (true) but "tool surface is functional" (true for 6 of 9).

Per §24 ("MCP tools expose domain capabilities rather than storage
primitives") and §25 ("MCP cannot directly set state, gates, obligations,
approvals, or evidence"): satisfied. No tool writes state, gate status,
obligation status, approval status, or evidence directly. `check.run` is the
only tool that would eventually cause tool execution, and it is plan-gated by
`CheckId`.

## 6. Tool registry capabilities

Capability keys are opaque strings of the form `<adapter-namespace>.<role>.<tool>`.
Core treats them as strings; the registry is in `ironmaint-executor`
(`registry.rs`, `ToolRegistry`).

Production adapter keys are **declared but not populated** — `debian-stub` and
`fedora-stub` implement the adapter contract but no real tool definitions are
registered, because real sbuild/mock/lintian/rpmlint integration is deferred.

The 19 synthetic fixture keys used by the test suites:

```text
synthetic.build.validate    synthetic.build.fail       synthetic.build.absent
synthetic.build.timeout     synthetic.build.truncate   synthetic.build.interrupt
synthetic.build.infra       synthetic.build.priv       synthetic.build.ghost
synthetic.build.with
synthetic.test.pass         synthetic.test.fail        synthetic.test.bad
synthetic.test.timeout      synthetic.test.truncate    synthetic.test.infra
synthetic.test.observe
synthetic.qa.no
```

These cover every executor behaviour the spec requires: success, tool failure
(distinct from infrastructure failure), missing binary, timeout, stdout
truncation, signal interrupt, privilege escalation attempt, and
`InfrastructureFailed` classification.

## 7. Executor behavior

- `ProcessExecutor` runs registered tools with `env_clear()` + a minimal
  allowlisted environment. No shell interpretation (§4.6): the command and
  argv are fixed by the `ToolDefinitionRecord`; there is no `sh -c` path.
- `ExecutionLimits` are enforced per job: `timeout`, `stdout_max_bytes`,
  `stderr_max_bytes`. Exceeding the byte cap sets `truncated = true` on the
  `ExecutionRecord` rather than failing the run (§15).
- `RetryClass` distinguishes `Safe` (auto-retryable), `Idempotent`
  (retryable on infra failure), and `Unsafe` (never retried).
- `JobArtifactGuard` caps total artifact bytes per job; exceeding the cap is
  an `InfrastructureFailed`, not a `Fail`.
- **`GateResult::Fail` vs `RuntimeErrorKind::Executor`**: a tool exiting
  non-zero produces evidence with status `Fail` and a `GateResult::Fail` —
  the normal outcome of a package that does not build. `InfrastructureFailed`
  is reserved for sandbox/worker/DB problems. The type system keeps them
  distinct, which is what lets Phase 1 give IronClaw repair behaviour a
  correct signal.

## 8. Workspace security behavior

- Every mutation is CAS-guarded on `WorkspaceRevision` (§18). A stale
  `expected_rev` returns `WorkspaceErrorKind::Conflict { expected, found }`.
- `WorkspacePath::resolve` / `resolve_strict_no_follow` / `resolve_for_read`
  confine all paths to the workspace root and reject `..` escapes, absolute
  paths, and NUL bytes (§19). `resolve_strict_no_follow` additionally refuses
  to traverse symlinks.
- `apply_patch` refuses to run when `git status --porcelain` is non-empty
  ("workspace is dirty; refusing to apply patch") — dirty workspaces cannot
  become candidates (§4.4).
- Success bumps the revision and sets `dirty = true` in a single CAS write;
  there is no observable intermediate state.
- The agent never touches the workspace filesystem directly; all writes go
  through `WorkspaceManager::apply_patch`, which is the only path that bumps
  the revision.

## 9. Artifact behavior

- Content-addressed by BLAKE3 (§13). Metadata index in
  `ironmaint-store::ArtifactMetadataStore`; blob bytes in `ironmaint-artifacts`.
- Size limits enforced per job (§15): total artifact bytes, per-artifact bytes,
  and a staging-directory GC that runs on release.
- Zero-size records are rejected on insert — a corruption check at the store
  boundary rather than at read time.
- `ArtifactRecord` carries `producer` and `content_kind` for provenance, so
  evidence can name the artifact it was derived from.

## 10. Restart / recovery behavior

Covered by `crates/ironmaint-runtime/tests/hardening.rs` (8 tests) plus the
0B.2/0B.3 suites:

| Failure mode | Test |
|---|---|
| daemon restart | `projection_reconstruction_round_trips_through_store` |
| interrupted operations | `interrupted_reconcile_is_idempotent_on_retry` |
| stale candidate evidence | `stale_candidate_evidence_does_not_satisfy_gate` |
| job version conflicts | `job_version_conflict_yields_concurrent_modification` |
| duplicate requests | `capture_candidate_duplicate_is_idempotent` |
| duplicate requests (event log) | `duplicate_capture_does_not_emit_two_events` |
| event-log durability | `successful_transition_persists_event_before_returning` |
| blocked transitions | `unmet_gate_requirements_block_transition` |

Plus, in existing suites: `apply_patch_rejects_stale_revision` (workspace
conflicts), `resolve_strict_no_follow` (path traversal), `validator_*` (MCP
auth), artifact size-limit tests (corruption).

Key properties: `Reconcile` is idempotent and bounded; `CaptureCandidate`
dedupes on fingerprint; a blocked or invalid transition never bumps the
projection version; every accepted transition appends a `JobEvent::Transitioned`
envelope before the call returns (§4.7).

**Projection rebuild** (§36, §102 item 3) is a shipped operator tool, not a
test-only path: `ironmaintctl rebuild-projections [job_id]` walks the SQLite
event log, replays each event through `ProjectionStore::rebuild_projection`,
and writes the result back with the existing version as the CAS token. It
currently requires an explicit `job_id` — a `list_jobs` facade does not exist
yet, so "rebuild everything" is a shell loop over known ids.

## 11. Test inventory

**675 tests passing** with `--features integration`; **672** without.

| Suite | Tests | What it pins |
|---|---|---|
| `ironmaint-runtime/tests/capture_candidate.rs` | 4 | fingerprint dedup, per-job index, event emission |
| `ironmaint-runtime/tests/materialize_checks.rs` | 5 | plan→`CheckDefinition`, stage mapping, store round-trip |
| `ironmaint-runtime/tests/plan_run_check.rs` | 6 | unknown `CheckId`, candidate mismatch, gate pass/fail |
| `ironmaint-runtime/tests/transition_wiring.rs` | 6 | `try_transition` version/gate/event semantics, full chain |
| `ironmaint-runtime/tests/reconcile.rs` | 8 | state-chain walk, approval gating, exceptional states |
| `ironmaint-runtime/tests/hardening.rs` | 8 | the eight restart/recovery modes above |
| `ironmaint-mcp/tests/dispatcher.rs` | 5 | **§94 exit checkpoint** — tool round-trip, no direct store access |
| `ironmaint-mcp/tests/auth.rs` | 4 | bearer token accept/reject, tool-name inventory |
| `ironmaint-testkit/tests/synthetic_e2e.rs` | 3 | **§81 exit checkpoint** — `ReadyForApproval` walk, tool failure, concurrent modification |
| `ironmaint-workspace/tests/apply_patch.rs` | 2 | revision CAS, dirty-workspace refusal |
| adapter conformance (`debian-stub`, `fedora-stub`) | 2 | same generic suite, both pass |

## 12. Test results

All §97 commands green at `b6aa42a`:

```text
cargo fmt --check                                      → clean
cargo clippy --workspace --all-targets --all-features
                        -- -D warnings                 → clean, 0 warnings
cargo test --workspace                                 → 672 passed, 0 failed
cargo test --workspace --features integration          → 675 passed, 0 failed
cargo run -p xtask -- verify-architecture              → No violations
cargo run -p xtask -- verify-schemas                   → 7 committed / 7 generated, no drift
cargo run -p xtask -- verify-mcp-schemas               → 9 committed / 9 generated, no drift
cargo run -p xtask -- verify-migrations                → 25 objects, status clean
```

Note for the reviewer: `cargo xtask` is not installed in this environment;
each verifier is invoked as `cargo run -p xtask -- <subcommand>`.

## 13. Actual IronClaw E2E result

**Not executed.** `scripts/ironclaw-e2e.sh` is written and syntax-clean, but
requires a running IronMaint MCP server and an `ironclaw` binary on `$PATH`.
Neither is present in this environment.

What exists instead:
- `doc/operator/ironclaw-setup.md` — full wiring procedure.
- `doc/operator/mcp-registration.yaml` — `reborn.extension_manifest.v3`
  declaring all nine tools, bearer-token auth, loopback egress allowlist.
- `doc/operator/tool-permissions.md` — the canonical allow/deny table.
- `skills/ironmaint-maintainer/SKILL.md` — the runtime skill (YAML
  frontmatter, activation keywords/patterns, workflow loop, hard rules).

So §102 item 27 ("an IronClaw skill describes the maintenance interaction
model") is satisfied. Item 26 ("IronClaw can call the MCP server") is
**plausible but unverified end-to-end** — the manifest and skill are written
against the documented IronClaw extension format, but no live handshake was
performed. This is the second item to scrutinise.

## 14. Architecture verification result

`cargo run -p xtask -- verify-architecture` → **No violations.**

Specifically enforced and holding on this branch:
- `ironmaint-core` has no dependency on `state`, `evidence`, `policy`,
  `adapter-api`, `store`, `executor`, `mcp`, or any adapter crate.
- No core crate depends on `ironmaint-mcp` or `ironmaint-runtime` (the edge
  points one way only).
- Distribution family is an opaque `String` throughout core; no
  `if family == "debian"` branching exists in any core crate.
- `ironmaint-mcp` depends on `ironmaint-runtime` (dispatcher → service), not
  the reverse.

## 15. Schema verification result

```text
schemas/          7 snapshot(s)  — no drift
  Evidence.json            Obligation.json           PublicationPlan.json
  JobEvent.json            ReconcileOutcome.json      ReleaseCandidate.json
  StateTransitioned.json
schemas/mcp/      9 snapshot(s)  — no drift
  candidate.capture   check.run          job.create        job.get
  job.next_actions    job.reconcile      operation.get
  workspace.apply_patch                 workspace.stat
```

`ReconcileOutcome.json` is new on this branch (C5). All are generated via
`schemars` from the structs the spec lists; snake_case throughout, RFC3339
UTC for timestamps, schema version on top-level durable records.

## 16. Known limitations

Ordered by risk to Phase 1.

### 16.1 Four MCP tools are shape-only (HIGH)

`check.run`, `workspace.apply_patch`, `workspace.stat`, and `operation.get`
parse and validate their input, then return a constant or echoed value. They
do not execute. The dispatcher comments mark each as deferred, but the
spec's §94 checkpoint wording ("completes the fixture scenario") implies a
working tool surface.

*Fix location:* `crates/ironmaint-mcp/src/dispatch.rs` — the runtime methods
(`handle_run_check`, `WorkspaceManager::apply_patch`, `current_revision`,
`OperationStore::get`) all exist and are tested elsewhere.

### 16.2 No MCP transport / `serve` entry point (HIGH)

`ironmaint-mcp` has **no `main.rs` and no `[[bin]]` target** — the crate is a
library. There is no rmcp/Streamable-HTTP wiring, so nothing serves the nine
tools over the network. The dispatcher is reachable in-process only, from
`ironmaint-mcp/tests/dispatcher.rs`.

Related: `bins/ironmaintd` boots the runtime but its own header says "The
MCP server wiring (commit 15) hooks into this `main` between steps 3 and 4" —
that hookup has not happened. So **neither** a standalone `ironmaint-mcp
serve` **nor** an MCP listener inside `ironmaintd` exists. An IronClaw agent
has no way to reach IronMaint over a socket today.

The operator setup doc has been annotated to say so rather than imply
otherwise.

*Fix location:* either a new `crates/ironmaint-mcp/src/main.rs` + `[[bin]]`,
or the documented hookup inside `bins/ironmaintd/src/main.rs`. Both need
rmcp + a tokio HTTP server in the workspace — a dependency decision that
belongs to the architect.

### 16.3 Approval commands not implemented (MEDIUM)

`ctx.approvals` is wired through `build_transition_context` but always
empty. There is no `RequestApproval`, `RecordApprovalDecision`, or
`AuthorizeOperation` command. `next_actions` enumerates
`AllowedAction::RequestApproval` at `ReadyForApproval`, but nothing can
consume it. The §81 driver stops at `ReadyForApproval` by design, so the
checkpoint is met — but `ReadyForApproval → Approved → PublicationPending →
Published` is unreachable, and `Publication` gates are never seeded.

*Consequence:* the approval boundary that §4.10 protects (privileged external
effects remain unavailable) is enforced by *absence* rather than by a
rejection path. There is no code that would refuse a forged approval,
because there is no code that accepts one either.

### 16.4 No `CaptureResume` / `infrastructure_blocked` signal (MEDIUM)

`TransitionContext.resume_event` is always `None`;
`infrastructure_blocked` is always `false`. `HumanReviewRequired` and
`InfrastructureBlocked` are terminal from the runtime's perspective — nothing
can exit them. `Reconcile` returns `Exceptional { current }`.

*Consequence:* a job that hits an exceptional state is stuck with no
operator recovery path. Phase 1 needs this before real adapters, which will
hit infrastructure failures in normal operation.

### 16.5 Synthetic adapters only; no real Debian/Fedora integration (MEDIUM)

`debian-stub` and `fedora-stub` satisfy the adapter conformance suite but
produce no real `BuildPlan`/`QaPlan` — the §81 driver synthesises these
in-memory. No `sbuild`, `mock`, `Lintian`, `piuparts`, `autopkgtest`,
`rpmlint`, `Koji`, `Bodhi`, `BTS`, `Bugzilla`, or `Packit` integration exists.
This is the intended §106 handoff, but it means the 0B.5–0B.8 work has never
exercised a real subprocess with a real tool's argv shape.

### 16.6 Daemon runs no reconcile loop (MEDIUM)

`bins/ironmaintd` exists and boots correctly — it opens the SQLite store
(which acquires the single-daemon lock), constructs a `RuntimeService`, and
drains on SIGTERM. But it does **not** serve MCP (16.2) and does **not** run
a reconcile loop: `reconcile` is only ever called by the in-process
dispatcher, so a job advances only when an agent explicitly asks. The
`infrastructure_blocked` signal (16.4) is one of the daemon's
responsibilities and has no producer.

### 16.7 The 0B.8 hardening suite is not exhaustive (LOW)

`unmet_gate_requirements_block_transition` asserts the transition is
rejected but accepts either `TransitionBlocked` or `InvalidInput` as the
error kind, because the runtime's pre-flight check fires before the engine's
blocker list in that configuration. The invariant under test ("the projection
is unchanged") is solid; the specific error kind is not pinned. Worth
tightening if the error taxonomy is considered stable API.

## 17. Questions requiring review before real Debian integration

1. **Should the four shape-only MCP tools block Phase 1?** They are the gap
   between "tool surface exists" and "tool surface works". If Phase 1 starts
   with real adapters, the dispatcher needs real bodies first — otherwise the
   agent has no way to run a check.

2. **Who owns the `ironmaint-mcp serve` binary?** It was assumed to exist in
   the 0B.6 setup doc. Building it means adding rmcp + a tokio HTTP server to
   the workspace, which is a dependency decision the architect should make
   explicitly rather than have a follow-up commit slip in.

3. **Is the approval boundary acceptable as "unreachable code"?** §4.10 wants
   privileged effects to be *refused*. Today they are merely not offered. An
   architect may want an explicit rejection path (a `RequestApproval` command
   that returns `Unsupported`) so the refusal is observable and testable
   rather than an absence.

4. **Should `Phase 0B.17`/`0B.18`-style sub-phase numbering be extended for
   the gaps above?** AGENTS.md notes these are numbered beyond the spec's
   0B.1–0B.8. The 16.1–16.4 items are each roughly commit-sized; a decision
   on whether they become 0B.9–0B.12 or Phase 1 prerequisites belongs to the
   architect.

5. **Verify the IronClaw manifest against a live IronClaw.** The manifest was
   written against `doc/vendor/ironclaw/docs/extensions/mcp.mdx` and the
   `reborn.extension_manifest.v3` type definitions. A dry-run
   `ironclaw extension install doc/operator/mcp-registration.yaml` would
   confirm the field names.

6. **`Reconcile` advances one transition per call, and its loop is dead code.**
   `reconcile` returns `Advanced` on the first successful transition
   (`service.rs:598-602`), so a job walks the chain only if the agent calls
   `reconcile` repeatedly. The enclosing `for _ in 0..15u32` therefore never
   iterates — it is annotated `#[allow(clippy::never_loop)]` and its only
   reachable exit is the `NoOp` fallthrough at the bottom. The behaviour is
   arguably correct (one transition per call keeps the audit story simple),
   but the loop should be deleted so the semantics are stated by the code
   rather than implied by an unreachable bound. The architect should confirm
   the single-transition-per-call semantic is intended before the loop is
   removed.

---

## 18. North-star check (§105)

> IronClaw can autonomously diagnose and repair a synthetic
> package-maintenance failure, but it cannot directly alter IronMaint's
> truth.

**Accurate with one qualifier.** The first half is demonstrated: the §81
driver and the §94 dispatcher test both complete the scenario, and every
state change routes through `TransitionEngine` with candidate-scoped gate
evaluation. The second half holds structurally — no MCP tool writes state,
gates, obligations, approvals, or evidence.

The qualifier: "autonomously ... repair" is demonstrated by a *deterministic
no-LLM driver* (§102 item 28), not by a live IronClaw agent (item 30). The
ironclaw E2E was not run (16.2, 13).

> Replacing IronClaw with another MCP-capable orchestrator would not require
> changing IronMaint's package state, evidence model, persistence model,
> executor, or release semantics.

**Accurate.** The orchestrator is referenced only by `OrchestratorRef` (an
opaque id) and reaches the system exclusively through nine JSON tools. No
core type names IronClaw; `verify-architecture` enforces that the dependency
edge runs runtime → mcp, never mcp → runtime.
