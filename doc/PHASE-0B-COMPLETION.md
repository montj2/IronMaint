# Phase 0B Completion Report

Per `doc/phases/PHASE-0B.md` §104. This is the review input for the §106
handoff gate ("review the implementation before connecting real Debian
infrastructure"). Written by the implementing agent; every claim below is
checkable against the repository.

- **Branch:** `feature/0b5-0b8-runtime-mcp-ironclaw`
- **Base:** `develop` (PR #23, Phase 0B.4)
- **Scope:** sub-phases 0B.5 – 0B.8, nine commits
- **Date:** 2026-09-28

> **Extended through 0B.10 (2026-10-01).** §§1–18 are the report as written on
> 2026-09-28, with dated amendments where a later sub-phase made a claim stale —
> each marked in place, so the original reasoning stays legible. **§19** is
> 0B.10's own section and **§20** is the §102 Definition-of-Done table, which is
> the answer to "is Phase 0B complete": **33 of 34, with item 30
> environment-blocked and its repro steps in `doc/DEBT.md` D-15.**
>
> The claims that changed most, in case you are reviewing against the original:
> §5's tool list (9 → **11**), §11–12's test inventory (now 838 unit / 847
> integration), §12's gate output, §15's schema count (9 → **11**), and §16.8 —
> the report's one HIGH, **closed**.

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

> **Amended 2026-10-01 (0B.10).** The table below is the state at 0B.5–0B.8 and
> is kept as written; the current surface is **eleven** tools, all executing
> against real subsystems. `job.resume` was added at C2 and
> `release.candidate.create` at the C5 prerequisite, and the four "partial" rows
> were closed in 0B.9 — see §16.1, which is still accurate, and §19 below for the
> current table.

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

> **Amended 2026-10-01 (0B.10).** Now **847** with `--features integration`,
> **838** without. The §101 drivers — `acceptance_scenario.rs` and
> `mcp_acceptance_scenario.rs`, three tests each — are the largest single
> addition, and they are the first tests in the workspace that assert on the
> *workflow* rather than on one component of it.

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
| `ironmaint-testkit/tests/synthetic_e2e.rs` | 3 | state-machine wiring on a `MockStore` with pre-seeded evidence — **not** an exit checkpoint; see the module doc, rewritten 0B.10 |
| `ironmaint-testkit/tests/acceptance_scenario.rs` | 3 | **§101 / §102 item 28** — the deterministic no-LLM driver, real SQLite, no pre-seeded gates |
| `ironmaint-testkit/tests/mcp_acceptance_scenario.rs` | 3 | **§101 / §102 item 29** — the same walk with no `RuntimeService` handle |
| `ironmaint-workspace/tests/apply_patch.rs` | 2 | revision CAS, dirty-workspace refusal |
| adapter conformance (`debian-stub`, `fedora-stub`) | 2 | same generic suite, both pass |

## 12. Test results

> **Amended 2026-10-01 (0B.10).** The §97 set is green at `2656528` with
> **838** unit and **847** with `--features integration`, and
> `verify-mcp-schemas` at **11**. The run below is 0B.9's, kept as written.
>
> **Amended again later on 2026-10-01**, at `8b95884`: **840** unit and **849**
> integration, both green, four verifiers clean, `fmt` and clippy `-D warnings`
> clean. The +2 is `bins/ironmaintctl/tests/rebuild_regression.rs` — two tests
> for two separately-verified defects in the §36 escape hatch, and the first
> tests in that crate to put a real job in front of the tool rather than
> exercising the no-events path.
>
> **Amended 2026-10-02**, at `ffccbd7`: **916** unit and **925** with
> `--features integration`, both green, and now **five** verifiers — `verify-seams`
> joined the §97 set. The +76 is the container and in-container work
> (`41682b2`, `13bf829`, `500e37c`) and the seam checker itself (`ffccbd7`).
> Every count above this line is kept as written; §21 records what changed.
>
> **Amended 2026-10-02**, at `50a6136`: **926** unit and **935** with
> `--features integration`, both green, five verifiers clean, `fmt` and clippy
> `-D warnings` clean. The +10 is this branch's three checks — S3's three
> capability-inventory tests, the two store-conformance drivers plus the orphan
> case S9 found, and S8's four xtask unit tests.
> **Amended 2026-10-03**, at `1e4c959`: **916** unit and **925** with
> `--features integration`, both green, `fmt` and clippy `-D warnings` clean,
> all five verifiers clean. The §97 set now runs in the
> `ironmaint/workspace` image via `make -f containers/Makefile gate` — 8 steps,
> **63 s** end to end, 1841 test executions across the two invocations.
>
> The counts here are measured, not carried forward, and the container's
> numbers were checked against the host's rather than assumed equal:
>
> | | host (macOS) | container (linux/arm64) |
> |---|---|---|
> | `cargo test --workspace` | 916 passed, 0 failed | 916 passed, 0 failed |
> | `cargo test --workspace --features integration` | 925 passed, 0 failed | 925 passed, 0 failed |
> | test names, `--list` | 894 / 902 | 894 / 902 — **`diff` empty** |
>
> The `--list` comparison is the one that matters. Totals agree, so the two
> environments run the same number of tests; the name diff shows they are the
> *same* tests, which totals alone cannot distinguish from one test replacing
> another. It was worth doing because a gate that quietly ran a smaller suite
> inside its own image would be the worst kind of defect to ship as "the
> gate" — a check that is cheaper in the place that matters.
>
> A note on the numbers themselves: they are the sum of `test result:` lines
> under `awk -F'[ ;]' '{p+=$4}'`, which counts executions, not unique test
> names. The gap between 916 executions and 894 names is doctests and
> generated cases. An earlier note in this file quoted 935 for the
> integration run; that does not reproduce on this tree and the reproducible
> figure is 925.

All §97 commands green at `bf11da7` (0B.9 C6):

```text
cargo fmt --check                                      → clean
cargo clippy --workspace --all-targets --all-features
                        -- -D warnings                 → clean, 0 warnings
cargo test --workspace                                 → 756 passed, 0 failed
cargo test --workspace --features integration          → 759 passed, 0 failed
cargo run -p xtask -- verify-architecture              → No violations
cargo run -p xtask -- verify-schemas                   → 7 committed / 7 generated, no drift
cargo run -p xtask -- verify-mcp-schemas               → 9 committed / 9 generated, no drift
cargo run -p xtask -- verify-migrations                → 25 objects, status clean
```

Counts grew from the 0B.6 report's 672/675 to 756/759: +84 across 0B.9, and
0B.10 added a further 88 — most of it the §101 drivers, which between them
assert the scenario end to end rather than one component of it at a time.
The largest blocks are `ironmaint-mcp/tests/transport.rs` (15, over a real
socket), `bins/ironmaintd/tests/startup_shutdown.rs` (13, driving the real
binary as a child process), and `bins/ironmaintd/tests/config.rs` (17).
`ironmaint-runtime/tests/approval_refusal.rs` adds 9.

Note for the reviewer: `cargo xtask` is not installed in this environment;
each verifier is invoked as `cargo run -p xtask -- <subcommand>`.

## 13. Actual IronClaw E2E result

**The script now runs; the IronClaw half still cannot execute here.**

> **Amended 2026-10-01 (0B.10).** The output below is the 0B.9 run and is kept
> as written. The script now checks for **eleven** tools — C2's `job.resume` and
> the eleventh, `release.candidate.create`, both had to be added, and a
> hardcoded tool count was the *fourth* place a stale nine survived a sub-phase
> (after `schema.rs`, `auth.rs` and `transport.rs`). The `ironclaw` half is still
> skipped with a notice, and the last paragraph's conclusion is now **wrong**:
> see §16.8, which is closed.
>
> **Amended again later on 2026-10-01.** A live daemon built from the 0B.10
> surface *was* driven, by hand, over HTTP with `tools/call` only — and that run
> found two defects no test could reach, so "not done" was costing more than the
> note said. What it proved, what it broke, and the one thing it still does not
> prove (the agent loop, i.e. row 30) are in **§20's closing section**. The
> script itself has not been re-run, because its IronClaw half skips anyway;
> the hand-driven walk covered the same server surface.

`scripts/ironclaw-e2e.sh` was rewritten in 0B.9. The previous version could
not run at all — it was a checked-in script guarded by a "STATUS" note saying
so, and it probed for a server that did not exist. It now runs against a live
daemon and passes:

```console
$ ./target/debug/ironmaintd --bind 127.0.0.1:7341 \
      --token-file /tmp/tok --state-dir /tmp/state &
$ IRONMAINT_TOKEN_FILE=/tmp/tok ./scripts/ironclaw-e2e.sh
ironclaw-e2e: no ironclaw binary on $PATH; skipping the extension checks
ironclaw-e2e: OK
  server:  http://127.0.0.1:7341/mcp (auth enforced: 401 without a token)
  surface: 9 tools advertised
  job:     01a0e9dd-... created and read back
```

It verifies, in order: that an **unauthenticated** `tools/list` is 401 (first,
because if the server were not authenticating, every later step would still
pass); the `initialize` handshake; that all nine tools are advertised and that
there are exactly nine; a `job.create` → `job.get` round-trip over the wire;
and that a failing call returns a readable tool error rather than a JSON-RPC
protocol error.

**What is still not executed:** the `ironclaw` binary is not present in this
environment, so the script's extension checks (installed, active, advertises
`job.create`) are skipped with a notice rather than silently. §95's checkpoint
— "an actual IronClaw agent can drive the fixture package to
`ReadyForApproval`" — is therefore **not met**, and not merely unverified:
§16.8 explains why no agent could, because the state machine cannot be
advanced from the tool surface at all.

So the honest status of §102:

- item 26 ("IronClaw can call the MCP server") — **now true and tested.** A
  conforming MCP client authenticates and calls all nine tools over a real
  socket. The remaining unverified piece is the IronClaw *extension manifest*
  round-trip, not the server.
- item 27 ("an IronClaw skill describes the maintenance interaction model") —
  satisfied, and the skill was corrected in 0B.9 (it described a
  `missing_approval` blocker that does not exist, an obligation tool that does
  not exist, and an `approval_id` that is never surfaced).

What exists:
- `doc/operator/ironclaw-setup.md` — full wiring procedure, rewritten
  against the real daemon in 0B.9 (the old one documented a `serve` subcommand
  that never existed, `403` where the server returns `401`, and a liveness
  probe missing the `accept` header, which gets a 406).
- `doc/operator/mcp-registration.yaml` — `reborn.extension_manifest.v3`
  declaring all nine tools. One field's comment is now known to be wrong; see
  §17 question 5.
- `doc/operator/tool-permissions.md` — the canonical allow/deny table.
- `skills/ironmaint-maintainer/SKILL.md` — the runtime skill.

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

**A second verifier now guards the seams rather than the dependency graph.**
`cargo run -p xtask -- verify-seams` (`ffccbd7`) is the fifth §97 command. It
carries four static checks — S1 (every `RuntimeCommand` variant is *constructed*
outside tests, not merely mentioned), S2 (every name `tool_for_action` returns
is a registered tool), S4 (every `JobEvent` has an arm in `ProjectionApply::apply`
and seed selection is decided in exactly one place), S5 (both store backends
implement the same supertrait). Three behavioural checks — S3, S8, S9 — are
specified in `doc/SEAM-VERIFICATION.md` and **not yet built**; §21 says what
that leaves uncovered.

Worth knowing before reading a green result: the check's first run reported six
violations and **five were bugs in the check**, because its test module had
never been compiled — `cargo test -p xtask` had not been run, and twelve of its
thirty-four tests failed. The lesson is written into the module's own doc
comment rather than left here.

## 15. Schema verification result

> **Amended 2026-10-01 (0B.10): `schemas/mcp/` is 11, not 9.** `job.resume` and
> `release.candidate.create` joined at C2 and the C5 prerequisite.
> `schemas/` is unchanged at 7.

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

> **This section is not the complete list.** `doc/DEBT.md` is the standing
> technical-debt register, audited 2026-09-29 against `0acaedc` and **re-audited
> 2026-10-01** after 0B.10. It carries four
> items this report does not (§16.x does not mention them), one of which is HIGH
> and concerns a safety invariant this document's §18 implicitly claims:
>
> - **D-01** — `ironmaint-store` and `ironmaint-store-sqlite` inherit no lint
>   set, so the no-panic / no-`unsafe` policy is unenforced in the persistence
>   layer, which contains 4 real violations. **Still open, and now the register's
>   only HIGH** — 0B.10 closed everything else it found and deliberately did not
>   touch this (`AGENTS.md`: one fix per branch).
> - ~~**D-02** — the `SKILL.md` path named by `PHASE-0B.md:2077` is a **stale
>   duplicate** documenting a `check.run` wire format the server no longer
>   accepts.~~ **CLOSED in 0B.10 C3** — `skills/` is canonical, the duplicate is
>   deleted, and `PHASE-0B.md` §61 is amended to point at the surviving file.
> - **D-04 / D-05** — `dead_code` was never promoted to `deny` as its own comment
>   promises, and 4 of 6 suppressions are vestigial. Still open.
> - ~~**D-03** below is **wider** than §16.8 records: `next_actions` also
>   advertises 3 actions (`MarkObligationSatisfied`, `AuthorizeOperation`,
>   `Publish`) that no tool can perform.~~ **CLOSED in 0B.10** — `allowed` is
>   strictly tool-performable, the human-action case has its own field, `Publish`
>   is removed, and an unperformable `AllowedAction` is now a *compile* error.
>   The census, including the two commands that gained a production caller rather
>   than a tool, is in `doc/DEBT.md` D-03.
>
> Also closed since: **D-07** (the suppressed `never_loop` — §17 Q6) and **D-14**
> (`rebuild_projection` rejected every real job's log, which is §102 item 3 — see
> row 3 of §20). **D-15** is new and is the §102 item 30 blocker.

### 16.1 ~~Four MCP tools are shape-only~~ — CLOSED in 0B.9

All four now execute against real subsystems rather than returning a constant:

| Tool | 0B.6 | 0B.9 |
|---|---|---|
| `check.run` | returned a constant | `RuntimeCommand::RunCheck` → the real executor; returns the evidence id, gate status, and tool outcome |
| `workspace.stat` | returned revision `0` | `WorkspaceManager::current_revision` — the persisted revision |
| `workspace.apply_patch` | echoed the input | `WorkspaceManager::apply_patch` — compare-and-swap, dirty-worktree refusal, real `git apply` |
| `operation.get` | echoed the input | `RuntimeQuery::GetOperation` → `OperationStore::get_operation` |

A fifth tool, `candidate.capture`, *executed* but fabricated its domain
data, which was worse than not executing. It hardcoded `"0".repeat(40)`
for both `commit` and `tree` and `"0.0.0"` for the version. Because the
fingerprint is derived from commit and tree, **two captures of the same
repository produced an identical fingerprint**, and
`handle_capture_candidate` dedupes by fingerprint — so a genuine source
update was silently discarded as a duplicate. It now reads the working
tree's real HEAD commit and tree OID. A regression test asserts that two
captures at two different commits yield two different fingerprints.

Fix locations: `crates/ironmaint-mcp/src/dispatch.rs`,
`crates/ironmaint-runtime/src/{command,query,service}.rs`.

### 16.2 ~~No MCP transport / `serve` entry point~~ — CLOSED in 0B.9

`crates/ironmaint-mcp/src/server.rs` serves the nine tools over
Streamable HTTP via rmcp, and `bins/ironmaintd` binds it on
`127.0.0.1:7341/mcp`. `ironmaint-mcp` stays a library — the daemon owns
the process and the shutdown policy — which is what keeps the crate free
of a persistence dependency (§98.6) and keeps the handler generic over
store and executor.

Four properties are load-bearing and each is tested:

- **Auth is enforced before the transport validates anything else.**
  The bearer check is axum middleware layered *outside* rmcp's service.
  rmcp answers a bad `Host`/`Accept`/`Content-Type` with 403/406/415
  before parsing a body, so a check inside the handler would only ever
  see well-formed requests and an anonymous caller could map the protocol
  surface for free. `auth_is_enforced_before_rmcp_validates_the_request_
  shape` pins the ordering by sending an unauthenticated request that
  also violates three protocol rules and asserting 401.
- **The advertised surface is generated from the same function the
  snapshot check reads** (`schema::generate_all_schemas`), so `tools/list`
  and `verify-mcp-schemas` cannot disagree.
- **Stateless**: `NeverSessionManager` plus `legacy_session_mode(false)`
  issues no `Mcp-Session-Id`, so there is no per-client state to reap.
- **A failed tool is `Ok(CallToolResult::error)`, not `Err`.** Clients
  render the former in the conversation and the latter opaquely, and the
  error body carries `{error: {kind, message}}` in both
  `structuredContent` and a text block.

Store trait futures are now declared `-> impl Future<..> + Send`
(45 methods). `rmcp`'s `ServerHandler` futures are `Send`-bounded and the
dispatcher reaches storage through those traits, so the transport could
not compile without it. `Executor::execute` has declared the same bound
since `36aa457`; the store traits had lagged it.

**Note for anyone re-deriving this:** rmcp's `local` feature is *not* a
workaround for non-`Send` futures. `transport/streamable_http_server.rs`
is gated on `#[cfg(all(feature = "transport-streamable-http-server", not(feature = "local")))]`,
so enabling `local` removes the Streamable HTTP server entirely.

### 16.3 ~~Approval commands not implemented~~ — CLOSED in 0B.9

`RuntimeCommand::RequestApproval` now exists and **always** returns
`RuntimeErrorKind::Unsupported`, with a message naming the boundary: an
approval is a human principal's decision, and the orchestrator that calls
this command must not be able to grant it. The refusal is observable and
tested rather than an absence — nine tests in
`crates/ironmaint-runtime/tests/approval_refusal.rs` pin the typed kind,
the message content, the byte-identical projection, the absence of an
appended event, and that repeated refusals accumulate nothing.

`next_actions` still advertises `request_approval` at `ReadyForApproval`.
That is not a contradiction: the runtime's *state machine* still requires
an approval record it has no way to receive, so the job genuinely stops
there and the agent's documented exit checkpoint remains truthful.

What is still true of 16.3: `RecordApprovalDecision` and
`AuthorizeOperation` do not exist, so `ReadyForApproval → Approved →
PublicationPending → Published` remains unreachable, and `Publication`
gates are never seeded. That is a *missing producer*, not a missing
refusal, and it is deferred with the rest of the privileged-operation
surface. See also §16.8.

### 16.4 No `CaptureResume` / `infrastructure_blocked` signal (MEDIUM) — **PARTLY CLOSED in 0B.10 C2**

> **What changed (2026-10-01).** "Nothing can exit them" is no longer true. The
> *exit* is complete for both exceptional states: `ResumeRecord` is a durable
> `JobEvent::ResumeRecorded`, and `job.resume` consumes it. The *entry* is half
> there — `EnterHumanReview` exists as a command, and no tool can call it, which
> is deliberate.
>
> | Half | State |
> |---|---|
> | A recorded resume state, durable and replayable | **done** — `JobEvent::ResumeRecorded`, SQLite round-tripped |
> | An exit that reads the record rather than inferring it | **done** — `job.resume`; with no record it returns a typed `InvalidInput` rather than scanning the log |
> | A way to *signal* `InfrastructureBlocked` | **still absent** |
>
> So the state now has a way out and no way in. `doc/DEBT.md` D-09 carries the
> detail; the remaining decision — who may mark a job blocked, and on what
> evidence — is a Phase 1 question, because real adapters are what will hit it.

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

### 16.6 Daemon runs no reconcile loop (MEDIUM) — narrowed in 0B.9

`ironmaintd` now boots the real daemon and serves MCP (16.2 closed), so
half of this item is done. The remaining half stands: there is no
background reconcile loop. `reconcile` is called only when an agent asks
for it, via the `job.reconcile` tool. That is defensible — the agent is
the orchestrator, and a daemon that advanced jobs behind its back would
make the audit story harder to read — but it does mean the
`infrastructure_blocked` signal (16.4) has no producer, because nothing
is watching.

Deferred to 0B.10.

> **NOT BUILT in 0B.10, and that is a decision (2026-10-01).** §41 defines
> `reconcile` as a per-call function — `runtime.reconcile(job_id)`, called by the
> caller, returning where it stopped. §67's startup sequence has no reconcile
> step, and no spec text asks for a loop. The supporting argument is in
> `doc/DEBT.md` D-10: an agent is the orchestrator here, and a daemon that
> advanced jobs behind its back would put state changes in the audit log that no
> actor requested.
>
> The last sentence above is now **stale and is the interesting part** — "a
> background loop would have nothing to do anyway" was true while D-03 was open
> and stopped being true the moment C1 landed. A limitation written as a
> *consequence* of another defect outlives the defect and starts to read as a
> requirement. Revisit when there is a first consumer, and name it in Phase 1's
> plan.

### 16.7 The 0B.8 hardening suite is not exhaustive (LOW)

`unmet_gate_requirements_block_transition` asserts the transition is
rejected but accepts either `TransitionBlocked` or `InvalidInput` as the
error kind, because the runtime's pre-flight check fires before the engine's
blocker list in that configuration. The invariant under test ("the projection
is unchanged") is solid; the specific error kind is not pinned. Worth
tightening if the error taxonomy is considered stable API.

### 16.8 A job cannot advance past `EventDetected` (HIGH) — found in 0B.9 — **CLOSED in 0B.10**

> **Closed 2026-10-01.** The text below is the finding as recorded on 2026-09-29,
> kept as written because the closure is only legible against it. Summary: C1
> gave the three orphaned commands one production caller each (driven by
> `candidate.capture`, via the adapter registry the daemon now loads at startup);
> C3 split the `next_actions` wire; and C5 proved the whole of §101 runs through
> `dispatch()` with **no `RuntimeService` handle** — the acceptance criterion this
> entry itself set for Phase 1, satisfied one sub-phase early. `doc/DEBT.md` D-03
> carries the re-derived census and the two further defects the driver found.
>
> **Nothing below is still true of the code.** The greps, the `no_op` loop, the
> "the wall is two deep", and the three options (a)/(b)/(c) are all historical.
> Option (a) was taken, and (b) turned out not to conflict with §94, which
> enumerates tool *capabilities* rather than a count.

> **Decision (architect, 2026-09-29): option (c) — leave it to the Phase 1
> adapter.** 0B.10 is `CaptureResume`/`infrastructure_blocked` (§16.4), the
> daemon reconcile loop (§16.6), and the privileged-operation producers.
> §16.8 closes when the adapter that already has to exist for §106
> materialises checks and activates a candidate. Options (a) and (b) are
> not taken, so the nine-tool enumeration in §94 stands unchanged.

> **Decision (architect, 2026-09-29): option (c) — leave it to the Phase 1
> adapter.** 0B.10 is `CaptureResume`/`infrastructure_blocked` (§16.4), the
> daemon reconcile loop (§16.6), and the privileged-operation producers.
> §16.8 closes when the adapter that already has to exist for §106
> materialises checks and activates a candidate. Options (a) and (b) are
> not taken, so the nine-tool enumeration in §94 stands unchanged.
>
> *The risk of this choice, stated plainly so it is not rediscovered as a
> surprise:* Phase 1's premise is "real adapters meeting real failures", and
> §16.8 is precisely where those two do not meet. Phase 1 therefore opens
> with the state machine unexercisable through the tool surface. The
> adapter must activate the candidate as part of its first commit or the
> premise is not actually being tested — that is the acceptance criterion
> for Phase 1's first adapter, and it should be stated in Phase 1's own
> plan rather than left here.

This is the most consequential finding of 0B.9 and it was not on the
list before. Everything in 16.1–16.3 is now fixed and all nine tools do
real work — and **no agent, human or LLM, can drive a job to
`ReadyForApproval`**, because the two runtime commands that would move it
are called from tests and nowhere else.

Verified by grep and then by running it:

```console
$ grep -rn 'SetActiveCandidate\|MaterializeChecks' crates/ bins/ | grep -v tests/
(no matches)
```

Both are invoked *only* from `crates/ironmaint-runtime/tests/`,
`crates/ironmaint-mcp/tests/`, and `crates/ironmaint-testkit/tests/`.

The chain, as it actually runs against a live daemon:

1. `job.create` → the job sits at `event_detected`.
2. `job.next_actions` → `allowed: ["capture_candidate"]`, no blockers.
3. `candidate.capture` → succeeds, and mints a **real** fingerprint from
   the working tree's actual HEAD commit and tree OID.
4. `job.next_actions` → **still** `allowed: ["capture_candidate"]`. The
   projection is unchanged, because `handle_capture_candidate` persists
   the candidate and appends an audit event but never sets
   `projection.active_candidate`.
5. `job.reconcile` → `{"outcome": {"no_op": {"current": "event_detected"}}}`
   forever, because `reconcile` refuses to evaluate any rule when there
   is no active candidate (`service.rs:665-668`).

And even if an active candidate were set, the first rule —
`EventDetected → Intake` — requires the `SourcePreparation` gate to
pass, and gate definitions only come from
`RuntimeCommand::MaterializeChecks`, whose input is an adapter
`PlannedCheck`. There is no adapter in Phase 0B (16.5). So the wall is
two deep, and the second wall is the real one.

**Why this is HIGH and not MEDIUM.** §95's exit checkpoint is "an actual
IronClaw agent can drive the fixture package to `ReadyForApproval`". That
checkpoint is unreachable, and the §102 table reports it as met. Every
tool in the surface works; there is simply no sequence of tool calls that
advances the state machine. An agent following `SKILL.md` faithfully will
capture a candidate, reconcile, get `no_op`, and have no way to
distinguish "the package is fine" from "nothing here can ever move".

It was hidden because the `job.create → job.get` round-trip succeeds and
`SKILL.md` describes a workflow that reads correctly — the missing link
is a step nobody had cause to execute.

**This is a scope decision, not an oversight, and 0B.9 did not make it.**
Closing it means one of:

- **(a) `candidate.capture` also sets the active candidate.** Smallest
  change; the capture is the natural moment. But it collapses a
  deliberate two-step into one, and the runtime currently keeps
  `CaptureCandidate` and `SetActiveCandidate` separate for a reason that
  §41 does not state.
- **(b) A tenth tool, `candidate.activate`.** Keeps the commands
  separate and honest. But §94 enumerates nine, and the schema
  snapshot, the manifest, `SKILL.md`, and the e2e script all hardcode
  nine.
- **(c) Leave it to the adapter.** The adapter that materialises checks
  would also activate the candidate. This is the shape Phase 1 implies,
  and it means the gap closes on its own — but only after Phase 1
  starts, and Phase 1's premise is "real adapters hitting real
  failures", which is exactly what this gap prevents from being tested.

Worth an architect's decision before Phase 1. Options (a) and (c) are
compatible; (b) is not compatible with §94 without a spec amendment.

**Post-merge note (2026-09-29, revised 2026-09-30).** A follow-up audit
(`doc/DEBT.md` D-03) widened this. The census is not two commands but **5 of 9
`RuntimeCommand` variants and 1 of 5 `RuntimeQuery` variants** with no MCP entry
point. `next_actions` also emits `MarkObligationSatisfied`, `RequestApproval` and
`AuthorizeOperation` into `allowed` for which no tool exists.

An initial reading called that "the runtime offers the agent a verb it does not
have," which overstated it: `SKILL.md` rules 7–8 tell the agent these are *not*
callable and to stop at `ReadyForApproval`, and two tests pin that. The agent is
not misled. The genuine defect is that `next_actions.rs:1-12` documents `allowed`
as "the `AllowedAction`s it can take right now" while the field is carrying two
meanings at once.

**Decided 2026-09-30:** `allowed` becomes strictly tool-performable, the
human-action case moves to its own field, an enforcement test makes an
unperformable `AllowedAction` a gate failure, and the never-emitted `Publish`
variant is removed. Behaviour is unchanged — the agent still stops at
`ReadyForApproval`. Lands at the top of 0B.10.

---

## 17. Questions requiring review before real Debian integration

**1. ~~Should the four shape-only MCP tools block Phase 1?~~ Answered in
0B.9: yes, and they did. All four execute for real now (16.1 closed), plus
a fifth that executed while fabricating its data. Phase 1 is unblocked on
this axis.**

**2. ~~Who owns the `ironmaint-mcp serve` binary?~~ Answered in 0B.9: the
daemon. `ironmaint-mcp` remains a library and the transport inside it stays
generic over store and executor, so the crate keeps no persistence
dependency (§98.6). No new dependency was needed — `rmcp`, `axum`,
`tracing-subscriber`, and `tokio-util` were all already declared in the
root `Cargo.toml` for this purpose. The CLI parser and the config struct
are hand-rolled, because §86's list of new dependencies does not include a
CLI or TOML parser, and every value the daemon reads is covered by a flag
or a variable.**

**3. ~~Is the approval boundary acceptable as "unreachable code"?~~ Answered
in 0B.9: no, and it is now explicit. `RequestApproval` returns a typed
`Unsupported` with a message naming the boundary, tested for the error
kind, the message, the unchanged projection, and the absence of an
appended event (16.3 closed). `RecordApprovalDecision` and
`AuthorizeOperation` are still absent — that is a missing *producer*, and
it is deferred rather than answered.**

**4. ~~Should the sub-phase numbering be extended?~~ Done: 0B.9, seven
commits, PR against `develop`. §16.4, §16.6, and the privileged-operation
producers are what 0B.10 is for.**

**5. Verify the IronClaw manifest against a live IronClaw. STILL OPEN.**
`doc/operator/mcp-registration.yaml` was written against
`doc/vendor/ironclaw/docs/extensions/mcp.mdx` and the
`reborn.extension_manifest.v3` type definitions, and its nine tool names
are verified against the live server by `scripts/ironclaw-e2e.sh` — but
the *manifest field names* have never been round-tripped through a real
`ironclaw extension install`. Needs a machine with the binary.

One field is now known to be a lie and should be corrected there:
the manifest's `auth` comment says "the bearer token is read fresh on
every call by IronClaw's egress layer; rotating the token file is enough
to invalidate every active session." The server reads the token **once at
startup** and compares per request, so rotating the file does nothing
until the daemon restarts. The comment overstates the revocation story.

**6. ~~`Reconcile` advances one transition per call, and its loop is dead
code.~~ ANSWERED 2026-10-01, in 0B.10 C4 (`cec87d7`).** The semantic question
was settled by evidence already in the tree, which this entry had not looked at:
**§41** says reconciliation "may continue through multiple trivially satisfied
stages", and **`ReconcileOutcome::Advanced`'s own doc comment** already promised
"the projection advanced through **one or more** rules". The code contradicted
its own type documentation. `reconcile` now walks every satisfied rule and stops
where the next is blocked, and the `#[allow(clippy::never_loop)]` is
**deleted** rather than re-suppressed.

Worth carrying forward: the suppression was a design decision deferred to a
future reader, written in the wrong place. Someone read the lint and silenced it
instead of reading it. This is the second time in this phase that a suppressed
lint has been a design note in disguise — **grep `#[allow(clippy::` and read each
argument.** `doc/DEBT.md` D-07 carries the full note.

It is worth noting in the meantime that **the loop is not the reason
§16.8 happens.** `reconcile` returns `NoOp` on its first iteration for a
job with no active candidate, long before the loop bound could matter.

**7. ~~How should a job advance past `EventDetected` while there is no
adapter?~~ ANSWERED 2026-09-29: option (c), leave it to the Phase 1
adapter.** See the decision block at the head of §16.8. The consequence
for Phase 1 is written down there rather than left implicit: the first
real adapter must activate the candidate, or Phase 1 is not exercising
what it claims to. §94's tool *capabilities* are unchanged. (The count
is now ten: 0B.10 C2 added `job.resume`, because an agent that lands in
`HumanReviewRequired` otherwise had no way out and §101 could not be
driven end to end over MCP.)

> **Update 2026-10-01 (0B.10 C5): the criterion is already met, and is now a
> regression test.** `crates/ironmaint-testkit/tests/mcp_acceptance_scenario.rs`
> drives §101 from `job.create` to `ReadyForApproval` with every step a
> `dispatch()` call and no `RuntimeService` handle — the criterion above, exactly
> as written. Phase 1's first adapter therefore inherits a test that fails the
> moment it cannot be reached through the tool surface, which is a stronger
> position than the one this question was asking for. (The count is now
> **eleven**; `release.candidate.create` landed at the C5 prerequisite, for
> §101 steps 26–27.)

**8. Who may put a job into `HumanReviewRequired` or `InfrastructureBlocked`,
and on what evidence?** **OPEN. This is the one question in this section that
Phase 1 cannot start without an answer to, and it is a state-ownership
decision.**

It is the same question asked three times, and each asking found the same hole:

- `RuntimeCommand::EnterHumanReview` is implemented, dispatched, and works.
  `handle_enter_human_review` writes the `ResumeRecord`, and `job.resume`
  consumes it. **The only thing missing is a caller** — the sole construction
  site in the tree is `crates/ironmaint-mcp/tests/dispatcher.rs:236`. No
  registered tool builds it. So `job.resume` is a tool that cannot succeed on
  any job a real agent can reach (D-20 part 2).
- `InfrastructureBlocked` has the mirror-image shape. D-09 records the *exit*
  as complete and the *entry* as absent: `ResumeRecord` is persisted and
  `job.resume` returns the job from either exceptional state, but nothing
  writes the transition into the first one.
- The intent is documented and deliberate — *"escalation is an orchestration
  decision, and an agent that could raise it against itself could also strand
  itself"* (`mcp/tests/dispatcher.rs:204`), and 0A §21 gives the state no entry
  point at all. **Both are correct answers to a question Phase 0 was not asked
  to answer.** The gap is that nobody has since been asked it.

Why this blocks Phase 1 specifically: the human-review loop is what a real
adapter's escalation path terminates in, and **state ownership is on §106's
do-not-redesign list.** If the first adapter needs an escalation entry point
and there is none, Phase 1 either waits or invents one — and inventing one
under deadline is exactly the redesign §106 says would mean 0B failed.

Three candidate answers. **These are options, not a recommendation** — the
choice is architectural, and the deciding constraint is one only the architect
can weigh.

> **(a) An operator tool the agent cannot call.** A registered MCP tool,
> alongside `RequestApproval`'s refusal, that only a human can invoke. This
> matches the principle already established for approvals — the agent may not
> self-grant — and it is the only option where "who" is settled by
> construction rather than by policy. *Does not change:* the state machine, the
> `ResumeRecord`, `job.resume`, or the projection. *Costs:* a new tool, a new
> authorisation surface, and a decision about whether the daemon can authenticate
> an operator at all — which it currently cannot, and which is the same missing
> piece that makes `RequestApproval` a refusal (D-20 part 1).

> **(b) Adapter-derived and deterministic.** The adapter's plan declares that
> a particular check *requires* human judgement, and the runtime escalates when
> the plan says so rather than when an agent asks. Escalation becomes a
> property of the distribution's requirements, which is the same shape as
> `Obligation` and keeps the decision in the engine. *Does not change:* the
> state machine or `job.resume`. *Costs:* a new field in `AdapterCapabilities`,
> a schema change, and a path where a distribution's policy can strand a job —
> which needs its own "who un-strands it" answer, and lands back on (a).

> **(c) A `reconcile` rule.** §41's rule list grows one: a job that has held
> the same state across N reconcile passes with no progress escalates. This
> keeps the decision in the state machine, next to the rules already there, and
> needs no new tool. *Does not change:* the state machine. *Costs:* a
> notion of progress or elapsed time that `JobProjection` may not currently
> carry, and it is in tension with D-10 — `reconcile` is per-call with no
> background loop, so "N passes" needs a loop that Phase 0 deliberately did not
> build. This option is the one that would re-open a recorded decision.

The three are not exclusive, and the cheapest honest answer may be "operator
tool now, adapter-derived later" — but that should be a decision, not a drift.

**What is not at stake.** Nothing here is broken. `EnterHumanReview` and
`InfrastructureBlocked` both have working handlers, a working exit, a working
projection and passing tests. The code is correct; what is missing is a
decision about who is allowed to start it.

---

## 18. North-star check (§105)

> IronClaw can autonomously diagnose and repair a synthetic
> package-maintenance failure, but it cannot directly alter IronMaint's
> truth.

> **Re-verified 2026-10-01 (0B.10). The qualifier below is now narrower, and
> the gap it described is closed.** Read the amended verdict before the original:
>
> - *"demonstrated at the API level and **not** at the tool level"* — **no longer
>   true.** §101 now runs end to end through `dispatch()` with no
>   `RuntimeService` handle (`mcp_acceptance_scenario.rs`), which is the tool
>   level the sentence is about.
> - *"not by a live IronClaw agent (item 30)"* — **still true, and the only
>   reason a qualifier remains.** No `ironclaw` binary exists in this
>   environment; `doc/DEBT.md` D-15 carries the verified repro.
>
> So: the first half is demonstrated at the tool level, by a conforming MCP
> client rather than by IronClaw specifically. The second half is unchanged, and
> is the half the sentence is really about.

**Accurate with one qualifier.** The first half is demonstrated: the §81
driver and the §94 dispatcher test both complete the scenario, and every
state change routes through `TransitionEngine` with candidate-scoped gate
evaluation. The second half holds structurally — no MCP tool writes state,
gates, obligations, approvals, or evidence.

The qualifier: "autonomously ... repair" is demonstrated by a *deterministic
no-LLM driver* (§102 item 28) calling the runtime directly, not by a live
IronClaw agent (item 30) and not by an agent going through the MCP tool
surface. The driver works because it calls `SetActiveCandidate` and
`MaterializeChecks` itself; no agent can, because no tool exposes them
(§16.8). So the north-star is demonstrated at the API level and **not** at
the tool level, which is the level the sentence is actually about.

> Replacing IronClaw with another MCP-capable orchestrator would not require
> changing IronMaint's package state, evidence model, persistence model,
> executor, or release semantics.

**Accurate.** The orchestrator is referenced only by `OrchestratorRef` (an
opaque id) and reaches the system exclusively through eleven JSON tools. No
core type names IronClaw; `verify-architecture` enforces that the dependency
edge runs runtime → mcp, never mcp → runtime.

**This second claim is the one 0B.10 turned from a claim into a test.** "Reaches
the system exclusively through N JSON tools" is a statement about the *wire*, and
the 0B.9 test suite could only assert it by holding a `RuntimeService` and
noting that it didn't need it. `mcp_acceptance_scenario.rs` asserts it by
construction: it completes the entire §101 workflow holding nothing else.

---

## 19. Sub-phase 0B.10 — making §101 real (2026-09-30 → 2026-10-01)

0B.1–0B.9 built the parts. **0B.10 checked the list, and found that three of the
thirty §101 steps were not merely untested but unrepresentable.** That is the
finding worth carrying, and it is why this section leads with it rather than with
the commit list.

**What the 0B.9 completion report claimed, against what a walk of §101 needed:**

| §102 item | Claimed at 0B.9 | Actually true |
|---|---|---|
| 28 — a deterministic no-LLM driver completes the workflow | met | the *only* driver pre-seeded twelve passing gates; it exercised no failure, no repair, and no evidence invalidation |
| 29 — an MCP-client test completes the workflow | not met | no test in `crates/ironmaint-mcp/tests/` even mentions `ReadyForApproval`; all four hold a `RuntimeService` |
| 31 — the final state is `ReadyForApproval` | met | true only for the pre-seeded path |

Step 18 is the sharpest case. §101 requires a **mandatory policy obligation to
fail** — and there was no production writer of any obligation status other than
`Pass`. Not "untested": unrepresentable. `RecordObligationOutcome` is that writer
(`8d91ad6`), and it is deliberately **off** MCP, because §102 item 25 forbids
exactly this.

The same shape then recurred eight more times — **twelve in all by 0B.10's
end**, and then two more, both in the **operator** surface rather than the MCP
one, on the live daemon run recorded in §20. **Fourteen in all.** It is the
single most important thing this section has to say:

> **A component modelled something correctly, and no production path produced the
> input.** Not one of the fourteen was a bug *inside* a component. Every one was
> a missing seam between two things that were individually correct and
> individually tested — and no amount of unit testing inside a component finds a
> missing seam.

### The method that found them

D-14 (`rebuild_projection` rejected **100%** of real jobs) shipped in 0B.9
because every pre-existing rebuild test hand-assembled the log it asserted on.
The same shape was visible in `synthetic_e2e.rs`, which pre-seeded its gates.

The method that found the rest: **drive the real command surface against a real
store, and assert on what the store says afterwards.** The second half matters as
much as the first — a mock-backed suite cannot see the production symptom, which
is why `MockStore::put_source_candidate` silently minting a second row passed CI
while SQLite raised `UNIQUE constraint failed: source_candidates.fingerprint` to
an agent mid-conversation.

### The commits

| Commit | What |
|---|---|
| C1 | `AdapterRegistry` on `RuntimeService`; `candidate.capture` drives activate + materialise + derive; the daemon loads the registry at §67 startup |
| C2 | `ResumeRecord` given somewhere durable to live (`JobEvent::ResumeRecorded`), `EnterHumanReview` / `ResumeJob`, and the tenth tool `job.resume` |
| C3 | the `next_actions` wire split; enforcement promoted to a compile error; `SKILL.md` made canonical and the stale duplicate deleted (D-02) |
| — | `RecordObligationOutcome` (`8d91ad6`) and D-14 (`0b33c3d`), both C4 prerequisites, landed first and separately |
| C4 | `reconcile` walks every satisfied rule (`cec87d7`); the repair window moves to `ReadyForApproval` (`e14caac`); `ScenarioAdapter` + `acceptance_scenario.rs` (`7aac374`) |
| `8138f37`, `d98d780` | `CreateReleaseCandidate` and the eleventh tool `release.candidate.create`, for §101 steps 26–27 |
| `ec9e06c` | eleventh instance — re-capturing an unchanged tree must not mint a second candidate |
| `f009e2e` | twelfth instance — `next_actions` must report an outstanding obligation at every state, not only where a `GatePending` blocker happened to fire |
| C5 | `mcp_acceptance_scenario.rs` — §101 through `dispatch()`, no `RuntimeService` handle (`2656528`) |
| C6 | the register and this report (`9b159d1`) |
| `8b95884` | thirteenth and fourteenth instances — `rebuild-projections` took its CAS token from the wrong projection, and the log cannot justify `active_candidate`. Found by running the binary against the live daemon, not by reading the code. **D-16** |

### Three things the plan did not predict

1. **Where the driver could live.** §98.6 forbids `ironmaint-mcp` from depending
   on a store backend in **either** dependency kind. The rule is about the
   crate, not about whether the caller is a test, so it was not weakened; the
   driver went to `ironmaint-testkit/tests/`, which §98 already blesses.
2. **A tenth tool was spec-compatible after all.** `crates/ironmaint-mcp/src/schema.rs`
   had a test asserting exactly nine tools, commented that a tenth "should be a
   deliberate edit to this assertion, not a surprise." It was — and the guard's
   *premise* was wrong. §94 lists **capabilities**, not a count. The guard was
   corrected in place, not removed.
3. **A failed mandatory obligation must not divert the job.** 0A §21 says
   orchestration "*may*" move a job to an exceptional state. Automatic diversion
   would have made §101's own repair loop unreachable for the agent that is
   supposed to perform it, so entering is explicit and has **no** tool.

### What 0B.10 deliberately did not do

- **Privileged-operation producers** — §4.10, §26 and §99 forbid them. Not
  deferred: building them would produce code the acceptance scenario must never
  exercise.
- **A daemon reconcile loop** — §41 defines `reconcile` per-call. See §16.6 and
  `doc/DEBT.md` D-10.
- **Building `ironclaw`** — toolchain mismatch and, more to the point,
  `CLAUDE.md` designates the submodule a reference. See §20 and D-15.
- **D-01 / D-04 / D-05 / D-08** — real, and unrelated. `AGENTS.md`: one feature
  or fix per branch. D-01 is the register's only remaining HIGH and wants its
  own clean review.

---

## 20. §102 Definition of Done — status at `2656528`

> **Amended 2026-10-01 at `8b95884`.** The count is unchanged — 33 of 34 — but
> **row 3 is now qualified rather than flatly met**, because the live daemon run
> proved the event log cannot justify `active_candidate`. A row reading "met"
> with a defect sitting directly behind it is worse than a row reading
> "met, with a stated limit": D-14 was marked CLOSED for three days on exactly
> that ambiguity, and the thing behind it turned out to be a second defect. The
> closing section of this § has also been rewritten — it previously said the
> live run had not happened.

**33 of 34 are true. One cannot be checked in this environment.**

This table is the answer to "is Phase 0B complete", and it is written so that the
one that is not can be told apart from the one that was never attempted.

| # | §102 item | Status | Evidence |
|---|---|---|---|
| 1 | durable SQLite persistence | **met** | `ironmaint-store-sqlite`; `verify-migrations` → 25 objects, clean |
| 2 | domain events are append-only | **met** | no update or delete path to `job_events`; `hardening.rs` |
| 3 | projections can be rebuilt from events | **met, with a stated limit** — was **broken for every real job** until 0B.10 | D-14 / `0b33c3d`; `projection_rebuild.rs` drives the real `CreateJob` path. **Amended 2026-10-01:** true for a job's birth and for every transition, and **not** for `active_candidate` — the log records activation as a bare `JobEvent::Domain(uuid)`. The §36 tool now refuses rather than writing a projection whose evidence chain it has invalidated; the durable event variant that would close it is **D-16** |
| 4 | jobs survive daemon restart | **met** | §101 steps 4–5, over real SQLite in both drivers |
| 5 | candidates survive daemon restart | **met** | §101 step 5 |
| 6 | evidence survives daemon restart | **met** | `hardening.rs` |
| 7 | artifacts are content-addressed and integrity checked | **met** | `ArtifactStore`; corruption tests |
| 8 | workspaces are isolated | **met** | `WorkspaceManager`; per-job roots |
| 9 | workspace mutations use optimistic revision checks | **met** | `apply_patch_rejects_stale_revision` |
| 10 | path traversal is blocked | **met** | `resolve_strict_no_follow` |
| 11 | dirty workspaces cannot generate authoritative evidence | **met** | `WorkspaceState.dirty` guard; §43 note asserted at every capture in the C5 driver |
| 12 | candidates can be captured from workspaces | **met** | `capture_candidate`; §101 step 3 |
| 13 | checks execute only through registered capability definitions | **met** | `ToolRegistry`; no arbitrary-executable path |
| 14 | the agent cannot provide arbitrary executables or shell commands | **met** | `check.run` takes a `check_id`, never a command line |
| 15 | process failures are distinct from infrastructure failures | **met** | `GateResult::Fail` vs `InfrastructureFailed`, kept distinct by the type system |
| 16 | check operations are persisted | **met** | `OperationStore`; `interrupted` recovery |
| 17 | interrupted executions recover as `Interrupted`, not fabricated `Fail` | **met** | `interrupted_reconcile_is_idempotent_on_retry` |
| 18 | evidence is generated by trusted normalizers | **met** | the §92 synthetic tool set |
| 19 | evidence remains candidate-bound | **met** | §30 fingerprint binding on every `EvidenceRecord` |
| 20 | old evidence cannot authorize new candidates | **met** | asserted twice in the C5 driver: a fresh candidate's gate returns `NotFound`, and `old_evidence_cannot_authorize_a_new_candidate` |
| 21 | runtime can deterministically calculate next actions | **met** | `project_next_actions` is pure; C3 made `allowed` mean one thing |
| 22 | runtime can deterministically reconcile state | **met** | one call walks every satisfied rule until blocked (D-07) |
| 23 | MCP is authenticated | **met** | bearer check is axum middleware *outside* rmcp; §94 exit checkpoint |
| 24 | MCP tools expose domain capabilities, not storage primitives | **met** | no tool names a table, a row, or a store method |
| 25 | MCP cannot directly set state, gates, obligations, approvals, or evidence | **met** | no tool has a status-writing parameter. `RecordObligationOutcome` is a command, **not** a tool — deliberately |
| 26 | IronClaw can call the MCP server | **met** | a conforming MCP client authenticates, calls all eleven tools, and drives a job to `ReadyForApproval`. The *extension manifest* round-trip is D-06, still open |
| 27 | an IronClaw skill describes the maintenance interaction model | **met** | `skills/ironmaint-maintainer/SKILL.md`, canonical since C3 (D-02) |
| 28 | a deterministic no-LLM driver completes the synthetic workflow | **met** | `acceptance_scenario.rs` — all 30 §101 steps, no pre-seeded gates |
| 29 | an MCP-client integration test completes the synthetic workflow | **met** | `mcp_acceptance_scenario.rs` — the same walk, `dispatch()` only |
| **30** | **an actual IronClaw agent can complete the synthetic repair workflow** | **NOT VERIFIABLE HERE** | **environment-blocked**, not unmet. No `ironclaw` binary; the submodule pins Rust 1.98.0 against our 1.88 MSRV. Full repro in `doc/DEBT.md` D-15 |
| 31 | the final state is `ReadyForApproval` | **met** | asserted by both drivers, over real SQLite, after a restart |
| 32 | no real Debian or Fedora maintenance tool is required | **met** | the scenario adapter is a testkit fixture; the daemon's registry is the §92 set |
| 33 | no release credential exists in the agent environment | **met** | no producer for approvals or privileged operations exists at all (§4.10) |
| 34 | all 0A architecture invariants continue to pass | **met** | `verify-architecture` → no violations. Note that the §98.6 edge rule (`ironmaint-mcp` → `ironmaint-store-sqlite`, forbidden in **both** dependency kinds) pre-dated 0B.10 but had never been exercised by a test; 0B.10's C5 driver is what proved it, by having to live in `ironmaint-testkit` |

### Reading row 30 honestly

Row 30 is not "unmet" and it is not a code defect. Every other row that a live
IronClaw agent would exercise **is** exercised without one: the C5 driver is a
conforming MCP client that holds no runtime handle and completes §101 end to end.
What row 30 adds is IronClaw's own agent loop and its extension manifest.

So the accurate sentence is: **the workflow is proven agent-drivable; the
specific agent named by the spec has not been run against it.** A green §97 gate
and a green C5 do not silently stand in for it — which is what `doc/DEBT.md` D-15
is for.

### The end-to-end run, and the two defects it found

> **Amended 2026-10-01.** This section previously said the live run had *not*
> been done. It has. What follows replaces that.

The C5 claim is "no `RuntimeService` handle", which a test process can honour
and a reviewer should still want confirmed over a socket. So a daemon was built
from this branch, started on a real port, and driven with nothing but
`tools/call` over HTTP — real auth, real process, real SQLite state directory.

**What it proved, server-side:**

- `candidate.capture` against the real `debian` adapter activates the candidate,
  materialises 4 checks, derives 2 obligations, marks the workspace clean, and
  reports each of those in `notes`. C1's wiring is not a claim about a mock.
- `next_actions` advertises four `run_check` verbs — the C3 `allowed` /
  `requires_human` split behaving correctly on the wire.
- `check.run` on a real materialised check returns a **typed, diagnosable**
  error — `no tool registered for capability debian.build.sbuild` — instead of
  the old behaviour, which was a job that silently never left `EventDetected`
  and told the agent nothing. The adapters plan `debian.*`; only the
  `synthetic.*` fixtures are registered. That is the honest end of 0B and it is
  now *legible* rather than silent, which is what the C1 note asked for.
- `reconcile` returns a real `blocked: MissingGate(GateId(...))` rather than an
  empty result.
- The daemon log reads `adapter registry loaded families=["debian", "fedora"]`,
  and the `deferred to Phase 1` line is gone. §67's startup step is real.
- A job survived a real daemon restart with its active candidate intact.

**What it found — two defects, both in code no test could reach.** Driving
`ironmaintctl rebuild-projections` against that same live state directory:

1. the tool took its CAS token from the *rebuilt* projection rather than the
   stored row, so it could never write back for any job that had advanced —
   `Conflict: expected_version=0, found=1`;
2. fixing that turned a loud failure into silent data loss, because
   `activate_candidate` appends a bare `JobEvent::Domain(uuid)` and a replay
   cannot recover the active candidate. The write would have succeeded, reported
   `rebuilt: 1 job(s)`, and dropped the §30 binding every gate verdict depends on.

Fixed in `8b95884`; the second is **D-16**, mitigated by a refusal rather than
closed. Full detail in the commit and in `doc/DEBT.md`.

**The lesson is the reason the run was worth doing, and it is the same one C4's
notes generalised.** Neither defect was a bug *in* a component. Both were
missing seams between components that were each correct and each individually
tested. `rebuild_projection`'s tests hand-built their log (D-14);
`rebuild_smoke.rs` exercises the no-events path — the one path that always
worked; `projection_rebuild.rs` never writes its result back, so it never meets
the CAS check that made the tool unusable. Three components, no missing logic,
and no test in the workspace capable of finding it.

**What this run does *not* prove.** It was a hand-driven walk, not
`scripts/ironclaw-e2e.sh` — whose IronClaw half still skips with a notice when no
binary is present, so a green run of *it* proves the server and not the agent.
Row 30 is unaffected: the agent loop and the extension manifest still have not
been exercised, and D-15's repro steps still stand.

---

## 21. Post-0B.10 amendment — 2026-10-02

Everything above this line is accurate as of the commit named in each section,
which for §20 is `8b95884`. This section records the five sub-phases that have
landed since, and — more to the point for a Phase 1 reader — what they changed
about the handoff.

### 21.1 What landed

| Commit | PR | What |
|---|---|---|
| `13bf829` | #34 | `ironmaint/base:0.1` and `ironmaint/workspace:0.1` images, Dockerfiles, Makefile, per-image smoke tests (D-17) |
| `41682b2` | #35 | the four remaining images: `debian-tools`, `fedora-tools`, `agent`, `fake-services` |
| `500e37c` | #36 | D-18 — one symptom, two independent causes, and a third found while verifying. In-container §97 green for the first time |
| `79a7a2a` | #37 | daemon teardown in tests: `kill_on_drop`, an awaited `shutdown()`, and a test that pins the reap (D-19's found half) |
| `ffccbd7` | #38 | `verify-seams` — static checks S1, S2, S4, S5, in the §97 set |
| this PR | — | `CLAUDE.md` rewritten: it described a repository with no code in it |
| this PR | — | D-20's two false doc comments corrected, and the question they were hiding posed as §17 question 8 |
| this PR | — | S9 store parity — **found D-21, a live agent-reachable divergence** |
| this PR | — | S3 capability inventory — eight unregistered keys, each with the phase that supplies it |
| this PR | — | S8 tool reachability — the direction S2 declined, plus `pub` dispatch helpers no arm reaches |

§97 on that tree: **926** unit, **935** integration, five verifiers, `fmt` and
clippy `-D warnings` clean — walked end to end rather than inferred from the
parts already green, because a gate assembled from earlier partial runs is not
a gate.

**The behavioural half of the seam work turned out to be the productive half.**
S9's script found D-21 on its first run, and nothing in the register, the phase
report or a code read had caught it — because the mock *is* the gate for every
test of the path it breaks. Both new checks also got broken on purpose before
they were trusted (`026e559`, `a34e0e9`, `5f2076a`), and **two of those breaks
corrected a claim the check's own documentation had made** — S3's failure
reported one key per run, and S8's doc promised a reachability the workspace
`dead_code` lint already provided for private helpers. That is recorded in
`doc/EXECUTION-PLAN.md` §3 because it is the part of the teeth-check that is not
about the check.

### 21.2 The register, and who owns each open row

`doc/DEBT.md` carried twenty rows; ten were closed by those five commits and
their predecessors. **Two more were added while writing this section, and both
are the kind of row that only appears when something is actually run** — D-21
(found by S9's script on its first run) and D-22 (what `verify-seams` provably
cannot find, which no run can tell you). The ten that were open each now opens
with a tag saying **who decides**, because a flat list of "OPEN" rows mixed six
different kinds of object. In summary: **three are architect decisions**
(D-09's open half, D-20 — closed on 2026-10-02 by *posing* its question rather
than coding it — and now **D-21**), **two are environment-blocked** (D-06 and
D-15 — neither closable by writing code), **two are Phase 1 by design** (D-11,
D-12), **one is a recorded non-decision** (D-10), **one is process** (D-13),
and **two are real but bounded and not Phase 1's problem** (D-17 needs a CI
owner; D-19's teardown defect is fixed and the unreproduced hang is not a
blocker). **D-22 is closed by being written down**, which is the whole point of
it: it is a scoping note about what a green `verify-seams` means, and no commit
closes it.

