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

**Re-audited 2026-10-01** against 0B.10, which closed D-02, D-03, D-07 and D-14,
partly closed D-09, and added D-15. Five items are closed, one is a recorded
decision not to act (D-10), and one cannot be checked in this environment
(D-15). The severity table below is unchanged; the register's shape is not — the
highest-severity item is now D-01, which 0B.10 deliberately did not touch.

**Re-audited again later on 2026-10-01**, after 0B.10's tool surface was driven
over real HTTP against a live daemon. That run found two defects in the §36
operator escape hatch, adds **D-16**, and amends **D-14**: its closure is true
about a job's birth and false about every field written after it. Worth noting
for how this register is read — **D-14 was marked CLOSED, and a day later a
defect sat directly behind it.** A closure records a fix, not an absence, and
the two are easy to mistake for each other when the item was found by reading
rather than by running.

**Wave 1 of `doc/EXECUTION-PLAN.md` landed 2026-10-01** and closed **D-01**, the
register's last HIGH. The lint set now covers all 21 workspace members, so the
project's central claim — no `unwrap`, no `expect`, no `panic`, no `unsafe` in a
production path — is enforced everywhere it is claimed rather than in 19 of 21
crates. Six items are now closed.

D-01 is also the first closure here that was **teeth-checked**: a real `unwrap`
was added to the SQLite crate's production path and clippy rejected it, so the
gate is known to reject rather than assumed to. Every closure after this one
should be treated the same way, given the D-14 note above.

**Wave 2 landed 2026-10-01** and closed **D-16** with `JobEvent::CandidateActivated`.
It is the first closure whose *receipt is a failing test*: S7 was committed red
first, on purpose, because a test that has never been seen to fail is a test whose
failure mode is unknown (D-14). The red run also found a third unrecoverable
field — `updated_at` — that neither D-16 nor the earlier amendment on `version`
had named, and it also corrected the entry's cost estimate: the format change
needs **no migration**, because `events.event_type` is unconstrained `TEXT`. The
refusal in `rebuild-projections` is kept, now as a legacy-data guard for rows
written by the older binary. Seven items are now closed.

**Wave 3 landed 2026-10-01** and closed **D-04** and **D-05** together, in the
order the register asked for: D-08 removed the executor's last two exemptions
first, so promoting `dead_code` to `deny` was a claim about a crate that no longer
needed the exemption, and could be checked. Ten `#[allow(dead_code)]` sites were
deleted, three were rewritten as real checks, and one was kept — the entry, and it
is the more useful half of this note.

**D-05's own analysis of `DaemonLock.file` was wrong**, and following it would
have been a silent regression: it said *"the allow is wrong … it does not need the
suppression"*, on a field whose only job is its `Drop`, because closing the file
releases the OS flock. Building with the deny on produces the opposite verdict, and
rustc volunteers the reason — *"has a derived impl for the trait `Debug`, but this
is intentionally ignored during dead code analysis"*. `File::drop` is not a read,
and the derived `Debug` does not rescue the field. Deleting it would have stopped
the daemon from locking and the gate would have been green throughout.

That is the **second entry in this register whose stated reasoning had to be
corrected by running the code** (the first was D-16's migration cost, wrong in the
*cheap* direction), and it is the D-14 lesson in its purest form: this register was
written by reading, and reading produced a confidently wrong verdict about a field
whose behaviour is a side effect. **An entry that has never been executed is a
hypothesis.** The standing rule above — *closed by a commit and a passing gate* —
is the same rule, and D-05 is what it looks like when the entry itself is the
defect. Nine items are now closed.

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
| D-01 | HIGH | `ironmaint-store` and `ironmaint-store-sqlite` inherit no lint set — the no-panic and no-unsafe policies are unenforced in the persistence layer | **CLOSED** 2026-10-01 — both manifests declare `[lints] workspace = true`, so §97 covers **21 of 21** members. The four predicted sites are fixed, one of them diverging from the register's own recommendation (recorded in the entry). Teeth-checked by adding a real `unwrap` to the SQLite crate's production path and watching clippy reject it |
| D-02 | HIGH | The spec-designated `SKILL.md` is stale and documents a wire format the server rejects | **CLOSED** by 0B.10 C3 — `skills/` is canonical, the stale `integrations/` copy is deleted, and `PHASE-0B.md` §61 has been amended to point at the surviving file |
| D-03 | HIGH | A job cannot advance past `EventDetected`; 5 of 9 runtime commands and 1 of 5 queries have no MCP entry point, and `next_actions` overloads `allowed` as both "moves you can make" and "this needs a human" | **CLOSED** by 0B.10 (C1–C5) — the three missing production links have one caller each, the `allowed` wire split landed in C3, and C5's `mcp_acceptance_scenario.rs` walks §101 from `job.create` to `ReadyForApproval` through `dispatch()` with no `RuntimeService` handle. The census is re-derived at the bottom of the entry, because the right column is now "has a *tool*" and several rows moved to "driven internally by `candidate.capture`" |
| D-04 | MEDIUM | `dead_code = "warn"` carries a promise ("promoted once 0A.6 lands") that was never kept | **CLOSED** 2026-10-01 — promoted to `deny` in the workspace lint table, after D-08 removed the executor's last two exemptions so the promotion could be teeth-checked rather than assumed. The deny was verified to reject a deliberately dead item **in a test target**, where the old `warn` was useless |
| D-05 | MEDIUM | 4 of 6 `#[allow(dead_code)]` sites are vestigial, two with false justifications | **CLOSED** 2026-10-01 — ten sites deleted, three rewritten as real checks, and the eleventh kept. The entry's own analysis of `DaemonLock.file` was **wrong** and recommended a fix that would have silently stopped the daemon from locking |
| D-06 | MEDIUM | `mcp-registration.yaml` field names never round-tripped through a real `ironclaw extension install` | OPEN (no binary available) |
| D-07 | MEDIUM | Dead `for _ in 0..15u32` loop in `reconcile`, suppressed with `clippy::never_loop` | **CLOSED** by 0B.10 C4 (`cec87d7`) — the loop now advances through every satisfied rule, matching §41 and `ReconcileOutcome::Advanced`'s own doc comment. The suppression is gone, not re-suppressed |
| D-08 | MEDIUM | `ProcessExecutor` held `artifact_store` and `guard_factory` and read neither; oversized tool output was silently discarded while the daemon logged the store as verified | **CLOSED** — every run's complete stdout and stderr are spilled to the store, bounded by the job's guard, and the record names each digest, how much it could not keep, and any refusal. Closing it exposed two more defects: the "per-job" caps were per-call because neither factory memoised, and the truncating writer rounded its own cap down to a whole buffer |
| D-09 | LOW | `CaptureResume` / `infrastructure_blocked` signal absent | **PARTLY CLOSED** by 0B.10 C2 — the *exit* is complete (`ResumeRecord` is persisted as a `JobEvent::ResumeRecorded`, and `job.resume` consumes it for **both** exceptional states), but nothing *writes* an `InfrastructureBlocked` transition, so the state has a way out and no way in |
| D-10 | LOW | No background reconcile loop in the daemon | **NOT TAKEN**, and the reason is now recorded rather than deferred again — §41 defines `reconcile` as a per-call function, and no spec text requires a loop |
| D-11 | LOW | No real Debian/Fedora adapters | DEFERRED to Phase 1 (§106) |
| D-12 | LOW | 0B.8 hardening suite is not exhaustive | DEFERRED |
| D-13 | LOW | `git add -A` in this repo can sweep in unrelated local artifacts | PROCESS |
| D-14 | MEDIUM | `rebuild_projection` rejects every real job's event log, so the §36 operator escape hatch cannot rebuild anything | **CLOSED** by 0B.10 — `JobEvent::JobCreated` carries the birth projection, so the log is authoritative for a never-transitioned job. **Amended 2026-10-01**: the closure is true about the birth and false about everything after it; the `active_candidate` gap is **D-16** |
| D-15 | LOW | §102 item 30 ("an actual IronClaw agent can complete the synthetic repair workflow") is **unverifiable in this environment** — not unmet, and not a code defect | **ENVIRONMENT-BLOCKED**, with repro steps at the bottom of the register. 33 of 34 DoD items are true; this is the one that cannot be checked here |
| D-16 | MEDIUM | The event log did not record candidate activation — **or the version and timestamp that activation writes to the row** — so `rebuild-projections` could not rebuild any job that had captured a candidate, and writing the replay anyway would have dropped the §30 binding every gate verdict depends on | **CLOSED** — `JobEvent::CandidateActivated` makes activation replayable, and `rebuild-projections` rebuilds a captured job with its active candidate intact. No migration was needed (`event_type` is unconstrained `TEXT`); the JSON schema snapshot was regenerated. The refusal is **kept** as a legacy-data guard for rows written by the older binary, whose logs genuinely cannot justify their activation |
| D-17 | LOW | Container images and the §97 gate not built or run anywhere automated. `ironmaint/base:0.1` and `ironmaint/workspace:0.1` ship as Dockerfiles + a Makefile + per-image smoke tests; nothing builds them, and nothing runs the gate. Spec §10 is explicit that adding CI is a separate decision with a separate owner. The smoke tests have teeth (D-14 lesson applied) but a digest bump or FROM-tag drift would not be caught without someone running them. | **CLOSED 2026-10-03**, by taking the decision rather than by writing code — and the decision went the other way from what was expected. The owner was asked what the gate should be and answered: it runs **locally**, from a Mac, in the workspace image. `make -f containers/Makefile gate` runs the §97 set there; `gate-fast` is the editing loop; `verify` builds and smokes all six images; `gate-teeth` proves the gate can fail. A GitHub Actions workflow was written first and is not in the tree, because it was not what was wanted. `CONTAINER-IMAGES.md` §10 records the resolution in place rather than deleting the non-goal, and §12.1 says plainly what it costs: **nothing runs unless a human runs it.** That is a real loss, accepted by the person entitled to accept it — which is exactly what §10's "different owner" was asking for |
| D-18 | MEDIUM | One symptom — 7 of 10 `process_exit_codes` tests failing with `exit_code: 127` in the container — with **two independent causes**, plus a third found while verifying. Six tests were a harness defect (three sites resolved the `ironmaint-fixture` binary by walking to `target/` and guarding with `path.exists()`); one was a **production** defect (on Linux `posix_spawn` reports a failed `exec` as a child exiting 127, so a missing tool was recorded as a tool that ran) | **CLOSED 2026-10-02** — the original entry's hypothesis was *half* right and the first correction was *half* wrong: both halves shared the exit code 127, which is why the 7/10 never identified a single cause. Four fixes: `67fedbc` one loadability-checking resolver, `3e30002` a fourth call site, `c0e551b` a pre-spawn launchability check, `29c8e23`/`02ee489` the container build dir and the verifier that read it. In-container §97 is green for the first time (877/886). Raised LOW → **MEDIUM** on the production half. See the long-form entry below |
| D-19 | MEDIUM | A §97 container run hung in `startup_shutdown.rs` and the cause is **not known**. One confirmed defect found while investigating: `Drop for Daemon` signalled SIGKILL but never reaped, so 11 of the 15 tests in that file depended on tokio's SIGCHLD orphan handler to collect their own children — and a live `ironmaintd` from a killed run survived on the host for two days | **PARTLY FIXED** — `kill_on_drop(true)`, an awaited `shutdown()` that escalates, and a test that pins the reap. The **hang itself is OPEN**: see the entry for what is ruled out, what is not, and what would settle it |
||||||| parent of a3040fc (Correct S1's allowlist, and record D-20: five are decisions, two are stale docs)
| D-20 | LOW | S1 in `verify-seams` found that **7 of 12 `RuntimeCommand` variants have no production construction site.** `handle_command` has twelve arms, all patterns; the five constructions are all in `crates/ironmaint-mcp/src/dispatch.rs`. Five of the seven are decisions with the reason written down at the call site. The two that are not: **`RequestApproval`'s doc makes a claim that is false**, and **`job.resume` cannot succeed in production** because the only `ResumeRecorded` writer is itself unreachable | **OPEN, two parts, both documentation-shaped** — no capability is missing from the runtime, and neither part is a bug to fix so much as a comment that will mislead. Part 1: `RequestApproval`'s doc says `next_actions` advertises it and that following it reaches the refusal; `ReadyForApproval` emits `allowed: vec![]` / `requires_human: ApproveRelease` and `RequestApproval` is not an `AllowedAction`. Part 2: the two `next_actions` docs describing `ResumeJob` and `ReviewEscalation` describe a state production cannot reach. See the long-form entry below |
| D-23 | **MEDIUM** | **Two of the six images were x86_64 inside an arm64 label.** `containers/workspace/Dockerfile` and `containers/fedora-tools/Dockerfile` pinned their bases by digest, and both pins resolved to a **single-manifest, amd64-only** image rather than a multi-arch index. `--platform linux/arm64` then pulled an amd64 base and BuildKit emitted one `InvalidBaseImagePlatform` warning and continued, so `docker inspect` reported `arm64` while every binary inside answered `x86_64` to `uname -m`. Measured: `base`, `debian-tools`, `agent`, `fake-services` were genuinely aarch64; `workspace` and `fedora-tools` were not | **CLOSED 2026-10-03** — both bases re-pinned to index digests, and every image's smoke test now asserts `uname -m` against the architecture it was requested at, so the next digest bump cannot silently reintroduce it. **Both contaminated images reported PASS**, and that is the finding worth keeping: Docker Desktop transparently emulates x86_64 on this host, so `rustc --version` and `rpm --version` both worked and the smoke tests were satisfied by an image that was wrong. The existing checks asked *does the tool respond*, never *is this binary the right architecture* — `fedora-tools/smoke.sh` asserted `fedora-44-aarch64.cfg` **exists**, which is a file, not an execution. This was D-18's shape one layer up. The teeth-check is the receipt: the *same* image that passed all six of its own other assertions fails the new one with `image is x86_64 (amd64) but was built for arm64`. Fixed before the CI job landed, because a gate that builds a contaminated image is a gate that cannot fail |

---

## D-23 — two of six images are x86_64 inside an arm64 label (MEDIUM) — **CLOSED 2026-10-03**

Found while planning the CI sub-phase, by asking whether the image job could run
on an arm64 runner and then checking rather than assuming. It gets a long-form
entry because the reason it survived is the interesting part, and because a
one-line row would let the next person re-pin a digest and reintroduce it
unnoticed.

### What it is

`containers/workspace/Dockerfile:27` and `containers/fedora-tools/Dockerfile:28`
pin their bases by digest, which is correct and is what §11.1 asks for. The
defect is *which* digest. Pinning `rust:1.94.0-bookworm@sha256:…` where that
digest names a **single manifest** rather than a **multi-arch index** pins the
amd64 build specifically:

```text
$ docker buildx imagetools inspect \
    rust:1.94.0-bookworm@sha256:4673f78d…
MediaType: application/vnd.oci.image.manifest.v1+json   # not …image.index…

$ docker buildx imagetools inspect rust:1.94.0-bookworm
MediaType: application/vnd.oci.image.index.v1+json      # the tag IS multi-arch
  Platform: linux/amd64
  Platform: linux/arm64/v8
```

The tag was multi-arch and the pin silently narrowed it. `--platform linux/arm64`
then resolves the amd64 manifest, BuildKit says so in a warning, and builds
anyway:

```text
InvalidBaseImagePlatform: Base image rust:1.94.0-bookworm@sha256:4673f78d… was
  pulled with platform "linux/amd64", expected "linux/arm64" for current build
```

The result is labelled correctly and is not:

| Image | `docker inspect` | `uname -m` inside |
|---|---|---|
| `ironmaint/base:0.1` | arm64 | `aarch64` |
| `ironmaint/debian-tools:0.1` | arm64 | `aarch64` |
| `ironmaint/agent:0.1` | arm64 | `aarch64` |
| `ironmaint/fake-services:0.1` | arm64 | `aarch64` |
| **`ironmaint/workspace:0.1`** | arm64 | **`x86_64`** |
| **`ironmaint/fedora-tools:0.1`** | arm64 | **`x86_64`** |

### Why every existing check passed

Docker Desktop emulates x86_64 transparently on this arm64 host, so an x86_64
`rustc` and an x86_64 `rpm` both run and both answer. `workspace/smoke.sh` asks
whether `rustc --version` reports 1.94.0; it does. `fedora-tools/smoke.sh:101`
asks whether `fedora-44-aarch64.cfg` is readable; it is — and that assertion is
named for the aarch64 contract while only ever checking that a *file exists*,
which is §9's "a check that cannot fail" failure mode wearing an aarch64 label.

The general shape: **the checks ask whether a tool responds, never whether the
binary is the right architecture.** That is D-18 one layer up — a binary that is
present, runnable, and the wrong one — and it is why this belongs in the CI PR
rather than a backlog. A native arm64 CI runner has no emulator, so the same
image fails outright there instead of quietly; the fix and the CI job are
mutually verifying, which is a better arrangement than either alone.

### The fix, and the trap inside it

Re-pin both bases to the **index** digest, which resolves per-platform and keeps
the reproducibility property §11.1 wants. The trap: a digest that parses and
still resolves to amd64 looks identical in a diff, so the pin edit is not itself
evidence. Verification is two independent checks — `imagetools inspect` reports
`…image.index` with `linux/arm64/v8` present, **and** the built image answers
`aarch64` to `uname -m`.

Alongside it, every image's smoke test gains an architecture assertion, so the
next digest bump cannot reintroduce this silently. That assertion is the durable
half; the re-pin is the one-time repair.

---

## D-18 — one symptom, two causes, and a register entry that was half right (MEDIUM) — **CLOSED 2026-10-02**

**Severity rationale:** MEDIUM, raised from the LOW it was filed as. The
harness half is a test-only defect and was never more than LOW. The executor
half is a production defect: on Linux, a tool that is not installed is recorded
as a tool that ran and exited 127, which is the specific confusion the
architecture exists to prevent — `GateResult::Fail` and `InfrastructureFailed`
are separate types so an agent can react to them differently, and this made
the executor report the second while the record claimed the first.

**This entry was wrong twice, in opposite directions, and the correction is
the point.** The original text said:

> *"Likely a real bug in `ProcessExecutor::spawn` (probably the missing-binary
> path is treated as 'sh exited 127' rather than `ENOENT`)."*

