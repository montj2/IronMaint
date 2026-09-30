# IronMaint technical-debt register

Standing register of known debt, stubs, and unenforced invariants. Created
2026-09-29 against `develop` @ `0acaedc`, immediately after Phase 0B.9 merged
(PR #25).

This document is **not** a phase-completion report. `doc/PHASE-0B-COMPLETION.md`
records what Phase 0B built and what it deliberately left open. This file records
everything an auditor would otherwise rediscover from scratch, including findings
that are *not* in the phase report at all.

**Rule of thumb:** if an item here is marked CLOSED, it was closed by a commit and a
passing gate — not by a decision to stop looking.

## How to read this

Severity is about consequence, not effort.

| Severity | Meaning |
|---|---|
| **HIGH** | An invariant the project claims to enforce is not actually enforced, or a documented interface is wrong. Ships a false claim. |
| **MEDIUM** | Real debt with a known fix that has not been paid. Does not currently produce wrong behaviour. |
| **LOW** | Deferred by an explicit decision, or a process hazard. Tracked so it is not lost, not because it is urgent. |

Every entry below was established by reading the code and by running a command, not
by inference. Where a fix was tested experimentally rather than proposed, the entry
says so and gives the result. Claims inherited from `doc/PHASE-0B-COMPLETION.md`
were **re-verified independently** before being copied here; one did not survive
(D-02).

## What is already mechanically enforced — do not re-audit this

Recorded so a future session does not repeat the sweep. Each was verified on
2026-09-29.

- **No `todo!()` / `unimplemented!()` macro anywhere in the tree.** One textual
  match exists, at `crates/ironmaint-executor/src/limits.rs:190`, and it is a
  *comment* asserting the invariant.
- **No `unwrap` / `expect` / `panic` in production code — in 17 of 19 crates.**
  `[workspace.lints.clippy]` sets `unwrap_used`, `expect_used`, `panic`, `todo`,
  `unimplemented`, and `map_err_ignore` all to `"deny"`. Every crate re-permits
  them under `#[cfg(test)]` via a crate-level `#![cfg_attr(test, allow(...))]`.
  Because §97 runs `clippy --all-targets --all-features -D warnings`, a green gate
  proves the production paths are clean. **The 2 exceptions are D-01.**
- **No `unsafe` anywhere.** `unsafe_code = "forbid"`, inherited everywhere the lints
  are (again, D-01 for the gap).
- **The roughly 380 raw `unwrap`/`expect` occurrences in `src/` directories are
  all inside `#[cfg(test)]` blocks.** The per-directory counts (policy 109, core 86,
  evidence 69, testkit 39, …) are not a debt signal; they are test assertions.

## Register

| ID | Severity | Item | Status |
|---|---|---|---|
| D-01 | HIGH | `ironmaint-store` and `ironmaint-store-sqlite` inherit no lint set — the no-panic and no-unsafe policies are unenforced in the persistence layer | OPEN |
| D-02 | HIGH | The spec-designated `SKILL.md` is stale and documents a wire format the server rejects | OPEN |
| D-03 | HIGH | A job cannot advance past `EventDetected`; 5 of 9 runtime commands and 1 of 5 queries have no MCP entry point, and `next_actions` advertises 3 actions nothing can perform | DEFERRED to Phase 1 |
| D-04 | MEDIUM | `dead_code = "warn"` carries a promise ("promoted once 0A.6 lands") that was never kept | OPEN |
| D-05 | MEDIUM | 4 of 6 `#[allow(dead_code)]` sites are vestigial, two with false justifications | OPEN |
| D-06 | MEDIUM | `mcp-registration.yaml` field names never round-tripped through a real `ironclaw extension install` | OPEN (no binary available) |
| D-07 | MEDIUM | Dead `for _ in 0..15u32` loop in `reconcile`, suppressed with `clippy::never_loop` | OPEN, needs a semantic decision |
| D-08 | MEDIUM | `ProcessExecutor` holds `artifact_store` and `guard_factory` and never reads either; oversized tool output is silently discarded | OPEN |
| D-09 | LOW | `CaptureResume` / `infrastructure_blocked` signal absent | DEFERRED to 0B.10 |
| D-10 | LOW | No background reconcile loop in the daemon | DEFERRED to 0B.10 |
| D-11 | LOW | No real Debian/Fedora adapters | DEFERRED to Phase 1 (§106) |
| D-12 | LOW | 0B.8 hardening suite is not exhaustive | DEFERRED |
| D-13 | LOW | `git add -A` in this repo can sweep in unrelated local artifacts | PROCESS |

---

## D-01 — the persistence layer has no lint enforcement (HIGH)

**Severity rationale:** the project's central safety claim is that no production
path can panic or use `unsafe`. That claim is enforced in 17 crates and silently
absent in the two that own the database.

### Evidence

Of the 19 workspace members, 17 declare `[lints] workspace = true`. Two do not:

```
crates/ironmaint-store/Cargo.toml
crates/ironmaint-store-sqlite/Cargo.toml
```

Because the table is not inherited, `[workspace.lints.clippy]`'s
`unwrap_used = "deny"`, `expect_used = "deny"`, `panic = "deny"`, `todo`,
`unimplemented`, `map_err_ignore = "deny"` and `[workspace.lints.rust]`'s
`unsafe_code = "forbid"` simply do not apply there. The §97 gate cannot catch a
regression in either crate.

The two crates then contain exactly the violations the policy exists to prevent —
which is the expected outcome, not a coincidence: the code was written with no
mechanical constraint and no reviewer alarm.

### The four sites

Verified by appending `[lints]\nworkspace = true` to both manifests and running
`cargo clippy -p ironmaint-store -p ironmaint-store-sqlite --all-targets
--all-features`. Exactly four errors, in two files:

| Site | Violation | Note |
|---|---|---|
| `crates/ironmaint-store/src/mock.rs:81` | `expect_used` | `self.inner.read().expect("MockStore lock poisoned")` — a poisoned `RwLock` **panics the process** |
| `crates/ironmaint-store/src/mock.rs:85` | `expect_used` | same, on the write path |
| `crates/ironmaint-store-sqlite/src/lock.rs:42` | `map_err_ignore` | `.map_err(\|_\| "another ironmaintd is using this state directory")` — the real `io::Error` is discarded |
| `crates/ironmaint-store-sqlite/src/lock.rs:53` | `expect_used` | inside `empty_for_tests()`, a `#[doc(hidden)] pub fn` that is compiled into the **production** library, not gated by `cfg(test)` |

`lock.rs:42` is the same defect class already fixed once during 0B.9 in
`bins/ironmaintd/src/config.rs` (where `ConfigError::InvalidValue.why` was changed
from `&'static str` to `String` precisely so the underlying error could be kept).
That fix was never applied here, because nothing in this crate would have flagged it.

### Fix

Four sites, all mechanical. Worth doing as a single small commit:

1. `mock.rs:81,85` — return a `StoreError` instead of panicking. A lock-poisoned
   `MockStore` is a test-harness failure, and a `StoreError` surfaces it as a test
   failure rather than an aborted process.
2. `lock.rs:42` — carry the `io::Error` through, as `ConfigError::InvalidValue`
   now does.
3. `lock.rs:53` — either gate `empty_for_tests()` behind `#[cfg(test)]` (it should
   be test-only) or give it a `Result`.
4. Add `[lints]\nworkspace = true` to both manifests.

**Verification after the fix:** the §97 gate, unchanged. That is the point — the
gate starts covering two more crates.

### Why this was missed

The lint config was added crate-by-crate during 0A, and these two crates were
created later (Phase 0B) without the `[lints]` stanza. Nothing checks for the
omission, because the absence of a stanza is not an error. A one-line
`verify-lint-inheritance` subcommand in `xtask` would close the class permanently —
see D-04's note on guardrail philosophy.

---

## D-02 — the spec-designated `SKILL.md` is stale (HIGH)

**Severity rationale:** an agent that follows the spec's own path installs
instructions to call `check.run` with a field the server rejects. This is a
documented interface that is wrong, not merely incomplete.

### Evidence

There are **two** `SKILL.md` files, and they are not in sync:

| Path | Lines | `check.run` signature documented |
|---|---|---|
| `integrations/ironclaw/skills/ironmaint-maintainer/SKILL.md` | 68 | `{ job_id, tool_key, retry_class? }` → `{ tool_key, exit_code, stdout, stderr }` |
| `skills/ironmaint-maintainer/SKILL.md` | 227 | `check.run(check_id)` → evidence + gate outcome |

Phase 0B.9 corrected the 227-line file. The 68-line file still describes the
**pre-0B.9 wire format**, which no longer exists:
`RunCheckInput` takes `check_id`, not `tool_key`, and `RunCheckOutput` no longer
returns raw `exit_code`/`stdout`/`stderr`.

### Which one is authoritative

This is the part that makes it HIGH rather than LOW, and it is the reverse of the
intuition:

- `doc/phases/PHASE-0B.md:2077` — **the spec** — names
  `integrations/ironclaw/skills/ironmaint-maintainer/SKILL.md`. That is the
  **stale** one.
- `doc/operator/ironclaw-setup.md:144,153` — the operator runbook, and the file an
  operator actually copies — points at `skills/ironmaint-maintainer/`. That is the
  **corrected** one.
- `doc/operator/tool-permissions.md:9` also points at the corrected one.

So the spec and the runbook disagree about where the skill lives, and the spec's
answer is the wrong file. A reader following `PHASE-0B.md` gets the stale skill; a
reader following the operator doc gets the right one.

### Fix

Decide which path is canonical, then:

- delete the other, or
- make one a symlink / generated copy, so they cannot drift again.

Two hand-maintained copies of an agent-facing contract is the underlying defect;
the specific divergence is just this instance. `doc/operator/ironclaw-setup.md:144`
and `PHASE-0B.md:2077` must then agree.

**Not yet done deliberately:** choosing the canonical path is an architectural
decision (does IronMaint own `integrations/`, or does the vendor?). Deferred rather
than guessed.

---

## D-03 — a job cannot advance past `EventDetected` (HIGH, deferred)

Carried from `doc/PHASE-0B-COMPLETION.md` §16.8, where it was correctly classified
HIGH and deferred to Phase 1 by decision. Re-verified here, and **widened**: the
§16.8 census covered two commands; the full census is larger.

### Full census at `0acaedc`

`RuntimeCommand` (9 variants):

| Variant | Reachable from MCP? |
|---|---|
| `CreateJob` | yes — `job.create` |
| `CaptureCandidate` | yes — `candidate.capture` |
| `RunCheck` | yes — `check.run` |
| `MaterializeChecks` | **no** — tests only |
| `SetActiveCandidate` | **no** — tests only |
| `RecordCheckEvidence` | **no** — tests only |
| `MarkObligationSatisfied` | **no** — *not referenced anywhere, not even a test* |
| `RequestApproval` | no, by design — always returns typed `Unsupported` |
| `Reconcile` | **no** — unreferenced; `job.reconcile` calls the public `reconcile()` method directly instead |

`RuntimeQuery` (5 variants): `GetJob`, `ListNextActions`, `GetCheckOutcome`,
`GetOperation` reachable; **`GetProjection`** tests only.

### The widening

- **`MarkObligationSatisfied` is the strongest instance.** `service.rs:1081-1173` is
  ~90 lines of fully-implemented logic, has **no caller and no test at all**. It is
  reachable only by the compiler's exhaustiveness check. This is the closest thing
  in the tree to a shipped stub, and it is the item most likely to rot silently.
- **`RecordCheckEvidence`** (`service.rs:1010-1074`, ~65 lines) is test-only. Note
  its `tool_key` field is bound to `_tool_key` and ignored — a vestigial field
  inside otherwise-live code.
- **`RuntimeCommand::Reconcile`** is genuinely dead: a four-line pass-through to a
  public method that `job.reconcile` already calls. No test, no caller.

### The sharper half: `next_actions` advertises actions nothing can perform

The orphaned commands are one face of this. The more consequential face is that
`job.next_actions` **tells the agent it may take an action for which there is no
tool, and in two cases no command either.**

`AllowedAction` (`crates/ironmaint-runtime/src/next_actions.rs:21-29`) has 7
variants. Mapping each to the MCP tool that would perform it:

| Advertised action | Tool that performs it | Reachable? |
|---|---|---|
| `CaptureCandidate` | `candidate.capture` | yes |
| `RunCheck { check_id }` | `check.run` | yes |
| `ApplyPatch` | `workspace.apply_patch` | yes |
| `MarkObligationSatisfied` | — | **no tool** (command exists, orphaned) |
| `RequestApproval` | — | no tool *by design*; returns typed `Unsupported` |
| `AuthorizeOperation` | — | **no tool, and no `RuntimeCommand` exists** |
| `Publish` | — | **no tool, and no `RuntimeCommand` exists** |

All three orphans are emitted from the per-state fallback table at
`service.rs:1496-1515`, which the code's own comment (`service.rs:1470-1475`)
describes as the *fallback* used "when no evidence-derived action can be
produced". The evidence-derived path emits only `RunCheck`
(`service.rs:246-248`).

**Why this is worse than an orphaned command.** An unreachable command is dead
weight. An advertised-but-unperformable action is the runtime making a claim about
what the agent may do, and then not honouring it. The project's north-star is that
IronClaw proposes and IronMaint disposes — an `allowed` list that cannot be acted
on undercuts the first half of that contract, because the agent is told "you may
proceed" and then finds no verb.

Two of the three are scheduled (privileged-operation producers are 0B.10 scope;
`RequestApproval`'s refusal is intentional and documented). `Publish` and
`MarkObligationSatisfied` have no owner recorded anywhere.

**Fix is not "add three tools."** It is a decision on which of these the tool
surface should advertise before Phase 1. Silently advertising a capability is the
one option that is wrong.

### Store methods with no production caller

Nine, in `crates/ironmaint-store/src/`: `put_artifact`, `get_artifact`,
`put_release_candidate`, `get_release_candidate`, `get_evidence`, `put_obligation`,
`put_operation`, `update_operation`, `get_event`.

Three more are **transitively** dead — reachable only from orphaned commands:
`get_obligation`, `update_obligation` (via `MarkObligationSatisfied`), and
`put_check` / `put_gate_definition` (via `MaterializeChecks`).

### Mechanism

`handle_capture_candidate` persists the `SourceCandidate` and appends an audit
event, but never sets `projection.active_candidate`. `reconcile` refuses to
evaluate any rule without one (`service.rs:665-668`) and returns `NoOp`. Even given
an active candidate, `EventDetected → Intake` requires a `SourcePreparation` gate
that only an adapter's `PlannedCheck` can materialise. So §95's exit checkpoint
("drive to `ReadyForApproval`") is unreachable through the tool surface, while
§102 reports the surrounding claims as met.

The test file `crates/ironmaint-mcp/tests/check_and_operation.rs:131-134` states
the design contract explicitly: *"0B.9 does not add a `check.materialize` tool —
adapters are supposed to do this."* That contract has no implementation;
`bins/ironmaintd/src/main.rs:290` logs `adapter registry: deferred to Phase 1`.

### Risk carried into Phase 1

Phase 1 opens with the state machine unexercisable through the tool surface. Its
first adapter **must** activate the candidate and materialise the checks, or the
phase is not testing the premise it exists to test. This should be the first
acceptance criterion of Phase 1, not an incidental outcome of it.

### Confirmed NOT the cause

The dead loop at `service.rs:614` (D-07) is **not** implicated. `reconcile`
returns `NoOp` on its first iteration, long before the loop bound could matter.

---

## D-04 — `dead_code` was never promoted to `deny` (MEDIUM)

### Evidence

`Cargo.toml:46`:

```toml
# Reachable code and unused imports are noise during skeleton bootstrap
# but will be promoted to deny once 0A.6 lands.
dead_code = "warn"
```

Phase 0A.6 landed long ago. The promotion never happened, so the stated rationale
no longer describes the codebase.

### The promotion is two fixes away — measured, not assumed

Tested on 2026-09-29: set `dead_code = "deny"` and ran the full §97 clippy
invocation (`--workspace --all-targets --all-features -D warnings`). The workspace
compiles clean **except for exactly two sites**, both fields on the same struct:

```
crates/ironmaint-executor/src/process.rs:72:5: field `artifact_store` is never read
crates/ironmaint-executor/src/process.rs:76:5: field `guard_factory` is never read
```

(Both currently carry `#[allow(dead_code)]`; the error appears once those are
removed. With the allows left in place the promotion compiles clean, which is how
this can sit unnoticed indefinitely — the suppression hides the only thing the
promotion would have caught.)

See D-08 for what those two fields are and why they are still zero.

---

## D-05 — vestigial `#[allow(dead_code)]` sites (MEDIUM)

Six `#[allow(dead_code)]` attributes exist in production `src/`. Four are
vestigial, and **two carry a justification that is factually false** — which is
the no-stubs rule being violated in miniature.

| Site | Covers | Verdict |
|---|---|---|
| `crates/ironmaint-store-sqlite/src/ops/workspaces.rs:89` | `fn _kind() -> StoreErrorKind` | **Vestigial.** Its comment says it exists to anchor the `StoreErrorKind` import. That import is genuinely unused in the file — so the right fix is to delete both the function *and* the import. |
| `crates/ironmaint-executor/src/operation.rs:139` | `fn _ensure_time_import() -> OffsetDateTime` | **Vestigial, and its justification is false.** The comment says it anchors the `time::OffsetDateTime` import — but `OffsetDateTime` is already used in real code at `operation.rs:69` and as a parameter type at `operation.rs:110`. The anchor protects nothing. |
| `xtask/src/mcp_schemas.rs:99` | `fn _ensure_used()` | **Vestigial.** Anchors `McpToolName`, which is genuinely unused in the file. Note its body is `let _ = McpToolName::from;` — this compiles only because name resolution falls through to the blanket `impl<T> From<T> for T`, so it is the identity conversion, not a real use. |
| `crates/ironmaint-store-sqlite/src/lock.rs:23` | `DaemonLock.file` | **The allow is wrong.** The field is genuinely load-bearing — closing the `File` releases the OS flock, which is the entire design — and is read by the `#[derive(Debug)]` and by `File::drop`. It does not need the suppression. |
| `crates/ironmaint-executor/src/process.rs:72` | `ProcessExecutor.artifact_store` | Legitimate reservation → **D-08**. |
| `crates/ironmaint-executor/src/process.rs:76` | `ProcessExecutor.guard_factory` | Legitimate reservation → **D-08**. |
| `xtask/src/schemas.rs:218` | `pub fn schemas_path()` | **Grey zone.** Doc comment names its consumers as "CI scripts, docs, architecture.rs"; none exist. Either delete it or state the concrete future caller. `#[allow(dead_code)]` on a `pub fn` is a smell, because cargo cannot see out-of-workspace consumers. |

### Why this matters beyond tidiness

`fn _kind()`, `fn _ensure_time_import()` and `fn _ensure_used()` are dead functions
kept alive by a lint suppression, one of which is documented with a reason that
does not hold. That is the exact shape the project rule forbids — *"Stubs are not
a closing mechanism."* They are small, but the class is real and the comment
misinformation is worse than the code.

**Fix:** delete the three functions and the two now-unused imports; drop the allow
in `lock.rs:23`; decide `schemas_path`. Cheap, and it makes D-04's promotion
honest.

---

## D-06 — the IronClaw manifest was never round-tripped (MEDIUM)

`doc/operator/mcp-registration.yaml` declares `api_version:
reborn.extension_manifest.v3` with a nested `extensions[].capabilities.tools` list.
**No `ironclaw` binary exists in this environment**, so the manifest's field names
have never been validated by an actual `ironclaw extension install`. The `auth`
block was corrected during 0B.9 against the spec, but the surrounding structure is
unverified.

Consequence: `scripts/ironclaw-e2e.sh` §6 correctly *skips* its extension checks
when no binary is present, so the smoke test reports OK without ever exercising
this. The script is honest about it; the risk is that a reader takes a green run
as coverage.

**Closes when:** an `ironclaw` binary is available and the install round-trips.

---

## D-07 — dead loop in `reconcile`, suppressed (MEDIUM)

`crates/ironmaint-runtime/src/service.rs:614`:

```rust
#[allow(clippy::never_loop)]
for _ in 0..15u32 { ... }
```

The loop body always takes the same branch, so the iteration bound is meaningless
and the suppression is what keeps the compiler quiet about it.

**Blocked on a semantic decision**, not on effort: `reconcile` is specified to
apply *one* transition per call. If that is the rule, the loop should be deleted
and the single-transition contract made explicit. If multi-transition-per-call was
intended, the loop needs a real exit condition. The current code is neither, and
picking either reading changes observable behaviour.

**Confirmed not to be the cause of D-03.**

---

## D-08 — `ProcessExecutor` holds two fields it never reads (MEDIUM)

`crates/ironmaint-executor/src/process.rs:72,76`. Both are constructed in
`ProcessExecutor::new` and never read. `Debug` is implemented with
`.finish_non_exhaustive()` and lists only `registry` and `env`, confirming neither
field participates in anything.

This is a **documented reservation**, not a stub: both comments name the intended
consumer — `execute()` spilling oversized stdout/stderr into the
`ArtifactStore`, and invoking `guard_factory(job_id)` to obtain a
`JobArtifactGuard`. 0B.9's plan listed "artifact spilling in `ProcessExecutor`" as
an explicit out-of-scope deferral.

The reason it is MEDIUM rather than LOW: `bins/ironmaintd/src/main.rs:194-200`
opens and verifies the `ArtifactStore` during startup (§67's "verify artifact
store" step) and logs success, but **nothing is ever written to it**. Tool output
is bounded in memory into the `ExecutionRecord`. An oversized build log is
therefore silently truncated-or-dropped with no artifact, no error, and no
indication that anything was lost — while startup logs claim the artifact store is
verified.

Also note: `with_guard_factory` (the setter for the second field) has **no
production caller**.

**Closes when:** `execute()` uses both, or they are removed.

---

## D-09 — no `CaptureResume` / `infrastructure_blocked` (LOW, deferred)

A tool that fails for infrastructure reasons (sandbox, worker, DB) has no way to
express "resume this later" as opposed to "this package did not build". Scheduled
for 0B.10, user-confirmed scope. See `doc/PHASE-0B-COMPLETION.md` §16.4.

---

## D-10 — the daemon runs no reconcile loop (LOW, deferred)

`reconcile` is reachable only when an agent calls `job.reconcile`. Nothing drives
it proactively. Scheduled for 0B.10, user-confirmed scope. See §16.6.

Note the interaction with D-03: until a job can leave `EventDetected`, a
background loop would have nothing to do anyway.

---

## D-11 — no real Debian/Fedora adapters (LOW, deferred)

`adapters/debian-stub` and `adapters/fedora-stub` return **real, distribution-distinct
plan data** — `debian.build.sbuild` / `debian.qa.{lintian,piuparts}` /
`debian.test.autopkgtest`, and `fedora.build.mock` / `fedora.qa.{rpmlint,spectool}` /
`fedora.test.functional` — with correct mandatory flags, and both pass the shared
conformance suite. They are not returning constants or empties.

What is missing is that `build_plan()` / `qa_plan()` are called **only** by the
conformance harness. Phase 1, §106. See §16.5.

---

## D-12 — the 0B.8 hardening suite is not exhaustive (LOW, deferred)

The hardening suite was written against discovered cases, not derived from the
spec's invariant list. See §16.7.

---

## D-13 — `git add -A` hazard (LOW, process)

**This actually happened.** During the 0B.9 merge, `git add -A` staged 4 files
(206 KB) from `.playwright-mcp/` — session captures written by an unrelated
Playwright MCP tool that happens to run on this machine. They were committed, and
had to be removed in a follow-up commit (`0acaedc`) after being untracked and
added to `.gitignore`.

**Why it recurs:** this repo sits next to unrelated tool state, and `git add -A`
has no way to distinguish. The `.gitignore` entry fixes this specific directory,
not the class.

**Mitigation:** stage by path (`git add crates/ bins/ xtask/ doc/ …`) or use
`git add -p`. Do not use `-A` in this repository.

---

## Suggested ordering

Not a plan — the user decides. In rough order of value-per-effort:

1. **D-01** — four sites, two files, closes a HIGH with a false safety claim. The
   test-artifact/lint-inheritance gap it exposes argues for a new
   `xtask verify-lint-inheritance` guardrail so the class cannot recur.
2. **D-05** — deletes three dead functions and two false comments; makes **D-04**'s
   promotion honest.
3. **D-02** — needs one architectural decision, then a deletion.
4. **D-07** — needs one semantic decision, then a deletion.
5. **D-04** + **D-08** — same two `ProcessExecutor` fields; land together or not at all.
6. **D-03** — Phase 1, and should be its first acceptance criterion.
7. **D-06** — blocked on an external binary; nothing to do until one exists.
8. **D-09**, **D-10** — 0B.10.
9. **D-11**, **D-12**, **D-13** — as scheduled.
