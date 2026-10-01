# `xtask verify-seams` — plan and specification

**Status:** plan, not implementation. Written 2026-10-01, alongside
`doc/CONTAINER-IMAGES.md`. Read that document's §1 first: it explains why a
container cannot find the defect class this targets, and this is the instrument
that can.

**Purpose:** to catch, mechanically and before review, the defect class that
produced **fourteen** defects during Phase 0B.10 — *a component modelled
something correctly, and no production path produced the input.*

---

## 1. The class, stated precisely

Every one of the fourteen had the same shape. Two components were each
individually correct and individually tested. The **link between them** was
absent, wrong, or unreachable, and no test in the workspace could see the
disagreement because each test sat entirely on one side of it.

That is not a testing gap. Adding tests inside the components does not close
it — it is the shape that survived every test that was written. The gap is
between the *inventory* of what a component declares and the *inventory* of what
the rest of the tree actually calls, and those two inventories are maintained by
hand, in different files, by different commits.

So the instrument is not "run more tests". It is **four inventories, compared
against each other**, plus **one behavioural round-trip** that no unit test can
stand in for.

### 1.1 What this can and cannot find — measured, not estimated

This is the most important section in the document, and it is the honest answer
to "will this catch the issues you found."

| # | Defect | Caught by |
|---|---|---|
| 1 | `SetActiveCandidate` had no production caller | **S1** |
| 2 | `MaterializeChecks` had no production caller | **S1** |
| 3 | `PolicyPlan` obligations were never persisted | **S1** |
| 4 | nothing could write an obligation outcome other than `Pass` | **S1** |
| 5 | `reconcile` returned on its first pass (`never_loop`) | **nothing here** |
| 6 | `SetActiveCandidate` refused outside four pre-build states | **nothing here** |
| 7 | `rebuild_projection` required a `Transitioned` first | **S7** |
| 8 | rule 12 required an input no writer could produce | **S1** |
| 9 | re-capture minted a duplicate candidate (SQLite threw, mock overwrote) | **S9** |
| 10 | `attach_obligation_state` read the fingerprint before checking one existed | **nothing here** |
| 11 | re-capturing an unchanged tree minted a second candidate | **S9** |
| 12 | `next_actions` omitted an outstanding obligation | **nothing here** |
| 13 | `replay_one` took its CAS token from the wrong projection | **S7** |
| 14 | replay dropped `active_candidate` | **S7** |

**Eight of fourteen. Six are out of reach, permanently, and no amount of static
analysis changes that.**

The six are *wrong-guard* defects: the link exists, has a caller, and does the
wrong thing. `reconcile` had a caller and a loop; the loop was inside a
`#[allow(clippy::never_loop)]`. `next_actions` had a caller and returned an
empty list. A verifier that checks "is this called" sees a called function and
has nothing to say.

**Those six were all found by the §101 drivers** — `acceptance_scenario.rs` and
`mcp_acceptance_scenario.rs` — and by the live daemon run. That is not a
coincidence and it is not luck: a driver that walks the workflow end to end is
the only thing that can execute a guard's condition and observe the refusal.
`verify-seams` is a complement to those drivers, not a replacement, and shipping
it as a replacement would be the same mistake as shipping a container instead.

### 1.2 Why not a runtime tracer

The alternative design — instrument the daemon, record which commands and store
methods are actually reached during the test suite, and diff that against the
declared surface — is strictly more powerful, because it covers wrong-guard
defects too.

It is not proposed, and the reason is worth recording: coverage of *reached*
code is not coverage of *reachable* code, and the interesting direction is the
latter. A trait method reached by no test is a strong smell; a guard that always
refuses is reached constantly and never passes. A tracer would report
`handle_set_active_candidate` as "called, 400 times" while the defect lived in a
condition inside it. It also requires the daemon to be running, which puts a
durable service in the §97 gate.

Static inventory comparison is cheap, deterministic, needs no service, and
catches the subclass that is mechanically detectable. It should ship first.
**If it lands and the wrong-guard class keeps recurring**, the tracer is the
next step and should be written down then, with evidence that it was needed.