An intermediate revision of this entry then concluded the opposite — that the
executor was correct and the whole thing was a test-harness defect. **Both
were partly wrong, and so was the 7/10 figure that seemed to settle it.**

### Evidence

Running the full suite in the Linux container: **7 of 10 failed, every one
reporting `exit_code: 127`.** That number was read as one mechanism. It is
two, and the composition is checkable from the test file rather than inferred:

- **Six** failures were the harness defect. Seven of the ten tests register the
  fixture binary and spawn it, but one of those seven,
  `infra_fail_exit_maps_to_infrastructure_outcome`, asserts `exit_code == 127`
  and so passes whether the binary runs or not. Six remain.
- **One**, `missing_executable_is_infra_failure`, never touches the fixture. It
  registers the literal path `/this/executable/does/not/exist/at/all`, so it
  was failing for an unrelated reason that happened to share the exit code.

7 = 6 + 1. The `127` was never evidence of a single cause; it was what two
different causes have in common.

The three that passed are the three that cannot fail this way, and the
arithmetic only closes if all three are accounted for:
`infra_fail_exit_maps_to_infrastructure_outcome` (127 either way),
`unknown_capability_is_rejected` (registers no tool), and
`privileged_external_class_is_rejected` (refused at registration, never
spawned). `timeout_kills_child` asserts
`Err(ToolFailed { timed_out: true })` rather than a bare 124, so it cannot
appear in either failure set on the basis of an exit code.

### Cause 1 — the harness resolved a binary by existence (6 tests)

Three sites walked to the workspace `target/` and guarded the result with
`path.exists()`:

- `crates/ironmaint-executor/tests/process_exit_codes.rs`
- `crates/ironmaint-executor/tests/artifact_spill.rs`
- `crates/ironmaint-testkit/src/executor_conformance.rs`

`containers/compose.yml` bind-mounted the repo — including `target/` — so at
`target/debug/ironmaint-fixture` sat the **host's macOS Mach-O arm64 binary**.
It exists, so the `target/release` fallback never fired, and Linux `fork`ed
successfully while `exec` failed `ENOEXEC`. The child died 127.

**Fix — `67fedbc`.** One resolver, in the crate that owns the binary, checking
that a candidate is **loadable on this host**: format against the host OS, and
for ELF the `e_machine` field, which catches a right-format / wrong-architecture
binary that a magic-byte check cannot see. The testkit re-exports it, so its
three call sites are unchanged.

**A fourth site — `3e30002`.** `bins/ironmaintd/tests/startup_shutdown.rs`
anchored on `CARGO_BIN_EXE_ironmaintd` and joined a sibling. The anchor was
sound; the sibling was not, since `ironmaintd` does not depend on the fixture
crate. Found by reading, not by the failing run — the receipt above covers the
executor suite, which does not include this one.

**Why four sites.** Cargo sets `CARGO_BIN_EXE_<name>` only for tests in the
package that declares the `[[bin]]`. `ironmaint-executor` has none, so the
variable is genuinely unset (verified: `env!` fails to compile,
`std::env::var` returns `None`). The module doc on
`bins/ironmaint-fixture/src/lib.rs` claimed the opposite, and that false belief
is what made the other three feel justified.

### Cause 2 — a missing executable was not an error on Linux (1 test, production)

`ProcessExecutor::execute` treated a failed `cmd.spawn()` as
`InfrastructureFailed`, which is correct — and on Linux it never saw one.
std uses `posix_spawn` when a `Command` is eligible, and `posix_spawn` reports
a failed `exec` by **exiting the child 127**. The parent gets `Ok(child)`.

Measured, with the executor's exact `tokio::process::Command` configuration:

| host | `Command::new("/nope/nothing")` |
|---|---|
| macOS (Darwin 25.5.0) | `Err(NotFound)` |
| Linux (Debian bookworm, glibc 2.36) | `Ok`, status 32512 (127 << 8) |

So the `Err` arm only ever fired for failures the parent can see, and "that
tool is not installed" arrived as a successful run of a process that instantly
died. It **cannot be repaired after the fact**: 127 is a legitimate exit code
in this workspace — `synthetic.build.infra_fail` returns it deliberately and
`process_exit_codes` asserts on it — so a post-hoc "exited 127 with no output"
rule would relabel a real tool failure as a missing binary.

**Fix — `c0e551b`.** A `stat` before the spawn. A `pre_exec` closure would force
the `fork`+`exec` path that does surface `ENOENT`, but that is `unsafe` and the
workspace sets `unsafe_code = "forbid"`. Bare names resolve against the
executor's own sanitised `PATH` — the one the child would have been given — so
the check cannot disagree with what the child would have found.

### Cause 3 — the container's build output was the host's (found while verifying)

`compose.yml` build-output comment cited a constraint that had already expired:
"integration tests hardcode `workspace_dir.join("target")`". True when written,
false once the fixture stopped being resolved by a hardcoded walk. The image had
anticipated the change — the Dockerfile pre-creates `/var/cargo-target` and
`clean-volumes` already removed `ironmaint_target` — so `compose.yml` was the
last file still describing the old arrangement.

**Fix — `29c8e23`.** `CARGO_TARGET_DIR` points at a named volume, and the
resolver honours it. This also removed a >20-minute cold build: `target/` was a
bind mount through the Docker VM's filesystem, and a dozen `rustc` processes
contending on it was a throughput problem, not a hang.

**Fix — `02ee489`.** Which exposed the same defect one layer up:
`verify-migrations` derived the workspace root by walking up from `current_exe`,
which only worked while the executable sat inside the tree. It now uses
`CARGO_MANIFEST_DIR`. Worth recording how this surfaced: the container run was
otherwise green and **this check printed nothing rather than failing loudly**,
because a shell wrapper had already reported the results around it. A verifier
that fails quietly is worth as little as one that is never run.

### CLOSED 2026-10-02 — the receipt

**In-container §97, which had never been green before:**

```
cargo fmt --check                                            OK
cargo clippy --workspace --all-targets --all-features        OK
cargo test --workspace                                       877 passed, 0 failed
cargo test --workspace --features integration                886 passed, 0 failed
verify-architecture / -schemas / -migrations / -mcp-schemas  OK
```

`process_exit_codes` goes **7/10 → 10/10**, and `artifact_spill` 8/8. The same
§97 set is green on the host at the same counts. A cold workspace build in the
container fell from >20 minutes (abandoned) to a few minutes.

**Teeth-checked; four of the checks could not fail when first written.**

