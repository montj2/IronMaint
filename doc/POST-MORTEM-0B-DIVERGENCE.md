# Post-mortem: 0B.3 + 0B.4 vs develop divergence

**Date:** 2026-10-13
**Triggered by:** PR #65 (`release/phase-1` → `master`) failing
with 30 add/add conflicts at the merge boundary.
**Author of the trigger:** Phase 1 release-promotion PR opened
in good faith by the implementing agent.
**Diagnosis:** the 0B.3 and 0B.4 work landed on `master` via
release branches, but never landed on `develop`. Phase 1 was
built on `develop` against the 0B.2 base, with parallel
implementations of the same executor / runtime / workspace /
state / store primitives that 0B.3 and 0B.4 also implemented on
`master`. The 30 add/add conflicts are the signature.

This post-mortem is the architect's review input. **Read this
before deciding what to do next.** Closing the release PR is the
right immediate action; the path forward is a sync, not a
conflict resolution.

## The diagnosis, in SHAs and PRs

- `master` tip: `d5658ed` — Phase 0B.4 (PR #23).
- `develop` tip: `2729264` — Phase 1 close (post-PR #64).
- Merge base: `b2eacbc` — Phase 0B.2 (PR #18).

Master's history on top of the merge base (3 commits):

| SHA | Title |
|---|---|
| `d5658ed` | Phase 0B.4: Executor + ToolRegistry + per-job artifact caps + truncated=true (#23) |
| `2abb519` | Develop (#22) — the 0B.4 release branch back-merge |
| `3799e70` | Phase 0B.3: Workspace and Candidate Runtime (#91) (#19) (#20) |

Develop's history on top of the merge base (100 commits): the
entire Phase 1 sequence — 16 PRs (#46 through #63, with 1C.2
reopened as #54/#55) plus the Phase 1 completion doc (#64).

**The two branches share 0B.2 as a base and have done entirely
different work since.** Master built 0B.3 (workspace + candidate
runtime) and 0B.4 (executor + ToolRegistry + per-job artifact
caps) — the production versions of the primitives Phase 1
expected to use. Develop built Phase 1 against the 0B.2
primitives, which were incomplete: 0B.2's executor was a
skeleton, 0B.2's runtime did not have the candidate capture
handler that 0B.3 introduced, 0B.2's state machine had no
`SourceCandidate` integration, 0B.2's store did not have the
`set_active_candidate` op.

Phase 1 added all of those. **Twice.** Once in the form Phase
1 needed, and once in the form 0B.3/0B.4 already implemented
on master.

## The 30 conflicts, in files

Hand-attempted merge of `release/phase-1` into `master` at
`d5658ed` (with `2729264` as the head). All 30 conflicts are
**add/add** or **content** conflicts; none are typo-level:

- `bins/ironmaint-fixture/src/lib.rs` (add/add)
- `bins/ironmaint-fixture/src/main.rs` (content)
- `crates/ironmaint-executor/src/artifact_guard.rs` (add/add)
- `crates/ironmaint-executor/src/fixture.rs` (content)
- `crates/ironmaint-executor/src/lib.rs` (content)
- `crates/ironmaint-executor/src/process.rs` (add/add)
- `crates/ironmaint-executor/src/record.rs` (content)
- `crates/ironmaint-executor/src/record_builder.rs` (add/add)
- `crates/ironmaint-executor/src/registry.rs` (content)
- `crates/ironmaint-executor/tests/process_exit_codes.rs` (add/add)
- `crates/ironmaint-executor/tests/registry.rs` (content)
- `crates/ironmaint-runtime/src/service.rs` (content)
- `crates/ironmaint-runtime/tests/evidence_obligation_check.rs` (content)
- `crates/ironmaint-runtime/tests/handle_run_check.rs` (add/add)
- `crates/ironmaint-state/src/apply.rs` (content)
- `crates/ironmaint-state/src/event.rs` (content)
- `crates/ironmaint-state/src/lib.rs` (content)
- `crates/ironmaint-store-sqlite/src/lib.rs` (content)
- `crates/ironmaint-store-sqlite/src/ops/events.rs` (content)
- `crates/ironmaint-store-sqlite/src/ops/projections.rs` (content)
- `crates/ironmaint-store-sqlite/tests/migrations.rs` (add/add)
- `crates/ironmaint-store-sqlite/tests/round_trip.rs` (content)
- `crates/ironmaint-store/src/mock.rs` (content)
- `crates/ironmaint-store/src/workspace.rs` (content)
- `crates/ironmaint-testkit/Cargo.toml` (content)
- `crates/ironmaint-testkit/src/executor_conformance.rs` (add/add)
- `crates/ironmaint-testkit/src/lib.rs` (content)
- `crates/ironmaint-workspace/src/capture.rs` (content)
- `crates/ironmaint-workspace/tests/capture_candidate.rs` (content)
- `schemas/JobEvent.json` (content)

`Cargo.lock` and several other files auto-merged. The
auto-merge was the easy part; the 30 conflicts are the
production-primitive files where 0B.3/0B.4 and Phase 1 each
implemented their own version.

## Why this is a process break, not a Phase 1 bug

`AGENTS.md` is explicit:

> `master` | Production-ready, tagged releases
> `develop` | Integration branch, deployable
> Rules:
>   - Never commit directly to `master` or `develop`.
>   - **One feature or fix per branch.**
>   - **Rebase feature branches on `develop` before opening a PR.**

And the Git Flow release flow:

> `release/<version>` | Release stabilization, no new features |
> from `develop` | `master` + `develop`

The 0B.3 and 0B.4 work landed on `master` via release
branches. The release branches were cut from `master`, not
from `develop`. The 0B.3 PRs (#19, #20, #91) and 0B.4 PR
(#23) were merged directly to `master` without a develop
back-merge. The result: master got ahead of develop, and
Phase 1 (which started on develop) was building on a stale
base.

**The architect's discipline gap:** the AGENTS.md
"`release/<version>`" row says "from `develop`." Both 0B.3
and 0B.4 release branches were cut from `master`, not from
`develop`. The discipline that would have caught this is
"every release branch is cut from develop, every release
branch is merged back to develop as a back-merge commit, no
commit reaches master without first reaching develop." That
discipline was not enforced for 0B.3 or 0B.4.

**The Phase 1 agent's discipline gap:** the Phase 1 plan
made no check for "is develop actually current with master?"
before starting work. The plan's "preconditions" (per the
prior planning session) was "Phase 0 is closed and the gate
is green on develop." But "the gate is green on develop"
tests develop's state, not develop's relationship to
master. A pre-flight check that would have caught this:
`git fetch origin && git log --oneline origin/master ^origin/develop`
should have been empty (or at most a release branch back-merge
with `master` in the body). Instead it returned 3 commits —
the entire 0B.3 and 0B.4 work.

Both gaps mattered. Neither one is bigger than the other.
A single one would not have produced 30 conflicts.

## Why this is a 30-conflict problem, not a 30-line problem

Each of the 30 conflicts is two complete file rewrites:

- **Master's version** — the production 0B.3/0B.4
  implementation. Has tests, has call sites in 0B.3/0B.4
  downstream code, has schema snapshots, has a working
  executor with the new ToolRegistry and per-job artifact
  caps and `truncated=true` semantics. This is the version
  `master` carries.
- **Develop's version** — the Phase 1 implementation that
  replaced the 0B.2 skeletons with Phase 1's own versions
  of the same primitives. Has tests, has call sites in
  Phase 1 downstream code (1A.2's `build_candidate_input`,
  1A.3's `handle_run_check` normalizer branch, 1A.4's
  evidence MCP tools, 1B.1's `plan_checks` extension, 1B.3's
  `register_synthetic_tools`-style wiring, 1C.x's
  `ToolDefinitionRecord` registrations, 1E.x's BTS
  shell-out, 1F.1's `reconcile` walk, 1G.1's
  `release.candidate.create` projection), has schema
  snapshots, has the production Debian adapter. This is
  the version `develop` carries.

Hand-resolving "take master's file" would discard the
Phase 1 work; hand-resolving "take develop's file" would
discard the 0B.3/0B.4 work. Neither is a 30-line
resolution; both are a multi-week rewrite of one side
or the other.

## What is recoverable

1. **The Phase 1 design.** The 16 PRs of Phase 1 are
   correct against the 0B.2 base that was used. The §41
   DoD claims are valid. The five pre-build gates walking
   `EventDetected → SourceIntegrity` through MCP is
   verified. The `doc/PHASE-1-COMPLETION.md` (PR #64) is
   the right review input. None of this changes.

2. **The seam check S6.** `xtask/src/seams/adapters.rs`
   is on develop and on master is unchanged. The S6
   deny list encodes Phase 1's intent and survives the
   rebase; it's a standalone xtask-side file that does
   not depend on the conflicting executor / runtime
   / store / state code.

3. **The MCP tool surface (11 → 14 tools).** The 1A.4
   tools are registered in the daemon. After the rebase,
   they need to be re-registered against the 0B.4
   executor's `ToolRegistry`, but the *tool definitions*
   and the *MCP dispatch* are independent of the
   executor internals.

4. **The 5 pre-build check binaries.** `bins/ironmaint-debian-tool/`
   is on develop, uses `deb822-lossless` + `debversion`,
   reads JSON on stdin, emits `DebianSourcePreparationV1`
   / `DebianSourceReportV1` / `DebianMaintenanceContextV1`
   / `DebianPolicyDeltaReviewV1` / `DebianIssueReportV1`.
   These are independent of the conflicting executor
   primitives; they are file-system reads + JSON emit.
   They will plug into the 0B.4 executor's `ToolRegistry`
   unchanged.

5. **The hermetic Debian fixture.** `containers/fixtures/debian/`
   is on develop and on master is unchanged. The §29
   hermetic-fixture mandate is satisfied.

6. **The Debian authority manifest.** `policy/debian/authorities.v1.json`
   is on develop. The `debversion` comparator is on
   develop (1B.2's `Cargo.lock` entry). Both survive
   the rebase.

7. **The completion doc.** `doc/PHASE-1-COMPLETION.md`
   is on develop (PR #64). It is the right review
   input once Phase 1 is re-rebased.

## What is not recoverable

1. **Phase 1's parallel implementations of 0B.3/0B.4**
   **primitives.** The Phase 1 versions of
   `record_builder.rs`, `service.rs`, `process.rs`,
   `artifact_guard.rs`, `mock.rs`, `workspace.rs`,
   `fixture.rs`, `lib.rs` (×5), `registry.rs`,
   `executor_conformance.rs`, `migrations.rs`,
   `round_trip.rs`, `evidence_obligation_check.rs`,
   `handle_run_check.rs`, `process_exit_codes.rs`,
   `apply.rs`, `event.rs`, `state/lib.rs`,
   `store_sqlite/lib.rs`, `store_sqlite/ops/events.rs`,
   `store_sqlite/ops/projections.rs`, `capture.rs`,
   `capture_candidate.rs`, `JobEvent.json`, and the
   `bins/ironmaint-fixture/` source must be
   *reconciled* against the 0B.3/0B.4 versions during
   the rebase, not kept as-is.

2. **The 1A.1 D-21 conformance pin.** The
   `handle_capture_candidate` pre-check was added in 1A.1
   against the 0B.2 store. The 0B.3 store has its own
   `set_active_candidate` op, and the D-21 conformance
   pin at `crates/ironmaint-testkit/tests/store_parity_sqlite.rs:83-110`
   was written against the 0B.2 store. After the rebase,
   the conformance pin must be re-derived against the
   0B.3 store; the pre-check pattern may or may not be
   the right shape.

3. **The 1F.1 vertical-slice test.** The 5-step
   `EventDetected → SourceIntegrity` walk in
   `bins/ironmaintd/tests/startup_shutdown.rs` was
   written against the 0B.2 state machine + 0B.2
   transitions. The 0B.3 work added workspace and
   candidate capture, the 0B.4 work added the new
   executor. The vertical-slice test will need to be
   re-validated against the merged base.

4. **The Phase 1 schema snapshots.** The 6 new schema
   snapshots (1C.1, 1C.2, 1C.3, 1D.1, 1D.2, 1E.1) are
   on develop, but they were generated against Phase 1's
   types. After the rebase, the types may have been
   updated to use 0B.3/0B.4 primitives, and the
   snapshots may need to be regenerated. The
   `verify-schemas` step is the catch.

5. **The Phase 1 `Cargo.lock`.** Phase 1 added `debversion`,
   `deb822-lossless`, and other deps. After the rebase,
   the lockfile may need to be reconciled with master's
   existing deps (which already include the
   0B.3/0B.4 transitive closure). The auto-merge of
   `Cargo.lock` at the conflict-attempted merge is not
   a guarantee of correctness.

## The path forward (not a 30-conflict resolution)

The 30-conflict resolution is not a unit of work; it's the
signature of a structural problem. The path forward is:

### Step 1 — sync develop to master

```sh
# On develop
git fetch origin
git merge origin/master --no-ff
```

This is a develop ← master merge (not a release branch
promotion). The merge will surface the same 30 conflicts
in the same files, but the *resolution direction* is now
"Phase 1 changes get ported to the 0B.3/0B.4 base,"
which is a smaller-merge problem than "two parallel
implementations get reconciled." Specifically:

- For files where 0B.3/0B.4 is the new primitive and
  Phase 1's parallel implementation is now redundant:
  take master's file, port Phase 1's call sites to the
  0B.3/0B.4 API. The 0B.4 executor's `ToolRegistry`
  with `truncated=true` and per-job artifact caps is
  the right primitive; Phase 1's `record_builder.rs`
  should be ported to it, not the other way around.
- For files where Phase 1 is the new primitive and
  0B.3/0B.4 is the old (or missing) primitive: take
  develop's file, master gets the new behaviour. The
  five `ToolDefinitionRecord` registrations, the
  `handle_capture_candidate` pre-check, the
  `build_candidate_input` v3 schema, the
  `ResultNormalizer` activation — all of these are
  Phase 1's additions and master does not have them.
  The conflict resolution is "develop's file wins,
  master's file is regenerated."
- For files where both sides changed the same
  primitive: this is the hard case. The
  `crates/ironmaint-runtime/src/service.rs` file
  has 0B.3/0B.4's `handle_capture_candidate` shape
  and Phase 1's `handle_run_check` normalizer branch
  and Phase 1's `build_candidate_input` extension.
  The merge is hand-edited, file by file, with the
  §106 invariant ("no redesign of state machine,
  persistence, executor, candidate identity, evidence,
  state ownership, MCP boundary, or IronClaw
  integration") as the gate.

This is a multi-day exercise. It is not a 30-line
exercise. The architect's call is whether the 16 PRs of
Phase 1 land in their current form (with 0B.3/0B.4 call
sites re-pointed at the new primitives) or whether
some/all of Phase 1 gets re-cut to use the 0B.3/0B.4
primitives directly.

### Step 2 — re-validate the §97 gate on the merged base

```sh
make -f containers/Makefile gate
```

The merged base must be green. The `verify-architecture`
step will catch any new forbidden edge. The `verify-schemas`
step will catch any drift between Phase 1's types and
the 6 new schema snapshots. The `verify-seams` step
will catch any drift in the seam check wiring (the seam
check itself is unchanged; the *file* it walks is on
develop and survives the rebase). The integration
suite will catch any drift in the 1F.1 vertical-slice
walk and the 1G.1 maintainer-intake scenario.

If the §97 gate is not green on the merged base, the
rebase is not done. The completion doc's §41 DoD claims
are still valid against the 0B.2 base that was used, but
they are *not* valid against the merged base until the
gate is green. The architect's call is whether the
DoD claims need to be re-validated against the merged
base, or whether the completion doc is the right
artefact for the 0B.2 base that was used and a
*new* completion doc is needed for the merged base.

### Step 3 — re-open the release branch

Once the merged base is green, the release branch
`release/phase-1` is cut from the new develop tip. The
release/phase-1 → master PR is opened with the §41 DoD
claims re-validated against the merged base. The 30
conflicts are gone; the only remaining question is
"is the §97 gate green on the release/phase-1 base."

### Step 4 — post-mortem the discipline gaps

This post-mortem is the first artefact. The architect's
call on the discipline gap:

- AGENTS.md says "release branches are cut from develop."
  Did 0B.3 / 0B.4 violate that rule, or did the rule
  not exist at the time?
- AGENTS.md says "never commit directly to master." Did
  the 0B.3/0B.4 release branches bypass develop, or did
  the back-merge to develop simply not happen?
- The Phase 1 plan should have included a
  `git log origin/master ^origin/develop` pre-flight
  check. Should that check be a §106-mandated
  pre-condition for every Phase-X plan?

The answers to these questions are AGENTS.md
amendments, not code changes. The architect's call.

## What this post-mortem is not

- It is not a critique of the Phase 1 work. The 16 PRs
  of Phase 1 are correct against the 0B.2 base that was
  used. The §41 DoD claims are valid.
- It is not a critique of the 0B.3/0B.4 work. The
  0B.3/0B.4 PRs are correct against the 0B.2 base that
  was used. The §101 acceptance scenario is verified.
- It is not a critique of the merge. The release/phase-1
  → master PR was opened in good faith and the
  diagnosis came from running the merge.

It is a record of the process break. The architect
decides what to do about the discipline gaps. The
implementing agent decides what to do about the
rebase. The §106 invariants (no redesign of state
machine, persistence, executor, candidate identity,
evidence, state ownership, MCP boundary, or IronClaw
integration) are the gate.

## State at this post-mortem

- `master` is at `d5658ed` (unchanged; the failed
  release/phase-1 → master PR was closed without
  merging).
- `develop` is at `2729264` (unchanged; the 16 PRs of
  Phase 1 plus the completion doc).
- `release/phase-1` is deleted.
- PR #64 (`docs/phase-1-completion` → develop) is
  still open and is the right review input for the
  Phase 1 work on develop.
- PR #65 (release/phase-1 → master) is closed with
  a comment pointing to this post-mortem.

The next action is the develop ← master sync (Step 1
above). That is a multi-day exercise, not a 30-line
exercise, and it is the architect's call when to
start it.