**D-06 and D-15 are the only two that stop a claim from being verified rather
than a capability from existing.** Neither will be closed by Phase 1 writing
code; both close on a machine with an `ironclaw` binary.

### 21.3 Phase 1 landmines

**Six things an adapter author will hit. The first is now fixed; the second is
a live defect; the rest are decisions the code is making by omission.** Landmine 1 was
originally written as "not in the register", and the store-parity script that
found the second of these has since landed and turned it into a fixed bug plus
a new one — so this list is now what the tree actually says.

**1. `MockStore` accepted a duplicate fingerprint; that is fixed, and the fix is
pinned.** `MockStore::put_source_candidate` did
`source_by_fp.insert(fp, id)` unconditionally, so a second candidate carrying a
duplicate fingerprint silently overwrote the index while real SQLite rejected it
(`migrations/0001_initial.sql:65` is `fingerprint TEXT NOT NULL UNIQUE`).
Defect instances 9 and 11, made invisible because every test of capture ran on
the mock. `ironmaint-testkit/src/store_conformance.rs` now runs the same
assertions against both backends, and the guard is in the store rather than
only at the one call site that had it. **D-09-era, closed by S9.**

**2. `MockStore` has no referential integrity, and `job.capture` reaches the
gap — D-21, and this is the one to read before writing an adapter.**
`source_candidates.job_id` is a `FOREIGN KEY` to `jobs(id)` and `SqliteStore`
sets `PRAGMA foreign_keys = ON` on every connection
(`crates/ironmaint-store-sqlite/src/lib.rs:90,121`), so SQLite genuinely
refuses a candidate whose job was never created. **Nothing between the agent and
the insert creates it**: the `job.capture` dispatcher takes `input.job_id`
verbatim (`crates/ironmaint-mcp/src/dispatch.rs:263`), `ensure_workspace` does
not create a job, and `handle_capture_candidate`
(`crates/ironmaint-runtime/src/service.rs:1461`) checks only that
`candidate.job_id() == job_id`. An agent that calls `job.capture` before
`job.create` gets a raw `FOREIGN KEY constraint failed` out of SQLite, while
every test of that path runs on the mock and sees it succeed.