For the resolver: a test asserting the *parser* reads two `e_machine` values
apart says nothing about the *decision*; a test asserting through
`is_loadable_here` never ran, because its `cfg!` is a compile-time constant and
returned early on every non-Linux host — green on macOS and on every CI machine;
and the resolver's rejection branch was unreachable through the real artifact,
since on any host where the workspace's own build is loadable the first
candidate always wins. The rejection is the resolver's only reason to exist, so
`select()` is now driven with synthetic planted binaries.

For the executor: a first probe **reimplemented** the target-dir resolution
instead of calling the resolver, and breaking `target_dir()` left it green —
the test was asserting on its own copy. The probe now calls the real function.
Separately, dropping the `is_file` check survived the first mutation pass,
because a directory exists, is traversable, and only `is_file` separates it; two
directory tests were added. All three executor mutations — ignoring `PATH`,
dropping the executable bit, dropping `is_file` — now each fail the one test
that owns that branch.

### Why the register was wrong — and then wrong again

This is the **third** entry whose stated reasoning had to be corrected by
running the code, after D-16's migration cost and D-05's `DaemonLock.file`
verdict. The standing rule is unchanged and is the whole of the remedy: **an
entry that has never been executed is a hypothesis**, and a hypothesis that
names the wrong file is worse than one that names none, because it looks
actionable.

The part worth carrying forward is what happened next. Given the 7/10, the
obvious correction was "the entry named the wrong crate" — the harness was
fixed, the executor was exonerated, entry closed. That correction was also
unchecked, and it was wrong: it exonerated the executor on the strength of a
127 that two different causes shared, and would have closed a production defect
as a test problem. The 127 was never evidence of a single cause.

So the rule needs its companion: **a refutation is a hypothesis too, and
"verified correct" deserves exactly as much scepticism as "probably broken."**
The thing that caught it was not more reading — the same reading had been done
twice. It was running the suite in the environment the register is about, and
being surprised by a result the theory had already explained away.

---

## D-19 — a §97 run hung in `startup_shutdown.rs`, and the teardown never reaped (MEDIUM) — **PARTLY FIXED 2026-10-02, hang OPEN**

### The register's rule, applied in the other direction

D-18 was closed by establishing that a *running* suite contradicted a written
diagnosis. This entry is the mirror case: the diagnosis is that the hang was
environmental, and **the evidence for that is a set of runs that did not
reproduce**. That is weaker than it looks. Seven attempts across three
configurations is not a refutation of a hang; it is an absence of a
reproduction, over a window in which the conditions that caused it were
deliberately absent.

So this entry stays open. The one part that is not open is below.

### What is confirmed

`Drop for Daemon` (`bins/ironmaintd/tests/startup_shutdown.rs`) was:

```rust
impl Drop for Daemon {
    fn drop(&mut self) {
        // A test that panicked mid-scenario must not leave a
        // daemon holding the state directory's lock, which would
        // make the *next* test fail for the wrong reason.
        let _ = self.child.start_kill();
    }
}
```

`start_kill` delivers SIGKILL and returns. Nothing calls `wait`, and
`command_for` did not set `kill_on_drop` (it defaults to `false`), so the reap
fell entirely to tokio's SIGCHLD orphan handler. **Eleven of the fifteen tests
in the file relied on that alone** — only `sigterm_drains_and_exits_zero` and
the first daemon in `a_second_daemon_reuses_the_persisted_database` went
through the bounded `terminate`.

The comment above claims the drop prevents a daemon "holding the state
directory's lock." It did not. Signalling is not reaping, and a reaped-away
lock file is what the next test needed; a process still in the process table is
a different problem, and one the comment does not describe.

That this was not hypothetical on this machine:

```
PID   PPID STAT  ELAPSED
52192     1 SN   02-01:00:40  ./target/debug/ironmaintd --state-dir .../tmp.GcMRbc9JnP/state ...
```

A daemon, reparented to init, two days old, pointing at a temp directory that
no longer exists — the fingerprint of a test binary killed before its `Drop`
ran. It was still resident when this entry was written.

### Teeth-checked

`a_shutdown_reaps_the_daemon_rather_than_merely_signalling_it` asserts that
when `shutdown()` **returns**, `kill -0 <pid>` reports the process gone —
which distinguishes a reaped child from a zombie, and is the property `Drop`
could never have.

The mutation: `shutdown()` reduced to `start_kill` and an early `return`.
Result: the new test fails (`10946 is still in the process table after
shutdown returned`), and **the other fifteen still pass.** That second half is
the point — the old behaviour was invisible to the entire suite. A green
`startup_shutdown` run was never evidence of correct teardown, and still is
not, on its own.

### What is ruled out

Every code-side witness checks out, so the next reader does not re-derive them:

| Candidate | Finding |
|---|---|
| `Daemon::start_on` blocking forever | bounded — `tokio::time::timeout(…, 30s)` waiting for the listening line |
| `Daemon::terminate` blocking forever | bounded — 30s, then `start_kill` + panic |
| State-dir lock contention | `fs2::try_lock_exclusive` (`crates/ironmaint-store-sqlite/src/lock.rs`) returns `WouldBlock`; it does not block |
| Port collision | `--bind 127.0.0.1:0` — the kernel assigns |
| Shared state between tests | every test gets a fresh `tempfile::TempDir` |
| A second spawner competing | no other test binary spawns `ironmaintd`; the `integration` feature gates in-process testkit binaries only |

### What is **not** ruled out

The hang showed 13 threads, the test process parked in `rt_mutex_schedule`, and
three defunct `ironmaintd` children alongside one live. Under the old `Drop`
that shape is reachable without any code being wrong: three tests' children
were signalled but not yet reaped, and the runtime had not drained them. So
the observation and the defect corroborate each other — and neither identifies
what stopped the runtime's timer wheel.

**Seven reproduction attempts, all after the conditions had changed:**

- 5 isolated runs of `--test startup_shutdown`: 0.88–1.41s each
- 1 serial run (`--test-threads=1`): 15/15 in 3.71s
- 1 full clean integration run: 886/886

### What would settle it

One of:

1. A run under a container left in the state `docker kill` produces — the
   original incident's only untested variable.
2. A core dump or `SIGQUIT` of a wedged test process, read for the runtime's
   parked-task list rather than one thread's stack. `rt_mutex_schedule` names
   the thread that was *looking*; the answer is in the threads that were not
   running.
3. The incident recurring after this fix. If the teardown was load-bearing,
   a fixed suite will hang again. That is the cheapest available experiment
   and needs nothing but patience.

### The fix

`kill_on_drop(true)` on the `Command` in `command_for`, so the panic path kills
*and* reaps; an awaited `Daemon::shutdown()` that SIGTERMs, waits 10s,
escalates to kill, and **waits again**; and `.shutdown().await` at the end of
all eleven tests that previously relied on `Drop`. `terminate()` keeps its
strict form for the two tests that are *about* §68's graceful drain.

`Drop` is now the panic path only. The `futures_executor::block_on` in a
`Drop` impl — which an earlier draft of this entry proposed — was rejected:
`Drop` runs on whichever thread releases the value, frequently a tokio worker,
and blocking there converts a leak into a deadlock.

§97 after the change: 878 unit, 887 integration, four verifiers, in-container
included.

---

## D-01 — the persistence layer has no lint enforcement (HIGH) — **CLOSED 2026-10-01**

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

### CLOSED 2026-10-01 — the four sites, and one recommendation not followed

The sweep predicted four sites and found four. All four are fixed and
`[lints] workspace = true` is on both manifests, so the §97 gate now covers 21 of
21 workspace members rather than 19.

**Three fixes followed the recommendation above.** `lock.rs:42` now branches on
`ErrorKind::WouldBlock` and keeps the `io::Error` on every other path, so a
read-only filesystem or a permissions problem no longer reports itself as
"another ironmaintd is using this state directory" — an operator is no longer sent
hunting for a second daemon that was never running. `empty_for_tests()` returns
`Result<Self, String>` and `open_in_memory` maps it into a `StoreError`, as the
entry suggested.

**The `MockStore` lock fix diverges, and the reason is worth recording.** The
recommendation was to return a `StoreError` rather than panicking. That is
correct in principle and disproportionate in practice: `read()` and `write()`
would have to return `Result<_, StoreError>`, threading a `?` through roughly
thirty-five trait method bodies in a test double, to model a state that arises
only when a test has *already* panicked.

The fix taken is `unwrap_or_else(PoisonError::into_inner)` — recovery, not
propagation. A lock poisons only when a holder panicked, so by the time the
poison is observed the original panic is already unwinding; panicking a second
time with "MockStore lock poisoned" **replaces** the real failure with a message
about a lock. For a test double, the failing test's own assertion is the more
useful signal, and recovering is also strictly closer to the §85 no-panic policy
than either `.expect` or a half-built `Result` threaded through a mock.

**The teeth-check, run before the fix was trusted.** A `read_to_string(...).unwrap()`
was added to `DaemonLock::acquire` — the production path of the crate that
previously had no lints at all — and `cargo clippy -p
ironmaint-store-sqlite --all-targets --all-features` rejected it:

```text
error: used `unwrap()` on a `Result` value
  --> crates/ironmaint-store-sqlite/src/lock.rs:58:17
```

Reverted immediately after. An hour earlier that line was invisible to the gate.
This is the standing hazard in `doc/EXECUTION-PLAN.md` §4 applied to the first
wave: a gate that has never rejected anything is an assumption wearing a green
tick, and this register has now been wrong about one of those twice.

**§97 after the fix:** `fmt` clean, clippy `-D warnings` clean, **840 unit / 849
integration** — both counts unchanged from `d1ff70a`, so nothing regressed — and
all four verifiers clean.

### Why this was missed

The lint config was added crate-by-crate during 0A, and these two crates were
created later (Phase 0B) without the `[lints]` stanza. Nothing checks for the
omission, because the absence of a stanza is not an error. A one-line
`verify-lint-inheritance` subcommand in `xtask` would close the class permanently —
see D-04's note on guardrail philosophy.

---

## D-02 — the spec-designated `SKILL.md` is stale (HIGH) — **CLOSED 2026-09-30 (0B.10 C3)**

**Severity rationale:** an agent that follows the spec's own path installs
instructions to call `check.run` with a field the server rejects. This is a
documented interface that is wrong, not merely incomplete.

### Evidence (as found)

There were **two** `SKILL.md` files, and they were not in sync:

| Path | Lines | `check.run` signature documented |
|---|---|---|
| `integrations/ironclaw/skills/ironmaint-maintainer/SKILL.md` | 68 | `{ job_id, tool_key, retry_class? }` → `{ tool_key, exit_code, stdout, stderr }` |
| `skills/ironmaint-maintainer/SKILL.md` | 227 | `check.run(check_id)` → evidence + gate outcome |

Phase 0B.9 corrected the 227-line file. The 68-line file still described the
**pre-0B.9 wire format**, which no longer exists:
`RunCheckInput` takes `check_id`, not `tool_key`, and `RunCheckOutput` no longer
returns raw `exit_code`/`stdout`/`stderr`.

### Which one was authoritative

This is the part that made it HIGH rather than LOW, and it is the reverse of the
intuition:

- `doc/phases/PHASE-0B.md:2077` — **the spec** — named
  `integrations/ironclaw/skills/ironmaint-maintainer/SKILL.md`. That was the
  **stale** one.
- `doc/operator/ironclaw-setup.md:144,153` — the operator runbook, and the file an
  operator actually copies — points at `skills/ironmaint-maintainer/`. That is the
  **corrected** one.
- `doc/operator/tool-permissions.md:9` also points at the corrected one.

So the spec and the runbook disagreed about where the skill lives, and the spec's
answer was the wrong file. A reader following `PHASE-0B.md` got the stale skill; a
reader following the operator doc got the right one.

### Resolution

`skills/ironmaint-maintainer/SKILL.md` is canonical, on the evidence the two
referring documents already formed: the runbook is what an operator copies from,
`tool-permissions.md` agrees, and the file had already been corrected in 0B.9.
Choosing the spec's path would have meant reverting that correction and re-breaking
the wire format.

Rather than keep a second copy in sync, the `integrations/` tree is **deleted** —
it contained nothing else, so `integrations/` is gone entirely. The residual-drift
hazard the fix named ("two hand-maintained copies of an agent-facing contract")
cannot recur with one file in the tree.

`PHASE-0B.md` §61 was amended to name `skills/`, with a dated note recording that
the path changed and why. The spec is the document a fresh reader trusts, and
leaving it pointing at a deleted file would have converted a wrong skill into a
missing one.

