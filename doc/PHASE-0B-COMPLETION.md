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

Counts grew from the 0B.6 report's 672/675 to 756/759: +84 across 0B.9.
The largest blocks are `ironmaint-mcp/tests/transport.rs` (15, over a real
socket), `bins/ironmaintd/tests/startup_shutdown.rs` (13, driving the real
binary as a child process), and `bins/ironmaintd/tests/config.rs` (17).
`ironmaint-runtime/tests/approval_refusal.rs` adds 9.

Note for the reviewer: `cargo xtask` is not installed in this environment;
each verifier is invoked as `cargo run -p xtask -- <subcommand>`.

## 13. Actual IronClaw E2E result

**The script now runs; the IronClaw half still cannot execute here.**

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

### 16.7 The 0B.8 hardening suite is not exhaustive (LOW)

`unmet_gate_requirements_block_transition` asserts the transition is
rejected but accepts either `TransitionBlocked` or `InvalidInput` as the
error kind, because the runtime's pre-flight check fires before the engine's
blocker list in that configuration. The invariant under test ("the projection
is unchanged") is solid; the specific error kind is not pinned. Worth
tightening if the error taxonomy is considered stable API.

### 16.8 A job cannot advance past `EventDetected` (HIGH) — found in 0B.9

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
code.~~ STILL OPEN, and 0B.9 did not touch it.** The `for _ in 0..15u32`
at `service.rs:615` remains, still `#[allow(clippy::never_loop)]`, still
with `Advanced` returning on the first transition. 0B.9 deliberately left
it: removing it changes `reconcile` from "loop until it stops" to "one
step", and that is a semantic change, not a cleanup. The single-transition
semantic needs confirming first.

It is worth noting in the meantime that **the loop is not the reason
§16.8 happens.** `reconcile` returns `NoOp` on its first iteration for a
job with no active candidate, long before the loop bound could matter.

**7. NEW — raised by 0B.9, and the most important question in this
section. How should a job advance past `EventDetected` while there is no
adapter?** See §16.8, which lays out three options and notes which of them
are compatible with §94's nine-tool enumeration. This one should be
answered before Phase 1 opens, because Phase 1's stated premise is real
adapters meeting real failures, and §16.8 is a case where the two do not
meet.

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
opaque id) and reaches the system exclusively through nine JSON tools. No
core type names IronClaw; `verify-architecture` enforces that the dependency
edge runs runtime → mcp, never mcp → runtime.