---

## 2. The checks

Nine checks. Six are static (`xtask`), three are behavioural (tests). Each is
stated as *the assertion*, not as a goal, because a check that cannot fail is
the exact failure mode D-14 recorded.

Static checks need the workspace's source but not a compile. `xtask` already
parses `Cargo.toml` via `toml_edit`; these parse `.rs` files. Add `syn` (with
the `visit` feature) and `proc-macro2` to `xtask`'s dependencies. That is the
only new dependency.

### S1 — every declared command has a production caller

**Assertion.** Every variant of `RuntimeCommand` is *constructed* at least once
outside `#[cfg(test)]` modules and outside `tests/` directories.

**Why construction, not mention.** A variant can be named in a match arm, a doc
comment, a test, and a schema, and be reachable by nobody. Only a construction
expression makes it callable. This distinction is what makes the check work:
`RequestApproval` is *matched* everywhere and *constructed* in one place that
returns `Unsupported`.

**Current state, to be confirmed by the first run.** Twelve variants. The
expected result is that three are constructed only in ways that are correct
refusals — `RequestApproval` (typed `Unsupported`, forbidden by §4.10/§26/§99),
`Reconcile` (a pass-through the `job.reconcile` tool already drives), and
`RunCheck` (permanently `Unsupported`; the real path is
`RecordCheckEvidence`).

**The allowlist is the design.** A check that fails on a correct refusal trains
people to add `#[allow]` reflexively, and D-14's lesson generalises: a
suppression is a decision recorded in the wrong place. So the allowlist is a
`const` in `xtask/src/seams.rs` of `(variant, reason)`, **each reason naming the
spec section or the design fact that makes the absence correct.** Adding an
entry is a reviewable act with a reason attached; blanking the check is not.

### S2 — every allowed action names a registered tool

**Assertion.** Every name returned by `tool_for_action` appears in the daemon's
registered tool set.

**Partly redundant, and that is fine.** Totality of `tool_for_action` is already
a *compile-time* property — the match has no wildcard and returns `&'static str`
— so C3 closed the mapping half. What is not covered is the other half: that the
named tool is actually **registered**. A typo in a string literal type-checks
perfectly. This check closes that.

### S3 — every planned capability resolves to a tool, or is on a list

**Assertion.** Every `ToolCapabilityKey` any `DistributionAdapter` can plan has a
registered tool, or appears in an allowlist whose reason names the phase that
will supply it.

**This is a seam between the adapter layer and the executor layer**, and it is
currently open in a way the daemon already reports honestly: a captured `debian`
job materialises four gates whose keys (`debian.build.sbuild`, `debian.qa.lintian`,
`debian.qa.piuparts`, `debian.test.autopkgtest`) have no registered tool, and
`check.run` returns `no tool registered for capability debian.build.sbuild`.

That is *correct* behaviour in 0B — the adapters are stubs and only `synthetic.*`
tools exist. But it is a fact that currently lives in a runtime error message,
which means it is discovered by running a job rather than by building. The
allowlist converts it into a build-time statement, and `doc/CONTAINER-IMAGES.md`
§4.3's image work is what will empty the list.

**Scope note.** The check needs the adapter's plan, which is a runtime value, so
this is not pure source analysis. Two options: a test that instantiates both
stubs against the `ScenarioAdapter`'s harness, or a generated table. **Recommend
the test** — it runs the real `build_plan`, so a new stub is covered the day it
lands, and it belongs in `ironmaint-testkit` where the conformance suite already
lives. It is listed as behavioural (S8) rather than static for that reason.

### S4 — every event variant is handled in every replay implementation

**Assertion.** For each variant of `JobEvent`, the body of `ProjectionApply::apply`
has an explicit arm, and each `rebuild_projection` implementation handles it.

**Why this exists.** `apply` is an exhaustive `match` today, so adding a variant
is a compile error — good. But the *seeds* are the hazard: D-14's fix had to
move seed selection out of two backends that each held their own copy of the
decision, and only one was ever exercised. Two copies of one decision is a
structural defect waiting for the copy that drifts.

