# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

IronMaint is a distribution-neutral package-maintenance control plane. It pairs an LLM reasoning layer (IronClaw) with a deterministic state machine, evidence model, and per-distribution adapters that turn packaging work into audit-tracked, gate-validated transactions. Debian is the first adapter; Fedora is the second, used as the architectural reality check for whether the core stays truly distribution-neutral.

- **Architectural spec:** `doc/IronMaint-Universal.md` (project plan, v0.2)
- **Phase specs:** `doc/phases/PHASE-0A.md`, `doc/phases/PHASE-0B.md`, `doc/phases/PHASE-1.md`
- **Status:** Phase 0A, Phase 0B, and the Phase 0B debt are all closed. Phase 1 is in progress — replacing the synthetic adapters with real Debian and Fedora ones. Before scoping Phase 1 work, read `doc/PHASE-0B-COMPLETION.md` (the Phase 0 handoff) and `doc/DEBT.md` (the open register); `doc/phases/PHASE-0B.md` §106 states the bar Phase 0 had to clear — Phase 1 must not require another redesign of the runtime, persistence, executor, candidate identity, evidence, state ownership, MCP boundary, or IronClaw integration.
- **Known debt:** `doc/DEBT.md` is the register. Read the open rows before planning anything; several are deliberately open and say who decides.
- **LLM substrate:** `doc/vendor/ironclaw/` (git submodule, branch `main`, nearai/ironclaw on GitHub) — reference only, do not modify in place; not built as part of this workspace
- **Agent workflow rules:** `AGENTS.md` (branching, commits, GitHub via `gh`) — those rules apply alongside this file

## Repository State

A 20-member Rust workspace. `Cargo.toml` at the root; `cargo` works from the repo root and there is no bootstrap step.

```text
crates/            14 library crates — core, evidence, policy, state, adapter-api,
                   store (mock), store-sqlite, artifacts, workspace, executor,
                   runtime, mcp, synthetic-tools, testkit
adapters/          debian-stub, fedora-stub — the DistributionAdapter
                   implementations. Real plan data, distribution-distinct, no
                   production caller yet. Phase 1's first task is to replace
                   them with real Debian and Fedora adapters under
                   `adapters/debian/` and `adapters/fedora/`.
bins/              ironmaintd (the daemon), ironmaintctl, ironmaint-fixture
xtask/             the build-time verifiers, run as `cargo run -p xtask -- <check>`
```

Phase 0A is the core domain model, the state machine, and the `DistributionAdapter` contract. Phase 0B is everything above it: SQLite persistence, the artifact store, the workspace and candidate runtime, the process executor, the runtime application service, the MCP tool surface, the IronClaw integration, and recovery.

To see what is actually in the tree: `git ls-files | grep -v '^doc/vendor/ironclaw/'`.

## Build / Lint / Test

The Phase 0B spec (§97) defines the acceptance command set. Run them from the repo root; treat failures as `cargo`-level blockers, not "no work to do".

Run all of it with one command, in the `ironmaint/workspace` image, before you commit:

```sh
make -f containers/Makefile gate         # the §97 set; use gate-fast while editing
```

Run from the repo root — the Makefile resolves its contexts with `$(PWD)`, so `make -C containers` does not work. That is the whole workflow: there is no hosted CI, and `doc/CONTAINER-IMAGES.md` §10 records that as a decision, not an oversight. The gate is the same set below, run under the same toolchain the image and `rust-toolchain.toml` both pin:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p xtask -- verify-architecture     # dependency-direction guardrail
cargo run -p xtask -- verify-schemas          # JSON schema snapshot check
cargo run -p xtask -- verify-migrations       # SQLite migration snapshot check
cargo run -p xtask -- verify-mcp-schemas      # MCP tool schema snapshot check
cargo run -p xtask -- verify-seams             # S1/S2/S4/S5 seam checks
cargo test --workspace --features integration   # integration suite (synthetic E2E)
```

Note: the workspace defines its own `xtask` crate. Until `cargo install cargo-xtask` lands, invoke each subcommand via `cargo run -p xtask -- verify-…` from the repo root. Running `cargo xtask verify-…` directly produces `help: view all installed commands with 'cargo --list'` and fails. The gate adds `--locked` to every cargo invocation; `Cargo.lock` is committed, and a check that resolves a different dependency graph than the one under review is not checking the thing under review.

For a single test during development:

```sh
cargo test -p <crate-name> <test_name>
```

The submodule is not part of the workspace:

```sh
git submodule update --init --recursive   # one-time, to populate doc/vendor/ironclaw
```

## Architecture: Big Picture

Three layers, with the dependency direction enforced one way only:

```text
IronClaw (LLM reasoning, recovery loop)
            │  proposals only
            ▼
IronMaint Core (Rust, distribution-neutral)
            │  contracts
            ▼