*This one is recorded, not fixed, and the reason matters.* Giving the mock
referential integrity has a measured blast radius of **7 tests across 5
binaries**, and most of them legitimately build a candidate for a job the
workspace layer has no business knowing about. So the disagreement underneath is
real and it is a design question: **the workspace layer and the runtime disagree
about who creates the job first.** That is a state-ownership decision, which
§106 puts on the do-not-redesign list, so it goes to the architect alongside
[§17 question 8](#17-questions-requiring-review-before-real-debian-integration)
rather than getting patched. The divergence is pinned executably by
`store_parity_sqlite::a_candidate_for_an_unknown_job_is_refused`, in the
backend-specific driver rather than the shared script, because the mock cannot
pass it.

**3. A `SourceCandidate` is a property of a source tree, not of a job.**
`compute_fingerprint` hashes family, release, source name, version, repository
URL, commit and tree — **`JobId` is not among them** — and the `UNIQUE`
constraint is global rather than per job. So two jobs that capture the same
upstream tree are contending for one candidate, and the second write is refused.
That falls out of two independent design choices, neither of which mentions the
other, and **nothing in the workspace asserted it until
`a_fingerprint_is_shared_across_jobs` existed.** Whether it is right is not the
suite's call; an adapter that lets two jobs point at one tree needs to know, so
the behaviour is pinned to make a future change a decision rather than an
accident.

**4. Nothing escalates a job to `HumanReviewRequired` in production**, so the
registered `job.resume` tool cannot succeed on any job a real agent can reach.
`EnterHumanReview` is implemented, dispatched, and works — its only
construction site is `crates/ironmaint-mcp/tests/dispatcher.rs:236`. Same
shape as the `InfrastructureBlocked` entry in D-09. **Both are posed as
[§17 question 8](#17-questions-requiring-review-before-real-debian-integration)
above, with three candidate answers.** Nothing is broken; the decision is
simply unmade, and state ownership is on §106's do-not-redesign list.

**5. The stubs' plans are real and unwired.** `adapters/debian-stub` and
`adapters/fedora-stub` return distribution-distinct plans — eight capability
keys across the two, none of which has a registered tool — and both pass the
shared conformance suite. **Their only caller is that conformance suite.**
Wiring `build_plan()`/`qa_plan()` into a production flow is Phase 1's first
task, and §17 question 7 already settled that the first real adapter must
activate its candidate or it is not exercising what it claims to.

**The allowlist that now names all eight is the useful artefact here.**
`crates/ironmaint-adapter-api/tests/capability_inventory.rs` pairs each
unregistered key with the phase that supplies it, and
`debian.qa.piuparts` is the one that does **not** need
`doc/CONTAINER-IMAGES.md` §4.3 — piuparts tests the source package, which the
capture boundary already produces — so it is the first that can be unblocked
without the image work.

**6. Six of the fourteen wrong-guard defects are out of reach of any verifier
shipped so far — now D-22.** `reconcile`'s `never_loop`, `SetActiveCandidate`'s
state guard, `attach_obligation_state`'s read-before-check, `next_actions`
omitting an obligation. The link exists, has a caller, and does the wrong thing.
A "is this called" check sees a called function and has nothing to say. This is
recorded so those six are not later re-attributed to `verify-seams`, and so **a
green `verify-seams` is never read as "no missing seams."**


### 21.4 What Phase 0A left behind

There is no `PHASE-0A-COMPLETION.md`. Phase 0A's six sub-phases landed as
`47ceb6e` (0A.1), `feature/phase-0a-domain-model` (0A.2–0A.3), PR #7 (0A.4),
`a514686` (0A.5), and PR #9 (0A.6). Their spec sections are
`doc/phases/PHASE-0A.md` §§76–81. The one post-condition that slipped — 0A.6
promoting the panic-policy lints — landed later under D-04 in 0B.10, not in
0A.6.

### 21.5 What is still claimed rather than shown

Two, both unchanged since 0B.10 and both recorded above:

- **§102 row 30** — an actual IronClaw agent completing the synthetic repair
  workflow. Environment-blocked (D-15). The workflow is proven agent-drivable;
  the specific agent the spec names has not been run against it.
- **The §36 operator escape hatch** — driven by hand, not by
  `scripts/ironclaw-e2e.sh`, whose IronClaw half skips with a notice when no
  binary is present. A green run of *that* script proves the server, not the
  agent.