So the check is not only "every variant is matched" (already enforced) but
"**seed selection is decided in exactly one place**" — a source assertion that
`rebuild_projection` delegates rather than re-deriving. It is a small check with
a large precedent.

### S5 — both store backends implement the same method set

**Assertion.** Every method on every `IronMaintStore` supertrait is implemented by
both `MockStore` and `SqliteStore`.

**Today the two are not the same shape**, and that is itself a finding: `MockStore`
has a synchronous `find_gate_result` helper and the mock's `put_source_candidate`
*overwrites* while SQLite's raises `UNIQUE`. A trait method implemented with
different semantics in each backend is a seam by definition, and instance 9/11
exploited exactly that.

Parity of *signatures* is what a static check can do. Parity of *behaviour* is
S9, and it is the one that found the eleventh instance.

### S6 — no third copy of a shared decision

**Assertion.** No decision that appears in more than one backend is written
inline in more than one backend.

**This is a lint, not an inventory.** It is the generalised form of D-14 and it
is genuinely hard to do well. **Recommendation: do not build the general version
in the first pass.** Build the one concrete instance (S4's seed-selection rule)
and, if a second instance is ever found, generalise then. A half-working
similarity lint produces false positives that people learn to ignore, which is
worse than no lint.

### S7 — projection replay round-trip *(behavioural)*

**Assertion.** For every field of `JobProjection`, a job whose field was changed
by a **production** path replays to that value.

`JobProjection` has five fields: `job`, `state`, `active_candidate`, `version`,
`updated_at`. The check is a single test that drives real commands and compares
the stored row against `rebuild_projection`.

> **STATUS: run, red, and green.** S7 was written before D-16's fix and
> committed **failing** (`84af759`); the fix is `test/seam-s7-replay-roundtrip`'s
> second commit. The first run's output is in `doc/DEBT.md` D-16's closing
> section. The paragraph below predicted two unrecoverable fields; it was
> **three** — `updated_at` joined `active_candidate` and `version`, for the
> reason the last paragraph of this section anticipated without naming. The
> prediction being two rather than three is the point: the check is what
> enumerates, not a reviewer's reading of the code.

**Against the code as it stood, this check failed, and not hypothetically.**
The mechanism, verified in `handle_set_active_candidate`
(`crates/ironmaint-runtime/src/service.rs`): it sets `active_candidate`, writes
`version: current.version + 1` and stamps `updated_at` into the **projection
row**, then appends `JobEvent::Domain(domain_event_id)` — a bare identifier, no
payload. `ProjectionApply::apply` treats `Domain` as `self.clone()`, a no-op. So
the log records *that* something happened and the row records *what*, and the two
disagree by construction on every activation.

Only a `Transitioned` event would carry any of them forward, because
`Transitioned`'s payload embeds a full `projection_after` snapshot — and
activation does not transition. A job whose last event is a `Domain` therefore
replays to the last *transition's* row, which is the thirteenth and fourteenth
defects, and is why `replay_one` refused rather than clobbered.

`updated_at` was the open question the check was written to answer, and it
answered it the other way: `EventEnvelope.occurred_at` exists and `apply` takes
a timestamp, but activation's timestamp was being passed to the row write and not
to anything durable, so a replay produced a projection that was right in every
field except its own audit clock. A variant carrying only the two predicted
fields would have left it unjustified with nothing left to notice.

**Why the test lives in the testkit, not in the store crate.** The check was
specified here as landing in `ironmaint-store-sqlite/tests/`, and it cannot:
driving a job through `CreateJob` / `CaptureCandidate` needs `RuntimeService`,
and `ironmaint-store-sqlite` is a dependency *of* the runtime, so the test could
not name the type whose event log it is auditing. It lives at
`crates/ironmaint-testkit/tests/replay_roundtrip.rs` instead — a location §98
already blesses, and the same one §101's two drivers use for the same reason.
The store crate is the *subject* of the check, not its author.

### S8 — every command is reachable through the public surface *(behavioural)*

**Assertion.** Every `RuntimeCommand` the check S1 clears is constructible
through at least one of: a registered MCP tool, the daemon's startup/CLI path, or
another command's body — and that path is exercised by at least one test.

This is the *positive* half of S1, and it is where S3 belongs (see above). The
distinction matters: S1 catches "nothing calls this", S8 catches "something calls
this only from a test", which is a weaker but real version of the same defect —
D-14's `rebuild_projection` tests all did exactly that.

### S9 — mock and SQLite behave identically *(behavioural)*

**Assertion.** A shared conformance script, run against both backends, produces
identical observable results — including the error cases.

The existing conformance suite (`ironmaint-testkit/src/conformance.rs`) is
adapter conformance, not store conformance. Extending it to the store is a
different and smaller job, and it is the check that found the eleventh instance:
every pre-existing capture test used the mock, so a `UNIQUE` violation that fires
on the backend the daemon actually runs was invisible to all of them.

The error cases are the load-bearing half. A mock that returns `Ok` where SQLite
returns `Err` is not a simplification, it is a hole with a passing test over it.

---

## 3. Where each check runs

| Check | Kind | Home | In §97? |
|---|---|---|---|
| S1 command inventory | static | `xtask/src/seams.rs` | **yes** |
| S2 action → tool | static | `xtask/src/seams.rs` | **yes** |
| S3 capability inventory | behavioural | `ironmaint-testkit` | **yes** |
| S4 seed selection | static | `xtask/src/seams.rs` | **yes** |
| S5 store method parity | static | `xtask/src/seams.rs` | **yes** |
| S6 no third copy | **deferred** | — | — |
| S7 replay round-trip | behavioural | `ironmaint-testkit/tests/replay_roundtrip.rs` | **yes** |
| S8 production reachability | behavioural | `ironmaint-testkit/tests/` | **yes** |
| S9 store parity | behavioural | `ironmaint-testkit` | **yes** |

`cargo run -p xtask -- verify-seams` is the entry point, matching the existing
four verifiers' shape: in-process, no clap, `Result<_, Box<dyn Error>>`.

**S7 must not be green on its first run.** If it is, the check is wrong, not the
code. Write the test, watch it fail against the mechanism in §2/S7, and record
both the failure and the fix. A verifier that has never failed is D-14's lesson
learned too late.

S7 is the one check on this list that is **already written** and does not wait
for `verify-seams`: it is an ordinary integration test, so `cargo test
--workspace` runs it, and D-16's closure depends on it. It was built first for
the reason the paragraph above gives — its failure is the receipt — and
`verify-seams` inherits it rather than reimplementing it.

---

## 4. Definition of done

- [ ] `cargo run -p xtask -- verify-seams` exists and is listed in `CLAUDE.md`'s
      §97 acceptance set
- [ ] Every allowlist entry carries a reason naming a spec section or a design
      fact — checked at review, and **counted**: an allowlist that has grown
      without a reason is the failure mode
- [x] S7's first run failed, and both the failure and its cause are written down
- [ ] S6 is either built or explicitly deferred **in this document**, not left
      as a heading with no content
- [ ] A deliberately introduced missing caller (S1) and a deliberately
      duplicated candidate (S9) each make the verifier fail, and both are
      reverted afterwards. **A verifier that has never been seen to fail is
      assumed broken until this is done**
- [ ] `doc/DEBT.md` gains an entry recording what each check *cannot* find, so
      the six wrong-guard defects in §1.1 are not re-attributed to this work

---

## 5. Non-goals

- **Replacing the §101 drivers.** They caught all six of the wrong-guard defects
  this cannot see. `verify-seams` complements them.
- **A coverage tracer.** See §1.2.
- **Proving absence of defects.** Nine checks narrow a class. They do not close
  it, and a green `verify-seams` must never be read as "no missing seams".
- **S6 in the first pass.** One concrete instance, generalised only if a second
  one is actually found.