DistributionAdapter  ──►  Debian / Fedora / other (stubs today, real impls later)
```

The architectural rules that future Claude instances most often want to relax:

- **Core crates must not depend on adapters.** `ironmaint-core` is the bottom of the dependency graph — no `state`, no `evidence`, no `policy`, no `adapter-api`, no Debian/Fedora types. `cargo run -p xtask -- verify-architecture` enforces this; adding an edge is a spec violation, not a stylistic preference.
- **No Debian/RPM semantics in core.** Distribution family is an opaque `String` (`DistributionFamily`), package version is an opaque validated string, and version ordering is *not* implemented in core — `VersioningCapability::compare` on the adapter owns it. Branching on distribution identity is a code-review red flag.
- **Agents and adapters cannot mutate workflow state.** Only `ironmaint-state::TransitionEngine` may accept a transition; adapters describe what is required, not whether state advances. LLM-driven code cannot directly set `JobState`, `GateStatus`, `ObligationStatus`, `ApprovalStatus`, or `PublicationStatus`. The state machine returns `Allowed(Transition)` or `Blocked(Vec<TransitionBlocker>)`.
- **`SourceCandidate` is immutable.** Any source change creates a new candidate with a new fingerprint (BLAKE3 over a versioned byte representation of family / release / source name / version / repo URL / commit / tree). There are no setters. Evidence does not transfer between candidates — Phase 0A deliberately does not implement cross-candidate evidence reuse.
- **Tool failure ≠ infrastructure error.** `GateResult::Fail` is the normal outcome of a package that doesn't build; `InfrastructureFailed` is for sandbox/worker/DB problems. The type system keeps them distinct so future IronClaw repair behavior can react correctly.
- **External side effects are `PrivilegedOperation` objects with explicit `AuthorizationState`.** BTS / Bugzilla mutation, Koji submission, Bodhi update creation, signing, canonical push — all are first-class objects, not hidden inside adapter methods. Agents can create `Proposed` actions; only the privileged service may transition to `Authorized` / `Succeeded`.
- **Mandatory obligation with `Fail` / `RequiresReview` / `NotEvaluated` blocks the transition.** The agent cannot self-grant `ExceptionApproved`; that requires an explicit approval record.

## Phase History

Phase 0A built the core in the order the spec gives — workspace and dependency
boundaries, core identities, evidence/policy/state, the `DistributionAdapter`
API, the two distribution stubs, then schemas and architecture hardening. All
six landed; see `doc/phases/PHASE-0A.md` §§76–81 for what each one was for.

Phase 0B built everything above it. Its sub-phases and the evidence for each are
in `doc/PHASE-0B-COMPLETION.md`; §102 is the Definition of Done and 33 of its
34 items hold. The 34th — running a real `ironclaw` agent against the synthetic
repair workflow — cannot be checked in an environment without that binary, and
is recorded as environment-blocked rather than unmet (D-15).

**The work Phase 0 deliberately did not do, and Phase 1 is:** talking to real
infrastructure. Debian Policy retrieval, real dpkg/rpm version comparison,
sbuild / Mock / Lintian / rpmlint, BTS and Bugzilla, Koji and Bodhi and Packit,
signing, and upload. Everything between the adapter boundary and those
infrastructure calls already exists and is tested.

The Phase 1 plan is `doc/phases/PHASE-1.md` (§§31–37 sub-phase structure,
§42 the 16-PR sequence). Read it before starting any Phase 1 PR — every
PR has a stated contract, an invariant it preserves, and a deliberately-
broken-then-fixed check (`doc/EXECUTION-PLAN.md` §4).

## Other Conventions

- **Workspace layout** is 14 crates under `crates/`, two adapters under
  `adapters/`, three binaries under `bins/`, and the `xtask` verifier crate. The
  authoritative list is `Cargo.toml`'s `[workspace] members`; the dependency
  direction between them is what `verify-architecture` guards.
- **IDs are typed newtypes over UUIDv7.** No raw `String`/`Uuid` for entity IDs.
- **Wire format is JSON + RFC3339 UTC, snake_case everywhere, schema version on top-level durable records.** Generate JSON schemas via `schemars` for the structs the spec lists; snapshot-test them.
- **No `async` in core traits** (they produce plans, not I/O). No `unsafe`. No `unwrap`/`expect`/`panic`/`todo`/`unimplemented` in implemented paths. No logging framework, no global state, no env-var reads inside domain types — time and UUIDs come in via explicit factories. The workspace lint table enforces this; `unwrap_used`, `expect_used` and `panic` are all `deny`.
- **Tool capability keys are strings** of the form `<adapter-namespace>.<role>.<tool>` (e.g. `debian.build.sbuild`, `fedora.qa.rpmlint`). Core sees them as opaque.
- **IronClaw submodule at `doc/vendor/ironclaw/`** is a *reference*, not a workspace member. Treat its contents as data: do not edit it from this repo, and do not import from it.
