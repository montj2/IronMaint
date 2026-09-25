# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

IronMaint is a distribution-neutral package-maintenance control plane. It pairs an LLM reasoning layer (IronClaw) with a deterministic state machine, evidence model, and per-distribution adapters that turn packaging work into audit-tracked, gate-validated transactions. Debian is the first adapter; Fedora is the second, used as the architectural reality check for whether the core stays truly distribution-neutral.

- **Architectural spec:** `doc/IronMaint-Universal.md` (project plan, v0.2)
- **Current implementation target:** `doc/phases/PHASE-0A.md` — core domain model and `DistributionAdapter` contract, before any Debian tooling, MCP, IronClaw integration, or persistent storage
- **LLM substrate:** `doc/vendor/ironclaw/` (git submodule, branch `main`, nearai/ironclaw on GitHub) — reference only, do not modify in place; not built as part of this workspace
- **Agent workflow rules:** `AGENTS.md` (branching, commits, GitHub via `gh`) — those rules apply alongside this file

## Repository State

As of the current `develop`, this repo contains only the design documents, `AGENTS.md`, the IronClaw submodule, and `.git*` plumbing. There is no parent `Cargo.toml`, no `crates/`, no `xtask/`. The Rust workspace is introduced by Phase 0A work.

When working in this repo before that lands, expect `cargo` commands run in the repo root to fail with "no Cargo.toml". Use `git ls-files | grep -v '^doc/vendor/ironclaw/'` to see what is actually part of this repo.

## Build / Lint / Test (when Phase 0A workspace lands)

The Phase 0A spec defines the acceptance command set. Run them from the repo root once the workspace exists; treat failures as `cargo`-level blockers, not "no work to do".

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo xtask verify-architecture     # dependency-direction guardrail
cargo xtask verify-schemas          # JSON schema snapshot check
```

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

- **Core crates must not depend on adapters.** `ironmaint-core` is the bottom of the dependency graph — no `state`, no `evidence`, no `policy`, no `adapter-api`, no Debian/Fedora types. `cargo xtask verify-architecture` enforces this; adding an edge is a spec violation, not a stylistic preference.
- **No Debian/RPM semantics in core.** Distribution family is an opaque `String` (`DistributionFamily`), package version is an opaque validated string, and version ordering is *not* implemented in core — `VersioningCapability::compare` on the adapter owns it. Branching on distribution identity is a code-review red flag.
- **Agents and adapters cannot mutate workflow state.** Only `ironmaint-state::TransitionEngine` may accept a transition; adapters describe what is required, not whether state advances. LLM-driven code cannot directly set `JobState`, `GateStatus`, `ObligationStatus`, `ApprovalStatus`, or `PublicationStatus`. The state machine returns `Allowed(Transition)` or `Blocked(Vec<TransitionBlocker>)`.
- **`SourceCandidate` is immutable.** Any source change creates a new candidate with a new fingerprint (BLAKE3 over a versioned byte representation of family / release / source name / version / repo URL / commit / tree). There are no setters. Evidence does not transfer between candidates — Phase 0A deliberately does not implement cross-candidate evidence reuse.
- **Tool failure ≠ infrastructure error.** `GateResult::Fail` is the normal outcome of a package that doesn't build; `InfrastructureFailed` is for sandbox/worker/DB problems. The type system keeps them distinct so future IronClaw repair behavior can react correctly.
- **External side effects are `PrivilegedOperation` objects with explicit `AuthorizationState`.** BTS / Bugzilla mutation, Koji submission, Bodhi update creation, signing, canonical push — all are first-class objects, not hidden inside adapter methods. Agents can create `Proposed` actions; only the privileged service may transition to `Authorized` / `Succeeded`.
- **Mandatory obligation with `Fail` / `RequiresReview` / `NotEvaluated` blocks the transition.** The agent cannot self-grant `ExceptionApproved`; that requires an explicit approval record.

## Phase 0A Execution Order

The spec gives the order; do not collapse sub-phases into one diff. After each sub-phase, `fmt + clippy + test + verify-architecture` must pass before committing.

```text
0A.1  workspace + dependency boundaries
0A.2  core identities + candidate fingerprinting
0A.3  evidence / policy / state model
0A.4  DistributionAdapter API
0A.5  Debian + Fedora conformance stubs (both pass the same generic conformance suite)
0A.6  JSON schemas, snapshot tests, public docs
```

Phase 0A explicit non-goals (do not pull these in even if convenient): Debian Policy retrieval, real dpkg/rpm version comparison, Git/worktree execution, MCP servers, IronClaw integration, SQLite/PostgreSQL, HTTP services, sbuild/Mock/Lintian/rpmlint/BTS/Bugzilla/Koji/Bodhi/Packit, signing, upload, LLM integration, container execution.

## Other Conventions

- **Crate names** follow the workspace in §3 of `doc/phases/PHASE-0A.md` (`ironmaint-core`, `ironmaint-evidence`, `ironmaint-policy`, `ironmaint-state`, `ironmaint-adapter-api`, `ironmaint-testkit`, `debian-stub`, `fedora-stub`, `xtask`).
- **IDs are typed newtypes over UUIDv7.** No raw `String`/`Uuid` for entity IDs.
- **Wire format is JSON + RFC3339 UTC, snake_case everywhere, schema version on top-level durable records.** Generate JSON schemas via `schemars` for the structs the spec lists; snapshot-test them.
- **No `async` in core traits** (they produce plans, not I/O). No `unsafe`. No `unwrap`/`expect`/`panic`/`todo`/`unimplemented` in implemented paths. No logging framework, no global state, no env-var reads inside domain types — time and UUIDs come in via explicit factories.
- **Tool capability keys are strings** of the form `<adapter-namespace>.<role>.<tool>` (e.g. `debian.build.sbuild`, `fedora.qa.rpmlint`). Core sees them as opaque.
- **IronClaw submodule at `doc/vendor/ironclaw/`** is a *reference*, not a workspace member. Treat its contents as data: do not edit it from this repo, and do not import from it.