`skills/ironmaint-maintainer/SKILL.md` gained a **Human review** section in the
same commit, so the surviving copy documents the `job.resume` / `requires_human`
behaviour C2 and C3 introduced. It is 250 lines as of 0B.10 C3.

---

## D-03 — a job cannot advance past `EventDetected` (HIGH) — **CLOSED 2026-10-01 (0B.10)**

Carried from `doc/PHASE-0B-COMPLETION.md` §16.8, where it was correctly classified
HIGH and deferred to Phase 1 by decision. Re-verified here, and **widened**: the
§16.8 census covered two commands; the full census is larger. The text below is
the state at `0acaedc`, left as written because the *closure* is only legible
against it; see **CLOSED** at the foot of this entry for what changed.

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
tool, and in one case no command either.**

`AllowedAction` (`crates/ironmaint-runtime/src/next_actions.rs:21-29`) has 7
variants:

| Variant | Tool that performs it | Actually emitted? |
|---|---|---|
| `CaptureCandidate` | `candidate.capture` | yes |
| `RunCheck { check_id }` | `check.run` | yes |
| `ApplyPatch` | `workspace.apply_patch` | yes |
| `MarkObligationSatisfied` | — | yes, at `ReleaseReview` / `FinalValidation` |
| `RequestApproval` | — | yes, at `ReadyForApproval` |
| `AuthorizeOperation` | — | yes, at `Approved` / `PublicationPending` |
| `Publish` | — | **never emitted at all** — a dead variant |

All three live orphans come from the per-state fallback table at
`service.rs:1496-1515`, which the code's own comment (`service.rs:1470-1475`)
describes as the *fallback* used "when no evidence-derived action can be
produced". The evidence-derived path emits only `RunCheck`
(`service.rs:246-248`).

**Correction to the 2026-09-29 first pass.** This was initially recorded here as
"the runtime tells the agent it may act and then offers no verb", which overstates
it. The 0B.9 design is deliberate and largely sound: `SKILL.md` rule 7 states
*"It is not exposed as a tool, so calling it is not something you can do even by
accident"*, rule 8 says to stop at `ReadyForApproval`, and the behaviour is pinned
by two tests (`ironmaint-runtime/tests/approval_refusal.rs:352` and
`ironmaint-testkit/tests/synthetic_e2e.rs:280`). The agent is not being misled —
it is being told to stop.

The real defect is narrower and is a **wire-format** one. The module doc
(`next_actions.rs:1-12`) promises `allowed` is "the `AllowedAction`s it can take
**right now**". For 3 of 7 variants that is false. `allowed` is doing two jobs at
once — "moves you can make with the tools you have" and "this state needs a
principal that isn't you" — and a field that means two things can only satisfy
whichever one a reader happens to assume.

### DECIDED (2026-09-30): split the two meanings, do not overload `allowed`

The architect delegated this call. Recommendation taken:

1. **`allowed` becomes strictly honest** — only moves the tool surface can
   actually perform. `RequestApproval`, `AuthorizeOperation`, and
   `MarkObligationSatisfied` leave `allowed`.
2. **A separate field carries the human-action state**, e.g.
   `requires_principal: Option<Principal>` on `JobNextActions`. This is preferred
   over promoting them to `ActionBlocker` precisely *because* `SKILL.md` already
   warns agents "there is no `missing_approval` blocker … do not wait for a
   blocker that will never arrive". Turning them into blockers would contradict
   documented guidance; a distinct field contradicts nothing and preserves both
   facts — the job needs a human, *and* the agent has no verb for it.
3. **An enforcement test makes the class non-reintroducible**: every
   `AllowedAction` the runtime can emit must have a corresponding MCP tool. A new
   `AllowedAction` with no tool fails the gate rather than shipping as a lie.
4. **`Publish` is removed** as a never-emitted variant, and re-added by 0B.10
   wired to a real publication producer.

This preserves the current behaviour exactly — the agent still stops at
`ReadyForApproval` — while making the wire format say what it means.

**Sequencing:** lands at the top of 0B.10, not as a 0B.9 follow-up. Rationale:
0B.10 already adds the privileged-operation producers that resolve two of the
three, and a semantics change to a merged sub-phase would need a fixup commit or a
new sub-phase number. The enforcement test lands *first*, so the list cannot grow
in the meantime. `AGENTS.md` requires an explicit go-ahead before a new sub-phase
begins, so this is recorded here as a committed 0B.10 decision, not started.

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

### RESOLVED IN PART (0B.10 C1): one production caller for all three links

