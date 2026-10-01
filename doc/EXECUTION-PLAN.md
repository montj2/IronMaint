# Execution plan — everything open after 0B.10

**Written** 2026-10-01, immediately after PR #27 (0B.10) and PR #28 (the two
specs) merged into `develop`.

**Purpose:** order the work. `doc/DEBT.md` holds the *inventory* of what is open,
`doc/CONTAINER-IMAGES.md` and `doc/SEAM-VERIFICATION.md` hold the *designs* for
the two new bodies of work. None of the three says what goes first, and the order
is not a preference — three of the seven waves are blocked on an earlier one, and
one of them is in the wrong order if you take the designs at face value.

---

## 0. The one ordering correction worth reading

**The D-16 fix and the `verify-seams` S7 check are specified in the wrong
relative order, and the plan has to invert them.**

`doc/SEAM-VERIFICATION.md` §2/S7 says its "first run must fail, or the check is
wrong." That is only achievable if S7 is written **before** D-16 is fixed. If
D-16 lands first, S7 passes on its first run and nobody has ever watched it fail
— which is the precise failure D-14 recorded ("a test that cannot fail is not a
test") being reproduced in the tool built to prevent D-14's class.

So wave 2 is two commits, in this order:

1. **S7, failing.** Assert that a job whose `active_candidate` and `version` were
   set by a production path replays to those values. It will fail on exactly
   those two `JobProjection` fields. Commit it red, with the failure in the
   message.
2. **D-16's fix.** The new event variant. S7 goes green.

A red commit in the history is the evidence the check has teeth. It is worth more
than a green one, and it costs nothing.

> **Both done, 2026-10-01** (`84af759` red, then the fix on
> `test/seam-s7-replay-roundtrip`). Two things the ordering got right and one it
> got wrong, all three worth carrying to wave 4:
>
> - **The inversion held.** S7's first run failed, which is the only part of this
>   plan that could not be recovered after the fact.
> - **"Exactly those two fields" was wrong — it was three.** `updated_at` was
>   also unrecoverable, for the reason §2/S7 of the seam spec had flagged as an
>   open question and not resolved. A variant carrying the two predicted fields
>   would have shipped a half-fix with nothing left to notice. This is the single
>   strongest argument in the plan for S1–S5 and S9: a reviewer's reading of the
>   code enumerated two of the three, and a *test* enumerated three.
> - **D-16's cost estimate was wrong in the cheap direction.** The entry said the
>   format change would need a migration on every existing database. It does not:
>   `events.event_type` is unconstrained `TEXT`. `verify-migrations` confirmed it
>   by staying clean. The only snapshot that moved was `schemas/JobEvent.json`.
>
> Wave 2 was scoped as medium-risk and came in as the cheapest of waves 1–4, and
> the reason is worth stating before wave 4 is scoped: **the expensive part was
> writing the test, not fixing the defect.** The defect was three fields on one
> function. Wave 4 is nine checks of the same shape, and should be scoped on that
> basis rather than on how small D-16's diff looked.

---

## 1. The waves

Seven waves. Each is one branch, one PR, one reviewable claim.

| # | Wave | Items | Blocked by | Risk | State |
|---|---|---|---|---|---|
| 1 | Lint the persistence layer | **D-01** | — | low | **done** 2026-10-01 |
| 2 | Prove the log gap, then close it | **S7 → D-16** | — | **medium** | **done** 2026-10-01 |
| 3 | The dead-code cluster | **D-04, D-05, D-08** | — | low | next |
| 4 | Seam verification | **S1–S5, S8, S9** | 1, 2 | medium | unblocked |
| 5 | Container images | new | — | **long** | unblocked |
| 6 | The extension manifest | **D-06** | 5 | medium | — |
| 7 | §102 item 30 | **D-15** | 5, 6 | low once unblocked | — |

Plus one item that is **not** code and should be written now, in parallel:

> **D-09's decision** — who may mark a job `InfrastructureBlocked`, and on what
> evidence. The exit machinery is complete and correct; nothing writes the
> transition. That is a Phase 1 design question, and writing it down is cheap and
> unblocks the Phase 1 spec. It is the one item on this page with no code at all.

### Why this order

**Wave 1 first** because the project's central safety claim — no `unwrap`, no
`expect`, no `panic`, no `unsafe` in production paths — is **unenforced in exactly
the two crates that own the database**, and wave 2 writes a serialisation arm and
a migration into both of them. Enabling the lints first means that code is written
under the constraint rather than retrofitted into it. It is also the register's
only remaining HIGH, and it is small: two `[lints]` lines and roughly a dozen
sites.

**Wave 2 before wave 4** because S7 is a check *about* D-16, and a check written
after its subject is a check that has never been seen to fail. See §0.

**Wave 3 as one PR, not three.** D-04 is a promise the config made
("promoted once 0A.6 lands"), D-05 is the vestigial `#[allow(dead_code)]` sites
that make that promise dishonest, and D-08 is the one place with genuinely unread
fields that promotion would catch. Promoting `dead_code` while the vestigial
allows still exist is D-14 again: a suppression recorded in place of a decision.
They land together or the promotion is theatre.

**Wave 4 after 1 and 2** so the new `xtask` code is itself lint-constrained, and
so S7 already exists as a model for the behavioural checks.

**Wave 5 is independent and long**, which is exactly why it should start first in
wall-clock terms even though it is fifth in dependency order. Nothing else waits
on it except 6 and 7, and its largest risk — whether `mock` supports a Fedora 42
aarch64 buildroot — is answerable in five minutes on day one. **If one thing
starts today in parallel, it is this.**

**Waves 6 and 7 are downstream of 5, not of 1–4.** D-06 is not "never
round-tripped"; it is structurally wrong, and closing it needs the `ironclaw`
binary. That binary needs Rust 1.98 against this workspace's 1.88 MSRV, which is
the whole reason wave 5 exists.

---

## 2. What is deliberately not scheduled

| ID | Why not |
|---|---|
| D-10 | **NOT TAKEN**, reason recorded — §41 defines `reconcile` as per-call, and no spec text requires a loop |
| D-11 | Deferred to Phase 1 (§106) |
| D-12 | Deferred |
| D-13 | Process, not code |
| S6 | Deferred **with the reason recorded**, per `doc/SEAM-VERIFICATION.md` §4 — one concrete instance of D-14's structural rule exists (S4); generalise only if a second is found |
| Wrong-guard defects | The six of the fourteen that static verification provably cannot find. Caught by the §101 drivers, which stay. |

---

## 3. Definition of done for this plan

- [ ] Every open register row is CLOSED, or carries a recorded reason it is not
      closing, with a *changed* reason rather than a repeated "deferred to 0B.10"
- [ ] `cargo run -p xtask -- verify-seams` is in the §97 set in `CLAUDE.md`
- [x] S7's failing commit is in the history, before D-16's fix — `84af759`
- [ ] `verify-seams` has been seen to fail on a deliberately introduced defect,
      twice (S1 and S9), per `doc/SEAM-VERIFICATION.md` §4
- [ ] §102 is 34 of 34, or the remaining gap is D-15 and D-15 says why

---

## 4. The standing hazard

`doc/DEBT.md` opens with a note added after D-14 was closed and a defect was
found directly behind it. Three of the seven waves above add new checks and new
gates, which is precisely when that failure mode is most likely.

**Every check in this plan gets broken on purpose before it is trusted.** S7
before D-16. S1 and S9 each with a deliberately introduced defect. D-01 by adding
a `unwrap` and watching clippy catch it. A gate that has never rejected anything
is an assumption wearing a green tick, and the register has now been wrong about
one of those twice.