All three orphaned stores methods had the *same* shape — a fully-implemented
handler with no production caller — so C1 fixed them together rather than one at a
time. `RuntimeService` gained an `AdapterRegistry` (`adapters.rs`, keyed by each
adapter's own declared `AdapterDescriptor::family`), attached with a
`.with_adapters(…)` builder so no existing construction site changed. The daemon
registers `debian-stub` and `fedora-stub` at startup, replacing the
`adapter registry: deferred to Phase 1` log line.

`handle_capture_candidate` now drives, in one call:

| Link | Store method | Caller before C1 | Caller after |
|---|---|---|---|
| activate candidate | `set_active_source_candidate` | none | capture |
| materialise checks | `put_gate_definition`, `put_check` | none | capture, via the adapter's `build_plan` / `qa_plan` |
| derive obligations | `put_obligation` | none | capture, via the adapter's `PolicyPlan` |

**What is still not true, and why that is now visible rather than hidden.** The
adapters plan `debian.*` / `fedora.*` tool keys; the only registered tools in 0B
are the `synthetic.*` fixtures. So a captured `debian` job activates and gets four
materialised gates, and `check.run` on one reports
`no tool registered for capability debian.build.sbuild`. That is a strictly
better failure than the old one — before, the job sat at `EventDetected` forever
and `next_actions` said only "capture_candidate", which looks identical to an agent
that is doing everything right.

Two further guards had to move with it, and both had a *false* stated rationale
that the fix exposes:

- **`post_capture_state`** refused `check.run` in `EventDetected`..
  `CandidateAssembly` on the grounds that "pre-capture states have no active
  candidate fingerprint and cannot materialise checks". C1 makes those states
  routinely hold both. The guard is now "not terminal, not exceptional"; the two
  conditions that actually matter (an active candidate exists, and the check's
  fingerprint is the active one) were already enforced separately.
- **`attach_pending_checks`** offered `RunCheck` only once the state machine had
  emitted a `GatePending` blocker, which cannot happen before the job reaches
  `SourceIntegrity`. Runnability is a fact about the store; pendingness is a fact
  about the state machine. Mixing them meant a job at `EventDetected` advertised
  no runnable check despite having just materialised four.

`candidate.capture` now returns its side effects as `notes` on `CaptureOutput`, so
an agent can tell an inert capture from a working one without a second round trip.

The test file `crates/ironmaint-mcp/tests/check_and_operation.rs:131-134` states
the design contract explicitly: *"0B.9 does not add a `check.materialize` tool —
adapters are supposed to do this."* That contract has no implementation;
`bins/ironmaintd/src/main.rs:290` logs `adapter registry: deferred to Phase 1`.

### Risk carried into Phase 1

Phase 1 opens with the state machine unexercisable through the tool surface. Its
first adapter **must** activate the candidate and materialise the checks, or the
phase is not testing the premise it exists to test.

### DECIDED (2026-09-30): Phase 1's first acceptance criterion

The architect delegated this call. Recommendation taken, and it is now the
governing criterion for Phase 1 (§106):

> **A job must reach `ReadyForApproval` driven only through the MCP tools —
> no `RuntimeService` handle, no test-only command, no direct store access.**

This phrasing is deliberate on three points:

- **"Driven only through the tools"** rules out the pattern the 0B.9 tests
  currently use — `check_and_operation.rs:131-134` openly says it calls the
  runtime "standing in for an adapter". If Phase 1's tests keep standing in, the
  gap is merely relocated, not closed.
- **"No `RuntimeService` handle, no direct store access"** keeps the trust
  boundary honest: everything the agent does must cross the same auth'd HTTP
  transport an IronClaw agent would use.
- **"`ReadyForApproval`"** rather than "`Published`" matches §95's exit
  checkpoint and stops at the point where a human principal — not the agent — is
  the next actor. Going further would require exactly the approval machinery the
  project deliberately refuses to let the orchestrator manufacture.

Until this passes, §102 item 26 ("IronClaw can call the MCP server") is true only
in the weak sense that the socket answers. The strong sense — that an agent can
drive a job through the workflow — is what Phase 1 exists to establish, and this
criterion is what makes that measurable rather than assumed.

### Confirmed NOT the cause

The dead loop at `service.rs:614` (D-07) is **not** implicated. `reconcile`
returns `NoOp` on its first iteration, long before the loop bound could matter.

### CLOSED (2026-10-01, 0B.10 C1–C5)

Three of the four parts closed in one sub-phase; the fourth, the `allowed` wire
overload, closed in C3 as its own commit because it changes the wire format.

**The census, re-derived.** The question the original table asked — "does this
command have a tool?" — could not tell a missing link from a correct refusal.
§102 item 25 forbids MCP from *directly* writing state, gates, obligations,
approvals, or evidence, so a command with no tool is not automatically a defect.
The question worth asking is "is there a production path that issues it?", and
that one is now answerable for every row.

`RuntimeCommand` (9 variants at `0acaedc`, 12 now):

| Variant | Before | After |
|---|---|---|
| `CreateJob` | `job.create` | unchanged |
| `CaptureCandidate` | `candidate.capture` | unchanged |
| `RunCheck` | `check.run` | unchanged |
| `MaterializeChecks` | **no caller**, tests only | issued by `candidate.capture`, from the adapter's `build_plan` / `qa_plan` (C1) |
| `SetActiveCandidate` | **no caller**, tests only | issued by `candidate.capture`; unconditional, because activation is a fact about the job and not a function of whether an adapter exists (C1) |
| `RecordCheckEvidence` | **no caller**, tests only | issued by `check.run` |
| `MarkObligationSatisfied` | **no caller and no test at all** — the register called it "the closest thing in the tree to a shipped stub" | replaced by `RecordObligationOutcome` (`8d91ad6`), which the daemon, the C4 driver and the C5 driver all write. **Deliberately off MCP** per §102 item 25 |
| `RequestApproval` | no, by design — typed `Unsupported` | unchanged. A typed refusal is not a gap |
| `Reconcile` | **unreferenced**; `job.reconcile` calls the public `reconcile()` method | still unreferenced by the tool, and still a four-line pass-through. Left alone: the variant is the *enum*'s spelling of an operation the runtime exposes as a method, and deleting it would be a cosmetic change to a public API |
| `ResumeJob` | *did not exist* | `job.resume` (C2) |
| `EnterHumanReview` | *did not exist* | **no tool, deliberately** — an agent must not be able to hand itself into an exceptional state (C2) |
| `CreateReleaseCandidate` | *did not exist* | `release.candidate.create` (C5 prerequisite, `d98d780`) |

`RuntimeQuery` (5 variants at `0acaedc`, 6 now): `GetJob`, `ListNextActions`,
`GetCheckOutcome` and `GetOperation` are all reachable through `dispatch`.
`GetProjection` is still test-only and still has no tool — it is the raw
projection, which `job.get` already returns in a shape the wire can carry, and
exposing it would be §102 item 24's "storage primitive" rather than a domain
capability. `GetReleaseCandidate` joined it at the C5 prerequisite.

**Two rows deliberately did not change, and that is the finding.** The commands
with no tool split cleanly into two kinds. The four that *write* domain state
(`MaterializeChecks`, `SetActiveCandidate`, `RecordCheckEvidence`, the
obligation writer) got a production caller. `RequestApproval`, `Reconcile` and
`GetProjection` did not, and **not** because they were missed — §102 item 25
forbids an MCP tool from writing state, and item 24 forbids one from exposing a
store primitive. The register's original question could not tell those apart, and
that is why "5 of 9 commands have no MCP entry point" read as five defects when
it was two defects and three correct refusals.

The `allowed` overload: C3 split it. `allowed` is now strictly tool-performable
and the human-action case moved to a separate field, and the enforcement is
stronger than a test — `ironmaint_mcp::tool_for_action` is an exhaustive `match`
with no wildcard arm, so adding an `AllowedAction` without deciding which tool
performs it **fails to compile**. The never-emitted `Publish` variant is
removed.

**The proof that matters.** `crates/ironmaint-testkit/tests/mcp_acceptance_scenario.rs`
(C5, `2656528`) walks the whole of §101 — thirty steps, four candidates, a failed
build, a failed QA run, a failed mandatory obligation, three repair loops, a
release-candidate snapshot, a restart — with **every** state change arriving as a
`dispatch(McpToolName, json)`. It holds no `RuntimeService`, no `RuntimeQuery`,
and no store handle for writes; it reads the store only for facts no tool
exposes, and the module doc says which and why. That is the criterion this
register wrote down as *"governing for Phase 1 (§106)"*, satisfied one sub-phase
early and one sub-phase before Phase 1.

**Two things the census did not record, both found by that walk**, and both the
same shape as the ones above — a component that models something correctly and a
production path that never produces the input:

- `capture_candidate` re-captured an unchanged tree and reached
  `put_source_candidate` without looking the fingerprint up. `migrations/0001`
  declares `source_candidates.fingerprint` UNIQUE, so the second capture of the
  same tree was a hard `UNIQUE constraint failed` on SQLite. `MockStore`
  overwrites silently, so a mock-backed suite would have called this a harmless
  duplicate row. Fixed in `crates/ironmaint-workspace/src/capture.rs` (`ec9e06c`).
- `next_actions` reported blockers but not `requires_human` for an outstanding
  obligation once the job was past `EventDetected`, so a fresh job's
  `next_actions` returned nothing at all. Fixed in
  `crates/ironmaint-runtime/src/service.rs` (`f009e2e`).

**Risk carried into Phase 1, restated.** A captured `debian` job activates and
materialises four gates, and `check.run` on one reports `no tool registered for
capability debian.build.sbuild` — the adapters plan `debian.*` / `fedora.*` and
the only registered tools in 0B are the `synthetic.*` fixtures. That remains a
strictly better failure than the original: it is visible, typed, and reported in
the capture's own `notes`, where before the job sat at `EventDetected` forever
saying only "capture_candidate", which reads identically to an agent doing
everything right.

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

### CLOSED 2026-10-01 — promoted, and the promotion immediately found something the measurement had missed

`dead_code = "deny"` in the workspace lint table. The comment above it was
rewritten at the same time, because the old text still said *"Reachable code and
unused imports are noise during skeleton bootstrap"* — a phase this repository
left long ago, and a rationale that read as a reason not to.

**The ordering was the point, and it was not cosmetic.** D-08 landed first, on
2026-10-01, and it is what removed the last two `#[allow(dead_code)]` markers in
the executor. Promoting before that would have compiled clean and taught nothing:
the two fields D-08 was about were exempted, so a `deny` landing over them would
have been indistinguishable from a `deny` landing over a crate that genuinely has
no dead code. The two fixes land together, and in the order that lets the
promotion be **teeth-checked** rather than assumed.

**What the promotion actually caught.** The 2026-09-29 measurement above says the
workspace compiles clean "except for exactly two sites". That is no longer true,
and the extra site is the interesting one: `DaemonLock.file` in
`ironmaint-store-sqlite/src/lock.rs`. The earlier measurement did not find it
because the 0B.10 test files had not yet been written — it was a production-only
sweep. `lock.rs` is production code and was there all along, so the miss was that
the earlier pass did not *build* with the deny in place; it reasoned about which
fields carried the marker instead. The field has carried the marker the whole
time, and removing it to see what happened is the only way to find that.

D-05's original analysis of that field was wrong in a way that would have made the
fix a **silent regression** — it recommended deleting a field whose only job is
its `Drop`. See the correction in D-05.

**Teeth-checked**, deliberately in a test target rather than an obvious production
one, because that is where `warn` was useless:

```text
error: function `teeth_check_this_is_dead` is never used
   --> crates/ironmaint-store/tests/smoke.rs:584:4
error: could not compile `ironmaint-store` (test "smoke") due to 1 previous error
```

**Result:** 852 unit / 861 integration (each +1 — D-05's new facade test), fmt
clean, clippy `-D warnings` clean, four verifiers clean.

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
| `crates/ironmaint-store-sqlite/src/lock.rs:23` | `DaemonLock.file` | **The allow is correct; this row's original claim was not.** The field is genuinely load-bearing — closing the `File` releases the OS flock, which is the entire design — but it is *never read*, and the derived `Debug` does not count: rustc ignores derived `Debug` during dead-code analysis and says so. **See the correction below.** |
| `crates/ironmaint-executor/src/process.rs:72` | `ProcessExecutor.artifact_store` | Legitimate reservation → **D-08**. |
| `crates/ironmaint-executor/src/process.rs:76` | `ProcessExecutor.guard_factory` | Legitimate reservation → **D-08**. |
| `xtask/src/schemas.rs:218` | `pub fn schemas_path()` | **Grey zone — resolved by deletion.** Doc comment named its consumers as "CI scripts, docs, architecture.rs"; none existed. `xtask` is a binary crate, so there is no out-of-workspace consumer to be missing — the "cargo cannot see" caveat that makes `#[allow(dead_code)]` defensible on a `pub fn` does not apply to it. |

### Why this matters beyond tidiness

`fn _kind()`, `fn _ensure_time_import()` and `fn _ensure_used()` are dead functions
kept alive by a lint suppression, one of which is documented with a reason that
does not hold. That is the exact shape the project rule forbids — *"Stubs are not
a closing mechanism."* They are small, but the class is real and the comment
misinformation is worse than the code.

**Fix:** delete the three functions and the two now-unused imports; drop the allow
in `lock.rs:23`; decide `schemas_path`. Cheap, and it makes D-04's promotion
honest.

### CLOSED 2026-10-01 — nine sites cleared, and the register's own claim about `lock.rs` was wrong

Landed with D-04. Eleven `#[allow(dead_code)]` sites existed; **one remains**, and
it is the one the register had argued against.

**The correction.** This entry said of `DaemonLock.file` that *"the allow is
wrong … it is read by the `#[derive(Debug)]` and by `File::drop`. It does not need
the suppression."* Both halves of that are incorrect, and the compiler says so
directly. Promoting `dead_code` to `deny` and building produces exactly one error
at that site:

```text
error: field `file` is never read
  --> crates/ironmaint-store-sqlite/src/lock.rs:32:5
note: `DaemonLock` has a derived impl for the trait `Debug`, but this is
      intentionally ignored during dead code analysis
```

`File::drop` is not a *read* in the lint's sense, and the derived `Debug` is
explicitly excluded from the analysis. So the field is reported, and the field
cannot be deleted: dropping it closes the file, which releases the OS flock,
which is the entire mechanism. A field held for its `Drop` is a real pattern the
lint cannot see, and the honest response is a **reasoned** allow, not a delete that
silently stops locking.

The allow stays. Its doc comment now records the compiler's own diagnostic as the
receipt, so the next reader who is tempted to "clean it up" can check rather than
guess. This is the second time in this register that an entry's reasoning had to
be corrected by running the code — the first was D-16's migration cost estimate,
which was wrong in the *cheap* direction. **A register entry that has never been
executed is a hypothesis.**

**What went.** The three vestigial anchor functions, and the two dead imports they
were keeping alive:

| Site | Deleted | Import consequence |
|---|---|---|
| `ops/workspaces.rs:89` | `fn _kind()` | `StoreErrorKind` orphaned → removed |
| `executor/src/operation.rs:139` | `fn _ensure_time_import()` | none — the import was already used |
| `xtask/src/mcp_schemas.rs:99` | `fn _ensure_used()` | `McpToolName` orphaned → removed |
| `xtask/src/schemas.rs:218` | `pub fn schemas_path()` | `Path` orphaned → removed |
| `tests/handle_run_check.rs:643,654` | `_query_projection`, `_imports_anchor` | `RuntimeQuery`, `MaintenanceEventId` orphaned → removed |
| `tests/capture_candidate.rs:339,354` | `_imports_anchor`, `_system_clock_anchor` | `MaintenanceEventId`, `Evidence`, `CandidateFingerprint`, `SystemClock` orphaned → removed |
| `tests/hardening.rs:114` | `fn fingerprint_zero()` | none |
| `tests/smoke.rs:559` | `_policy_baseline_anchor` | `PolicyBaseline` orphaned → removed |

Each row was checked for a real consumer before deletion rather than assumed. The
outcome split three ways: the import was genuinely used elsewhere and the anchor
was dead; the import was used *only* by the anchor, so both went; or — the
`hardening.rs` case — the anchor was a copy-paste duplicate of two byte-identical
helpers in `plan_run_check.rs` and `materialize_checks.rs`, used by neither in its
own file.

**Three sites were not deleted, because deleting them would have been worse.**

`tests/smoke.rs`'s `_facade_satisfied` was a *real* compile-time check expressed
as a dead function — the exemption was standing in for the assertion, which is the
failure mode this entry exists to remove. It is now a `#[tokio::test]` that
constructs the facade and round-trips a bundled method, so the check runs and the
lint has nothing to suppress. Note it is `impl IronMaintStore`, **not**
`Box<dyn IronMaintStore>`: the facade bundles traits with generic methods and is
deliberately not dyn-compatible, so the `Box<dyn>` spelling does not compile. The
first attempt used it.

`tests/capability_object_safety.rs` carried a **file-level** `#![allow(dead_code)]`,
which is the worst shape in this register: it exempts every line, so any genuinely
dead item added to that file later would have been invisible. The seven unused
`fn _v(_: &dyn Trait)` probes are now each bound to a `let _: fn(&dyn Trait) = …`,
which is the coercion that *is* the object-safety assertion. They are seven
separate `let`s rather than one tuple because `clippy::type_complexity` reads a
seven-element tuple of `fn(&dyn Trait)` as a complex type, and the fix for that
warning would be a type alias — more machinery than the assertion is worth.

`lock.rs`, above.

**Teeth-checked.** A deliberately dead `fn teeth_check_this_is_dead() -> u32` was
appended to `tests/smoke.rs` and the build rejected it:

```text
error: function `teeth_check_this_is_dead` is never used
   --> crates/ironmaint-store/tests/smoke.rs:584:4
error: could not compile `ironmaint-store` (test "smoke") due to 1 previous error
```

Deliberately in a **test target**, because that is where the old `warn` setting
would have scrolled past unremarked. The check is not "does the deny fire on an
obvious production case" — it is "does it fire where the old setting was
useless".

**The duplication is left in place deliberately.** `fingerprint_zero()` now exists
twice, identically, in `plan_run_check.rs` and `materialize_checks.rs`. The obvious
consolidation is a `tests/common/` module, and that is wrong here: a helper in a
shared module that a given test does not call is itself dead code, which the new
`deny` rejects — reintroducing exactly the problem this entry closes, one level
up. Recorded rather than fixed.

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

**Same root cause as D-15**, and they should be closed on the same machine in the
same sitting: the repro steps are written out there, and the manifest's tool list
has since grown to eleven (`job.resume` at C2, `release.candidate.create` at the
C5 prerequisite), so the round-trip is worth more than it was on 2026-09-29.

---

## D-07 — dead loop in `reconcile`, suppressed (MEDIUM) — **CLOSED 2026-10-01 (0B.10 C4)**

`crates/ironmaint-runtime/src/service.rs:614` at `0acaedc`:

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

### The decision, and the evidence for it (0B.10 C4, `cec87d7`)

The semantic question was answered by the spec, twice, and the register did not
look:

- **§41** — reconciliation "may continue through multiple trivially satisfied
  stages", and the mandated call is `runtime.reconcile(job_id)`, once.
- **`ReconcileOutcome::Advanced`'s own doc comment** already promised "the
  projection advanced through **one or more** rules".

So the intended reading was multi-transition, and the code contradicted its own
type documentation. `reconcile` now walks every satisfied rule and stops where
the next one is blocked; the `#[allow]` is **deleted**, not re-suppressed, and
the clippy gate is what keeps it deleted.

The cost of the old behaviour was not subtle. An agent calling `job.reconcile`
at `EventDetected` with every gate passing was told to call it twelve times to
cross §20's path, and nothing in the response explained why the walk stopped
where it did. C5's driver hit this: step 18 asserted an unsatisfied mandatory
obligation, and `reconcile` had already carried the job past it on the previous
call, so the state was `SourceRevision` rather than the `EventDetected` the test
expected.

**The lesson is worth more than the fix.** The suppression was a design note
written in the wrong place: someone read `clippy::never_loop`, silenced it, and
the reasoning that should have settled the semantic question was never written
down anywhere. It is the second time in this phase that a suppressed lint has
been a design decision deferred to a future reader. **Grep for
`#[allow(clippy::` and read the argument each time.**

---

## D-08 — `ProcessExecutor` holds two fields it never reads (MEDIUM) — **CLOSED 2026-10-01**

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

### CLOSED 2026-10-01 — the reservation is now the behaviour, and the reservation was hiding two more defects

`execute()` uses both. The closure took a working implementation rather than a
deletion, because "the fields are removed" closes the entry and leaves the MEDIUM
defect — the truncated build log — exactly where it was, and deletes the
`ArtifactStore` alongside it.

**The shape of the fix.** Each stream is read through `bounded_read`, which keeps
at most `stdout_max_bytes` in memory for the record's ergonomic copy and forwards
*every* byte down a 64 KiB bounded channel to a task that streams it into the
store. Backpressure is what makes this safe: a child emitting a gigabyte is
slowed rather than buffered, so the memory bound the old code was defending is
kept. The artifact is the complete stream; the record's `stdout` is the prefix.

Three things the record has to say, or the fix is half a fix:

| Field | Why it cannot be omitted |
|---|---|
| `artifacts: Vec<SpilledArtifact>` | The digest. A content-addressed store with no index is a place bytes go; a record naming none of them is a run whose output cannot be recovered. That is the *same* silent-truncation defect, one level up. |
| `dropped_bytes` | The artifact is bounded (§15 asks for a bounded capture). Without the number an operator reading a 1 MiB artifact cannot tell "the tool printed 1 MiB" from "the tool printed 40 and we kept 1". `retained + dropped` reconstructs what the tool actually printed, which is what makes it a measurement rather than a flag. |
| `artifacts_dropped: Vec<DroppedArtifact>` | §15's caps are **refusals**, not silent trims. A spent budget must be a fact on the record, with a reason that distinguishes `budget_count` from `budget_bytes` from `store_cap`. An empty `artifacts` with an empty `artifacts_dropped` means nothing was lost; a non-empty one means it was. |

The refusal is deliberately **not** an error. The tool already ran, its exit code
is a fact, and refusing to keep a log does not make a build any less passed or any
less failed. It only changes what can be read afterwards, and the record says so.

**`ExecutionRequest` gained a `job_id`, and that was the actual blocker.** The
guard factory is `Fn(JobId) -> Arc<JobArtifactGuard>` and the request had no job
on it, so `execute()` *could not call the guard* even if it had wanted to. The
field is required rather than optional for the same reason the reservation was
worth closing: a per-job cap over a request that does not say which job is not a
per-job cap. All 17 call sites now pass one.

### Two defects the reservation was hiding

Both were live for as long as the fields were unread, and neither was findable,
because the code that would have hit them was never reached.

**1. The "per-job" caps were per-call.** Neither `factory_from_config` nor
`permissive_guard` memoises: both return a *fresh* guard on every call, so
`count` and `bytes` reset to zero each time and no cap can ever be reached. The
type's own doc comment says "caps outlive transient executor errors and are
reused across retries", which is a description of behaviour the implementation
did not have. `ironmaint-executor/tests/artifact_spill.rs` found it on its first
run, by asserting that a second run on the same job would be refused.

The fix splits the two things one type was conflating. The factory is now a
**policy** — "what caps apply to this job" — and `ProcessExecutor` memoises it in
a `GuardCache` keyed by `JobId`. Putting the memoising in the executor rather than
in the factory is the part that matters: a caller cannot construct their way out
of the accumulation the caps exist to enforce. A test that hands the executor
`Arc::new(|job| Arc::new(JobArtifactGuard::new(...)))` — which is exactly what the
first draft of the test did — now gets a genuinely per-job budget, where before it
would have got a fresh one and a false pass.

**2. The truncating writer rounded its own cap down.** `write_from_truncating`
was written to stop at the cap, and it stopped at the largest whole read-buffer
that fitted: ask for 32 768 and receive 28 672. A cap that silently rounds itself
down by an arbitrary amount is not the cap that was asked for, and the caller
cannot see the shortfall. The executor's test caught it as
`assertion left == right failed — left: 28672, right: 32768`. It now writes the
part that fits and drains the rest.

A third, smaller one is worth naming because it is the kind that survives review:
the first draft closed the pipe to the spill task with
`AsyncWriteExt::shutdown()`. On a `DuplexStream` that tears down **both** halves
and discards the buffer, so a tool that printed 256 KiB produced a 0-byte
artifact — a spill that reported success while having retained nothing, which is
the exact failure mode the entry exists to prevent. Dropping the write half is
what signals EOF.

**Closes when (revisited):** both fields are read by `execute()`, the daemon
calls `with_guard_factory(factory_from_config(LimitsConfig::default()))`, and
`crates/ironmaint-executor/tests/artifact_spill.rs` drives the real fixture binary
through the real executor. The two `#[allow(dead_code)]` markers are gone, which
is what makes D-04's `dead_code = "deny"` promotion honest for this crate —
`doc/DEBT.md`'s note that D-04 and D-08 "land together or not at all" is satisfied
in the order that lets D-04's teeth be checked against a crate that no longer
needs the exemption.

---

## D-09 — no `CaptureResume` / `infrastructure_blocked` (LOW) — **PARTLY CLOSED 2026-10-01 (0B.10 C2)**

A tool that fails for infrastructure reasons (sandbox, worker, DB) has no way to
express "resume this later" as opposed to "this package did not build". Was
scheduled for 0B.10. See `doc/PHASE-0B-COMPLETION.md` §16.4.

### What C2 built, and what it did not

The entry as originally written conflated two things that the code separates
cleanly: a tool's ability to *signal* `InfrastructureBlocked`, and the
machinery's ability to *leave* it. 0A §21 requires a recorded resume state for
both exceptional states symmetrically, and C2 found that neither had a durable
home — `ResumeRecord` existed and was serialisable, but was constructed only in
tests, because `JobEvent` had three variants and none of them was one.

C2 gave the record somewhere to live and an exit that consumes it:

| Piece | Where |
|---|---|
| `JobEvent::ResumeRecorded(ResumeRecord)` — the §21 "recorded event containing the resume state" | `ironmaint-state/src/event.rs`; a pass-through in `apply.rs`, skipped by both `rebuild_projection` implementations |
| SQLite serialisation for the new variant | `ironmaint-store-sqlite/src/ops/events.rs` |
| `RuntimeCommand::EnterHumanReview` — writes the record **at the moment of entry**, from the pre-transition projection | `ironmaint-runtime` |
| `RuntimeCommand::ResumeJob` — reads the last `ResumeRecorded` whose `from` matches the current state | `ironmaint-runtime` |
| `job.resume` | `ironmaint-mcp` |

Both exceptional states share the exit machinery, so `job.resume` works for
`InfrastructureBlocked` the moment a producer exists. **That producer is the
part still open**, and 0B.10 did not add one: the only writer of an exceptional
transition in 0B is `EnterHumanReview`, and no tool can call it — deliberately,
so an agent cannot hand *itself* into a state only a human should enter.

So D-09 is now precisely: **the state has a way out and no way in.** The
remaining half needs a decision about who may mark a job infrastructure-blocked
and on what evidence, and that decision is a Phase 1 question — it is the first
thing real adapters will hit, because real adapters do fail their sandboxes.

`ResumeJob`'s refusal is worth stating, because the obvious "helpful" behaviour
is the wrong one: with no record, it returns a typed `InvalidInput` naming the
requirement. It does **not** scan the log and infer a target. The test
`resume_without_a_record_is_refused_rather_than_inferred` strips the
`ResumeRecorded` event from a real log and replays the rest, and every
`Transitioned` event survives — so an implementation that inferred would pass
every other test in the file. 0A §21's prohibition has no other guard against
quietly regressing.

---

## D-10 — the daemon runs no reconcile loop (LOW) — **NOT TAKEN, and the reason is now recorded**

`reconcile` is reachable only when an agent calls `job.reconcile`. Nothing drives
it proactively. See §16.6.

This was scheduled for 0B.10 and 0B.10 did not build it, which would otherwise
read as an oversight. It is a decision, and re-deferring it a fourth time would
be worse than recording why.

**§41 defines `reconcile` as a per-call function** — `runtime.reconcile(job_id)`,
called by the caller, returning where it stopped. No spec text asks for a loop.
§67's startup sequence has no reconcile step. And the argument for one was always
weak: an agent is the orchestrator here, and a daemon that advanced jobs behind
its back would put state changes in the audit log that no actor requested, which
is precisely the property the rest of the design works to preserve.

The original note ("a background loop would have nothing to do anyway, until a
job can leave `EventDetected`") is now stale — D-03 is closed, so a loop would
have something to do. It is kept here rather than deleted because the way it
became stale is the useful part: a limitation written as a *consequence* of
another defect outlives the defect and starts to look like a requirement.

**Revisit when there is a first consumer**, and name it in Phase 1's plan. The
honest candidate is a stalled job at `EventDetected` with all gates passing and
no agent asking — an agent-initiated design has no way to notice that.

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

## D-14 — `rebuild_projection` rejects every real job (MEDIUM) — **CLOSED 2026-09-30 (0B.10)**

**Found 2026-09-30**, while writing 0B.10 C2's projection-replay test. It is not a
C2 regression; C2's event variant is additive and every pre-existing test still
passes.

`ProjectionStore::rebuild_projection` refuses any log whose first event is not a
`JobEvent::Transitioned`:

```text
"first event is a Domain reference; no projection to seed rebuild"
```

But `handle_create_job` seeds the projection with `put_projection` and *then*
appends a `JobEvent::Domain` as sequence 1. So the first event of every job the
runtime has ever created is a `Domain` reference, and the rebuild **always**
fails.

**Why it matters:** `ironmaintctl rebuild-projections` (`bins/ironmaintctl/src/rebuild.rs`)
is the §36 operator escape hatch for "a projection row drifts from the event log"
— manual edits, recovery from a corrupted DB. On any real database it reports
every job as an error and rebuilds nothing. It is the tool an operator reaches for
precisely when things are already wrong, and the tool silently does nothing. It
also means §101 step 30 ("entire history is reconstructable after restart") cannot
be demonstrated through this path.

**Why the existing tests miss it:** every `rebuild_projection` caller in
`ironmaint-store-sqlite/tests/` builds its log by hand, starting from a
`Transitioned`. No test drives `CreateJob` and then rebuilds.

**The fix is not obvious, and that is why it is logged rather than fixed inline.**
The projection's *initial* contents live in the `put_projection` write, not in any
event — a `Domain` event carries only an id. So "rebuild from the log alone" is
genuinely impossible for a job that has never transitioned. The real design
question is whether the job-creation seed should emit a `Transitioned` (it is not
a transition, so probably not), whether `rebuild_projection` should read the
existing projection row as the seed and only replay from there (defeats the
purpose of the escape hatch), or whether the rebuild contract should be
"replay events *after* the first transition" with the seed taken from the DB.

**Deliberately not fixed in C2:** `AGENTS.md` — "One feature or fix per branch. Do
not stack unrelated work on branch." C2 is exceptional states and resume; this is
a pre-existing defect in a different subsystem, found by C2's tests. Fixing it
here would also have required a design decision this commit should not make.

**It blocks §101 step 30** (C4), so it needs a decision before C4, the same way
`RecordObligationOutcome` does.

### Resolution

The three options above were all ways of *not* making the log sufficient: seed
from the projection row (the very row the escape hatch exists to verify), or
narrow the contract to "replay after the first transition". Either leaves §102
item 3 — "Job projections can be rebuilt from events" — false for a job that has
never transitioned, and leaves `rebuild-projections` dependent on the row it is
meant to check.

The fix is a fourth option the entry did not list: make the log carry the
projection's birth. `JobEvent::JobCreated(JobProjection)` is appended by
`handle_create_job` as sequence 1, ahead of the `Domain` audit reference, and a
replay starts from it.

A projection is not a transition, so this is not modelled as one — it has no
`from`/`to` and never advances the FSM. It is the record that a job came into
being and what it looked like when it did.

Seed selection also moved out of the two store backends into
`JobEvent::seed_projection`, which is where the drift started: the mock and the
SQLite implementation were separate copies of the same decision, and only one of
them was ever exercised against a real job's log.

`crates/ironmaint-runtime/tests/projection_rebuild.rs` closes the gap that let
this ship. Every pre-existing rebuild test hand-assembled a log beginning with a
`Transitioned`; those tests now drive the real `CreateJob` path against a real
SQLite store, so a regression in what the runtime emits fails in CI rather than
in production.

### AMENDED 2026-10-01 — the closure is necessary and not sufficient

**This entry's claim that the log is now authoritative is true about the birth
and false about everything after it.** `JobCreated` carries the projection as it
was at sequence 1. Nothing else in `JobEvent` carries a payload except
`Transitioned` and `ResumeRecorded`, so the only field of `JobProjection` a
replay cannot recover is `active_candidate` — which `activate_candidate` sets
without ever writing an event that records it.

So the log is authoritative for a never-transitioned job's birth, and **not**
authoritative for any job that has captured a candidate. §102 item 3 is narrower
than this entry's resolution implied. The follow-on defect is **D-16**, found
three days later by running the binary rather than by reading the code, and it
is worth noting that this entry was marked CLOSED in the interval.

What the follow-on also found is that the *other* half of the escape hatch was
broken independently, and the same blind spot hid it: `rebuild-projections` took
its CAS token from the rebuilt projection rather than the stored row, so it
could not write back even for a job whose log was authoritative. See **D-16** for
both.

---

## D-15 — §102 item 30 is unverifiable in this environment (LOW, environment)

**This is not a code defect and there is no diff that closes it.** It is recorded
because 0B.10's DoD status is "33 of 34", and an unqualified "33 of 34" invites
the reader to assume the thirty-fourth was checked and failed.

§102 item 30 — *"An actual IronClaw agent can complete the synthetic repair
workflow"* — requires an `ironclaw` binary. Two independent blockers, both
verified on 2026-10-01:

```console
$ which ironclaw
ironclaw not found

$ rustup toolchain list
stable-aarch64-apple-darwin (default)          # 1.94.0
nightly-2025-11-21-aarch64-apple-darwin
1.85.0-aarch64-apple-darwin
1.94.0-aarch64-apple-darwin (active)           # the workspace MSRV

$ grep channel doc/vendor/ironclaw/rust-toolchain.toml
channel = "1.98.0"
```

1. **No released binary on `$PATH`.** The documented path is
   `ironclaw extension install doc/operator/mcp-registration.yaml`
   (`doc/operator/ironclaw-setup.md` §2), which is a distribution artifact, not
   something this repo builds.
2. **The submodule pins a toolchain nobody here has.** `1.98.0` is not in the
   list above, and raising the workspace to match is not an option:
   `Cargo.toml:30` declares `rust-version = "1.94"` and every crate inherits it.
   Note that nothing *enforces* 1.94 today — there is no CI workflow in the repo
   — so this is a declared floor, not a gate. Worth knowing, and worth a real
   gate later; it is not what blocks item 30.

Building it is also beside the point. `CLAUDE.md` designates
`doc/vendor/ironclaw/` a **reference** — "not built as part of this workspace" —
and `AGENTS.md` treats a submodule build as its own unit of work. No sub-phase
owns it, and 0B.10 listed it as explicitly out of scope.

**What is *not* blocked, and is the reason this is LOW rather than a gap in the
phase.** Every other DoD item that a live agent would exercise is exercised
without one. The C5 driver is a conforming MCP client, holds no `RuntimeService`,
and completes §101 end to end (`mcp_acceptance_scenario.rs`). §102 item 26
("IronClaw can call the MCP server") is met in the strong sense: an
authenticated MCP client calls all eleven tools and drives a job to
`ReadyForApproval` over the same dispatcher. What item 30 adds on top is
IronClaw's *own* agent loop and its extension manifest — which is D-06, still
open for the same underlying reason.

So the honest claim is: **the workflow is proven agent-drivable; the specific
agent named by the spec has not been run against it.** A green §97 gate and a
green C5 do not silently stand in for it, and this entry is what stops them from
being read that way.

### Repro steps for the machine that closes it

```sh
# 1. An `ironclaw` binary on $PATH.
which ironclaw && ironclaw --version

# 2. A daemon with the real tool surface.
cargo build -p ironmaintd
./target/debug/ironmaintd --bind 127.0.0.1:7341 \
      --token-file /tmp/tok --state-dir /tmp/state &

# 3. The extension, from the manifest the daemon actually serves.
ironclaw extension install doc/operator/mcp-registration.yaml
ironclaw extension activate ironmaint
cp -r skills/ironmaint-maintainer ~/.ironclaw/installed_skills/
ironclaw skill refresh

# 4. The smoke test. It skips the extension checks with a notice when no binary
#    is present, so "OK" alone does NOT mean item 30 passed — read the notice.
IRONMAINT_TOKEN_FILE=/tmp/tok ./scripts/ironclaw-e2e.sh

# 5. Item 30 itself: drive the §101 repair workflow through the IronClaw agent
#    and confirm the job lands at ReadyForApproval with the candidate-bound
#    evidence chain intact.
```

Step 5 has no script yet, and writing one is the natural first task on a machine
that can run it. `scripts/ironclaw-e2e.sh` verifies the *server*; it does not
verify the *agent's repair loop*. The gap between those two is exactly what item
30 asks for.

---

## D-16 — the event log does not record candidate activation, so `rebuild-projections` cannot rebuild any job that has captured one (MEDIUM)

**Found 2026-10-01**, by running `ironmaintctl rebuild-projections` against the
live state directory of a daemon built from 0B.10's tool surface. This is the
first defect in this register found by *operating the product* rather than by
reading or unit-testing it, and the way it was found is the argument for the
method.

### The log records *that* something happened, not *what*

`JobEvent` has five variants. Only three carry a payload:

| Variant | Payload | Replayed into the projection? |
|---|---|---|
| `JobCreated(JobProjection)` | the birth projection | yes — the seed |
| `Transitioned(Transition)` | `projection_after` | yes |
| `ResumeRecorded(ResumeRecord)` | the record | no — pass-through, like `ToolRunFinished` |
| `Domain(uuid)` | a bare identifier | no |
| `ToolRunFinished(uuid)` | a bare identifier | no |

`activate_candidate` writes `active_candidate` onto the projection, bumps
`version`, and appends `JobEvent::Domain(domain_event_id)` — a `DomainEventId`
and nothing else. There is no candidate id in it, and no path by which one could
be recovered. A replay therefore reproduces the job's `state` faithfully and
arrives with `active_candidate: None`, for every job that has ever captured a
candidate.

§30 binds every gate verdict to the active candidate's fingerprint, and §22
makes a source change create a *new* candidate. So `active_candidate` is the
highest-value field in `JobProjection` and the only one the log cannot justify.

### What the tool does now, and why that is the right interim answer

`replay_one` **refuses**, naming the missing fact:

```text
refusing to rebuild: the event log does not record candidate activation, so
the replay would drop the active candidate (stored 0196…, replayed None).
Repair this row by hand, or wait for the log to carry activation.
```

This is `8b95884`. The alternative — writing the replay anyway — is strictly
worse in every dimension, and worth spelling out because it is the tempting
one: the write would succeed, the tool would report `rebuilt: 1 job(s)`, and
the job would lose the binding that every gate result already recorded. Not a
partial loss — an *invalidating* one. The operator would be told the job was
repaired at the moment its evidence chain was broken.

A repair tool that destroys the thing it repairs is worse than one that refuses,
which is why the interim answer is a refusal with a name in it. An operator can
act on "the log does not record activation" and on nothing else.

### AMENDED 2026-10-01 — the same gap takes `version` with it

The entry above names one field. It is two, and the second is currently being
masked.

`handle_set_active_candidate` (`crates/ironmaint-runtime/src/service.rs`) does
two things to the **row**: it sets `active_candidate`, and it writes
`version: current.version + 1`. It then appends `JobEvent::Domain(id)` — a bare
identifier, which `ProjectionApply::apply` treats as `self.clone()`, a no-op. So
**neither** the activation nor the version bump it performs reaches the log. A
replay reproduces the last `Transitioned` event's candidate *and* version, which
are the values from before the activation.

Only a `Transitioned` event would carry either one forward, because its payload
embeds a full `projection_after` snapshot — and activation does not transition.

**Why this matters for the fix rather than the report.** `replay_one` sets
`proj.version = expected`, taken from the stored row, to make the CAS token
correct. That line was written to fix the *thirteenth* instance and it is right
to exist, but it also silently sources `version` from the very row the tool
exists to verify. A `CandidateActivated` variant carrying only the candidate id
would satisfy the refusal above, the refusal would go away, and `version` would
still be unlog-justified with nothing left to notice.

So the variant needs a fourth field:

```text
JobEvent::CandidateActivated { job_id, candidate_id, fingerprint, version_after }
```

and `xtask verify-seams` S7 — a round-trip asserting every `JobProjection` field
against a production path — is what keeps a future variant from shipping half
this again. See `doc/SEAM-VERIFICATION.md` §2/S7, whose first run is expected to
fail on exactly these two fields.

### The real fix, and what it costs

A new event variant carrying the candidate id **and the version it left behind**,
on the D-14 pattern:

```text
JobEvent::CandidateActivated { job_id, candidate_id, fingerprint, version_after }
```

appended by `activate_candidate` in place of (or alongside) the `Domain`
reference, and applied in `JobProjection::apply` as a pass-through that sets
`active_candidate`. `Domain` is *not* the variant to widen: it is used for
several unrelated domain writes, and the reason it carries a bare id is
deliberate — it is an audit back-reference, not a state record. Widening it
would make every `Domain` event ambiguous about which of its uses it is
answering.

The costs, stated so the next reader is not surprised:

- **It changes the durable event format.** `serialise_event` / `deserialise_event`
  in `ironmaint-store-sqlite` gain an arm, so every existing database needs a
  migration or a forward-compatible deserialiser. This is the reason it is not
  a small follow-up.

  > **CORRECTED 2026-10-01 — no migration is needed, and the estimate above was
  > wrong.** `migrations/0001` declares `events.event_type` as unconstrained
  > `TEXT` with no `CHECK` and no enum, so a new variant is a new *string* rather
  > than a new column value the schema would reject. `verify-migrations` was
  > re-run after the variant landed and stayed clean, which is the confirmation
  > rather than an argument from reading the DDL. (`schema_version` is a
  > hardcoded `"1"` that nothing reads back, so it could not have caught it
  > either.) The real cost was the JSON schema snapshot:
  > `schemas/JobEvent.json` grew 2 926 bytes and had to be regenerated.
- **The forward-compatible deserialiser half is still a live concern** — not for
  this variant, but for the next one. An older binary reading a newer database
  will meet `"candidate_activated"` and fall through to its `unknown event type`
  arm. That is the correct behaviour and is not what this entry is about.
- **The two store backends must agree**, which is the drift D-14's
  `JobEvent::seed_projection` already had to fix once. Whatever seeds this
  variant belongs next to that function, not in a backend.
- **It is a new row in the ledger for every capture.** Candidate identity is
  currently derivable from the `source_candidates` table joined on the job; a
  first-class event makes the ledger self-sufficient and that is the point,
  but it does mean the log grows.

### Why it stayed logged rather than fixed in `8b95884`

`AGENTS.md` — "One feature or fix per branch." The durable-format change above
is its own reviewable decision, and stacking it onto a defect fix would have
made neither legible. The refusal is what makes deferring it safe: **the
exposure is bounded to a tool that now announces, by name, that it cannot do
the job** rather than one that quietly does it wrong.

### The standing hazard, which is the real content of this entry

The CAS defect and this one were found together, in the same run, and both were
invisible to the suite for the same reason — D-14's, one layer out:

| Test | Why it could not see either |
|---|---|
| `rebuild_smoke.rs` | exercises the **no-events** path — the one path that always worked — and asserts the CLI prints a report |
| `projection_rebuild.rs` | calls `rebuild_projection` and asserts on the return; never writes back, so it never meets the CAS check |
| every pre-existing `rebuild_projection` test | hand-built its log (D-14) |

Not one of these is a bad test. Each answers a real question about a real
component. **No amount of testing inside the components finds a missing seam
between them**, and the §36 escape hatch is a seam between four.

The method that found both is the one 0B.10's C4 notes already generalised —
*drive the real command surface against a real store, and assert on what the
store says afterwards* — applied here to the **operator** surface rather than
the MCP one. `bins/ironmaintctl/tests/rebuild_regression.rs` is that test,
written afterwards so the pair cannot recur, and both of its halves were
verified to fail without their respective fix.

That makes three components in this repository — `rebuild_projection`, the
`§101` drivers, and now the CLI — where the missing thing was never logic. It
is the single most productive defect class in this codebase and it is not
findable by any tool currently in the workspace.

### CLOSED 2026-10-01 — three fields, not two, and a red commit as the receipt

`JobEvent::CandidateActivated(CandidateActivated)` landed, carrying four fields:

```text
CandidateActivated { candidate_id, fingerprint, version_after, updated_at }
```

`job_id` is **not** among them: the envelope already carries it, and duplicating
it would let the two disagree — the same "let the two disagree by construction"
shape the entry above is about. `Domain` was not widened, for the reason already
given.

**The third field.** The two amendments above name `active_candidate` and
`version`. S7's first run reported **three** unrecoverable fields: `active_candidate`,
`version`, and `updated_at` — the last one appearing for the first time in this
entry. `handle_set_active_candidate` stamps the row's `updated_at` from the same
`now` that the envelope carries, so a replay that took its timestamp from the
`JobCreated` seed's would produce a row that is correct in every other field and
wrong in its own audit clock. Had the variant shipped with only the two predicted
fields, `updated_at` would have stayed unjustified with nothing left to notice —
the same half-fix the `version` amendment warned about, one field over.

The second S7 test is what makes the report actionable rather than a bare failure:
it asserts the two fields activation does *not* touch (`state`, and the timestamp
of a never-activated job) round-trip cleanly, which is what isolates the
unrecoverable set to the fields activation writes. Without it the report would
have been "three of five fields differ" with no way to say whether the fault
lies in the event or in the replay.

**The refusal is kept, and its role changed.** With the variant in place, a job
written by the new binary is fully log-justified and rebuilds. A row written
before it is not, and cannot be made so — the event was never written and no
amount of replay invents it. The guard is therefore a **legacy-data guard**
rather than a live limitation, and its message says which of the two an operator
is looking at. Removing it would make the tool report `rebuilt: 1` on precisely
the rows whose logs cannot justify them, which is the data loss the refusal
exists to prevent; there is no flag to "force" past it because the force would
be the bug.

`bins/ironmaintctl/tests/rebuild_regression.rs` is split into the two halves that
distinction implies: a captured job rebuilds and keeps its active candidate, and
a log stripped of its `CandidateActivated` event is refused and left untouched.
The second builds its legacy database by **removing one event from a real log**
and replaying the rest — the opposite of hand-building a log, so every other
event survives and an implementation that inferred the activation from history
would be caught.

**The ordering, which was the point of the exercise.** D-16's fix and S7 landed
in that order on purpose. S7's *first run is the evidence* that the defect was
real:

```text
the replay cannot recover: active_candidate, version, updated_at
stored   : state=EventDetected active_candidate=Some(CandidateId(..)) version=1 updated_at=…669241
replayed : state=EventDetected active_candidate=None version=0 updated_at=…665628
```

Landing the fix first would have made S7 pass on its first run, having never been
seen to fail — which is D-14's exact lesson reproduced inside the tool built to
prevent D-14's class. The red commit is the receipt, and it is why the plan
sequenced wave 2 as two commits rather than one.


---

## Suggested ordering

Not a plan — the user decides. In rough order of value-per-effort. **Revised
2026-10-01** against the register as it now stands: four items are closed and one
was deliberately not done, so this list is shorter and its shape has changed.

1. **D-01** — four sites, two files, closes a HIGH with a false safety claim. The
   test-artifact/lint-inheritance gap it exposes argues for a new
   `xtask verify-lint-inheritance` guardrail so the class cannot recur. **Now the
   largest single item in the register**, and unrelated to everything 0B.10
   touched: `AGENTS.md`'s "one feature or fix per branch" is the reason it did
   not ride along.
2. **D-16** — a new durable event variant, so the log can justify the
   active-candidate binding and the §36 escape hatch works on real jobs rather
   than only on the ones that have never captured a candidate. **It is
   mitigated, not fixed**, and the mitigation is a refusal, which is safe but
   leaves §102 item 3 narrower than D-14's closure claimed. It changes the
   durable event format, so it wants its own branch and its own review.
3. **D-09** — the remaining half is a *producer* for `InfrastructureBlocked`, and
   the decision of "who may mark a job blocked, on what evidence" is a Phase 1
   question, not a 0B one. Write the decision down before writing the code.
4. **D-11** — Phase 1, gated on the `ReadyForApproval`-through-the-tools criterion above.
5. **D-06** and **D-15** — same root cause, same fix: an external binary. Nothing to
   do until one exists; the repro steps are in D-15.
8. **D-12**, **D-13** — as scheduled.

**Closed since the previous revision, and therefore gone from this list:** D-02
(C3 — `skills/` is canonical, the `integrations/` copy is deleted), D-03 (C1–C5),
D-03's wire split (C3, as its own commit because it changes the wire format),
D-07 (C4 — the semantic question was already answered by §41 and by
`Advanced`'s own doc comment; the loop walked only the first rule), and D-14
(0B.10 prerequisite).

**Deliberately not done, so not a debt:** D-10. See its entry — §41 defines
`reconcile` per-call and no spec text asks for a loop. Re-deferring it without a
reason would have been the fourth deferral of a thing nobody had asked for.

## Decisions taken 2026-09-30

Two calls the architect delegated, both now recorded above rather than left open:

| Question | Decision | Where it lands |
|---|---|---|
| Should Phase 1's first adapter be required to drive a job through the tool surface? | **Yes** — "reaches `ReadyForApproval` driven only through the MCP tools, no runtime handle, no direct store access" is the governing acceptance criterion | Phase 1 (§106) |
| Should `next_actions` keep advertising actions with no verb behind them? | **No** — `allowed` becomes strictly tool-performable, the human-action case moves to its own field, an enforcement test makes an unperformable `AllowedAction` a gate failure, and `Publish` is removed as never-emitted. Agent behaviour is unchanged. | Top of 0B.10 |

Both have since landed. The first turned out to be satisfiable one sub-phase
early: 0B.10 C5's `mcp_acceptance_scenario.rs` meets the criterion as written, so
it is a **regression test for Phase 1's premise** rather than a Phase 1
deliverable — if a Phase 1 adapter cannot be driven through the same surface, the
change that broke it fails a test that already exists.

---

## D-20 — seven of twelve commands are constructed by nobody, and five of the seven are fine (LOW)

**Severity rationale:** LOW. Nothing in the runtime is missing a capability, and
nothing computes a wrong answer. Both parts of this entry are a comment that
will mislead a future reader, which is real but is not a defect in behaviour.
It is filed at all because the two comments are *load-bearing*: each is the
justification a future implementer would rely on, and each justifies a design
that is not the design.

**How it was found.** S1 in `verify-seams`, on its first honest run. The check
counts `Expr::Struct` — a construction — rather than occurrences of
`VariantName {`, because a `match` arm and a struct literal are the same `syn`
node separated only by a `=>`. A text search finds all twelve arms of
`handle_command` and reports the seam as closed when it is wide open. Seven
variants have no construction site outside tests.

### Evidence

| Variant | Reached by | Verdict |
|---|---|---|
| `MaterializeChecks` | `handle_capture_candidate` → `handle_materialize_checks` (service.rs:1622) | decision, reason at the call site |
| `SetActiveCandidate` | `handle_capture_candidate` → `handle_set_active_candidate` (service.rs:1712) | decision; D-16's event makes the binding durable |
| `RecordCheckEvidence` | `handle_run_check` writes the gate result inline (service.rs:2417) | decision, and §53 is the reason |
| `RecordObligationOutcome` | `handle_run_check` → `derive_obligation_verdicts` (service.rs:2431) | decision, and the comment says why |
| `Reconcile` | MCP dispatcher calls `RuntimeService::reconcile` directly (mcp/src/dispatch.rs:214) | decision; §41 is per-call, no loop |
| `EnterHumanReview` | tests only | **deliberate, and said so twice** |
| `RequestApproval` | nothing | **the refusal is correct; its doc is wrong** |

The two that are not decisions are described below.

### Part 1 — `RequestApproval`'s doc describes a design that no longer exists

The variant's own doc (`crates/ironmaint-runtime/src/command.rs:125-132`) says:

> *"`next_actions` still advertises `RequestApproval` at that state, and that is
> not a contradiction … An agent that follows `next_actions`, calls this, and
> reads the refusal has learned it must hand off to a person — which is the exit
> checkpoint in `SKILL.md` made executable rather than implied."*

Both halves of that are false today. `RequestApproval` is not a variant of
`AllowedAction` at all — that enum has four (`CaptureCandidate`, `RunCheck`,
`ApplyPatch`, `ResumeJob`), and S2 confirms all four map to registered tools.
`ReadyForApproval` emits `allowed: vec![]` with
`requires_human: Some(HumanAction::ApproveRelease)` (service.rs:2706).

So the agent never calls it, and the exit checkpoint is exactly as *implied* as
the doc says it is not. **The refusal is not the problem.** 0B shipping no
approval principal, no delivery channel and no durable `ApprovalStore` is a
deliberate boundary, and recording a decision nobody made would be worse. What
is wrong is the doc: it claims a reachable path to the refusal, and a future
implementer reading it would add an MCP tool to satisfy a contract that the
handler already satisfies by refusing.

**Fix:** correct the doc to describe the two `next_actions` fields as they are —
`requires_human: ApproveRelease` is the advertised move, and the refusal is what
a caller gets if it constructs the command anyway.

### Part 2 — `job.resume` is a registered tool that cannot succeed

`EnterHumanReview` being unreachable is a decision, written down twice:
*"escalation is an orchestration decision, and an agent that could raise it
against itself could also strand itself"* (mcp/tests/dispatcher.rs:204) and
*"0A §21 gives it no entry point, so the runtime cannot yet produce this
projection"* (next_actions.rs, on the `HumanReviewRequired` arm).

What nobody wrote down is the consequence. The only `ResumeRecorded` writer in
the workspace is `handle_enter_human_review` (service.rs:1037). `attach_resume_action`
only advertises `AllowedAction::ResumeJob` when such a record exists. Therefore
in production no record is ever written, the action is never advertised, and
`job.resume` — registered, mapped, and dispatchable — cannot succeed against any
job the daemon can actually reach.

Two docs then describe the state as though it were reachable:
`HumanAction::ReviewEscalation` says *"The same job also lists
`AllowedAction::ResumeJob`, because a tool for it exists"*, and
`AllowedAction::ResumeJob` says it is emitted when a record exists. Both are
true of the code and unreachable in practice.

**This is not a bug to fix.** The escalation entry point belongs to whichever
phase supplies an operator surface, and the consequence is exactly what that
phase needs to know. It is recorded so that the eventual implementer of the
entry point sees the other half of the contract, and so that a future audit
does not read `job.resume` being registered as evidence that the review loop
works.

**Why LOW and not higher.** S8 — "reachable only from a test" — is the check
that would treat part 2 as a defect, and S8 is deferred to the behavioural PR.
What S1 could report here is the weaker, truer statement: a variant nobody
constructs. That statement is accurate and is worth a register entry, but on its
own it does not distinguish the five decisions from the two comment defects.
Distinguishing them required reading every call site, which is what the
allowlist reasons are for.

## Decisions taken 2026-10-01

Four more, taken while closing out 0B.10. Each is recorded at the point it applies
rather than only here.

| Question | Decision | Where it lands |
|---|---|---|
| Is `reconcile` one transition per call or many? | **Many**, until it blocks. §41 and `ReconcileOutcome::Advanced`'s own doc comment both said so; the code contradicted its own type documentation. | C4, `cec87d7` — D-07 |
| Should a failed *mandatory* obligation divert the job to `HumanReviewRequired`? | **No — entering is explicit.** 0A §21 says orchestration "*may*" move a job there; automatic diversion would make §101's repair loop unreachable for the agent that is supposed to perform it. `EnterHumanReview` exists as a command and has **no** MCP tool, so an agent cannot hand itself into a state only a human should enter. | C2 |
| Where should the §101 scenario's MCP driver live? | **`ironmaint-testkit/tests/`, not `crates/ironmaint-mcp`.** §98.6 forbids the MCP crate from depending on a store backend in *either* dependency kind, and "MCP must never bypass runtime" is a rule about the crate, not about whether the caller is a test. | C5 |
| Do the privileged-operation producers belong in 0B? | **No — dropped, not deferred.** §4.10, §26 and §99 forbid them, so building them would produce code the acceptance scenario must never exercise. | 0B.10 scope |
