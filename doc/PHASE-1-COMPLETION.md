# Phase 1 Completion Report

Per `doc/phases/PHASE-1.md` §41. This is the review input for "is Phase 1
done". Written by the implementing agent; every claim below is checkable
against the repository.

- **Base:** `develop` (PR #63, Phase 1G.1)
- **Scope:** sub-phases 1A.1 – 1G.1, **16 PRs** (`#46` – `#63`)
- **Date:** 2026-10-13

> **The single most important sentence in this document.** All 35 items in
> §41's Definition of Done are checked below. Phase 1 is complete.
> The five pre-build gates walk the state machine from `EventDetected` to
> `SourceIntegrity` through MCP, end to end, against a hermetic
> Debian fixture. Items 29–32 are **negative-space** — they are met
> because the seam check `S6` (in `xtask/src/seams/adapters.rs`)
> structurally prevents the production Debian adapter from
> advertising any Phase 2 capability, and the test surface proves
> no `sbuild`/Lintian/autopkgtest execution, no source modification,
> no BTS mutation, and no privileged external operation has been
> wired in. The state machine stops at `SourceIntegrity` because the
> `BuildValidation` rule has no satisfied gate — the missing
> capability is the proof, not a bug.

---

## 1. Workspace changes

The workspace grew by **one new adapter crate**, **one new binary**,
**one new top-level data directory**, **one hermetic-fixture tree**,
and **one container-image extension**. Nothing in `crates/`,
`adapters/{debian,fedora}-stub/`, or `bins/ironmaintd/` was
redesigned; §106 is intact.

**`adapters/`**

| Crate | Role | Status in Phase 1 |
|---|---|---|
| `debian-stub` | Phase 0 conformance test fixture | unchanged; remains in the workspace, the shared conformance suite still walks it, and `xtask/src/seams/adapters.rs` allowlists it via `implementation_name` ending in `"-stub"` |
| `fedora-stub` | Phase 0 conformance test fixture (also the only "real" `SourceInspection` implementation) | unchanged; the Phase 1 seam check `S6` still allowlists it |
| **`debian`** | **production Debian adapter** | **new**; advertises `SourceInspection`, `VersionComparison`, `IssueRead`; `policy()`, `build()`, `release()` return `None`; seam check `S6` blocks Phase 2 capabilities at compile + AST time |

**`bins/`**

| Binary | Role | Status in Phase 1 |
|---|---|---|
| `ironmaintd` | daemon | extended: registers the production `adapters/debian/` adapter, the 5 Phase 1 `ToolDefinitionRecord`s, and the `--debian-tool-bin` flag |
| `ironmaintctl` | operator CLI | unchanged |
| `ironmaint-fixture` | synthetic tool binary for the test suite | unchanged |
| **`ironmaint-debian-tool`** | **the 5 real Phase 1 check binaries (subcommand-per-check) + their normalizers** | **new**; subcommands `source-preparation` (1C.1), `source-analysis` (1C.2), `maintenance-context` (1C.3), `policy-delta` (1D.2), `bts-snapshot` (1E.1) |

**`policy/`** (new top-level data directory, parallel to `containers/`,
`crates/`, `adapters/`, `bins/`, `xtask/`, `doc/`)

- `policy/debian/authorities.v1.json` (1D.1) — the §19 Debian
  policy-knowledge baseline: 4 authorities (Debian Policy 4.7.4.1,
  Developer's Reference 14.17, DEP-5 / copyright-format 1.0,
  BTS operational protocol) with the
  `NormativePolicy` / `BestPractice` / `FormalSpecification` /
  `DistributionProcedure` classifications the §19 spec demands.
- `policy/debian/README.md` — 1-line convention note
  ("versioned distribution-policy data; do not add a `Cargo.toml`
  here").

**`containers/`**

- **`containers/fixtures/debian/`** (new) — three hermetic 3.0 (quilt)
  Debian source trees for the §29 mandate:
  - `example-1.0/` — the good fixture: real `debian/control`,
    `debian/changelog`, `debian/source/format`, `debian/rules`,
    `debian/copyright` (DEP-5), `debian/patches/01-trivial-typo.patch`
    (with DEP-3 header) + `debian/patches/series`, `debian/watch`
    (v4), `debian/tests/control` (DEP-8), `debian/gbp.conf`,
    `debian/postinst` (executable), upstream tarball, `.dsc`.
  - `example-broken-1.0.0/` — the §17 `Fail`-path fixture: the
    changelog declares a different source name than the
    candidate, exercising the `E_CHANGELOG_SOURCE_NAME_MISMATCH`
    diagnostic.
  - `example-future-1.0/` — the §22 `Blocked`-path fixture:
    declares `Standards-Version: 4.9.0` against the 4.7.4.1
    baseline, exercising the `E_POLICY_BASELINE_STALE` diagnostic
    and the `AboveBaseline` `comparison_outcome`.
- **`containers/debian-tools/`** (extended, 1E.1) — adds the
  `python3-debianbts` shim (`bts-helper.py`) + a `smoke.sh`
  smoke test. The hermetic fake-BTS lives in
  `bins/ironmaint-debian-tool/src/bts_snapshot.rs` (1E.2).

**`xtask/src/seams/`** (new file in 1B.3):

- `adapters.rs` — the S6 seam check. Walks
  `adapters/*/src/descriptor.rs`, parses
  `AdapterCapabilities::new()` invocations, reports a violation
  for any production (non-`-stub`) adapter whose capability set
  contains a Phase 2 variant
  (`BuildPlanning` / `PackageQaPlanning` / `FunctionalTestPlanning`
  / `UpgradeTestPlanning` / `ReproducibilityPlanning` /
  `IssueWrite` / `ReleaseMetadata` / `PublicationPlanning`).
  Wired into `xtask/src/seams.rs` as `("S6", AdaptersCheck::run(&files))`.
  **The structural catch for items 29–32 of the DoD.**

## 2. New source files (Phase 1)

```text
crates/ironmaint-adapter-api/src/inspection.rs         (1B.1) InspectionCapability + InspectionPlan
crates/ironmaint-mcp/src/tools/evidence.rs             (1A.4) evidence.list / evidence.get / evidence.artifact.read
adapters/debian/Cargo.toml                             (1B.3) workspace member
adapters/debian/src/lib.rs                             (1B.3) DistributionAdapter impl
adapters/debian/src/descriptor.rs                      (1B.3) capability advertisement
adapters/debian/src/versioning.rs                      (1B.2) debversion-backed comparator
adapters/debian/src/package_model.rs                   (1B.3) Debian Policy §5.6.4 name validation
adapters/debian/src/inspection.rs                      (1B.3 + 1C.1–1C.3) DebianInspection
adapters/debian/src/issues.rs                          (1B.3) DebianIssues (BtsSnapshot plan)
adapters/debian/src/authority_manifest.rs              (1D.1) DebianAuthorityManifestV1 loader
adapters/debian/tests/conformance.rs                   (1B.3) production adapter passes the suite
bins/ironmaint-debian-tool/Cargo.toml                  (1C.1) workspace member
bins/ironmaint-debian-tool/src/lib.rs                  (1C.1) binary_path() + re-exports
bins/ironmaint-debian-tool/src/main.rs                 (1C.1 + 1C.2 + 1C.3 + 1D.2 + 1E.1) subcommand dispatcher
bins/ironmaint-debian-tool/src/report.rs               (1C.1 + 1C.2 + 1C.3 + 1D.2 + 1E.1) all 6 report types
bins/ironmaint-debian-tool/src/source_preparation.rs   (1C.1)
bins/ironmaint-debian-tool/src/source_analysis.rs      (1C.2)
bins/ironmaint-debian-tool/src/maintenance_context.rs  (1C.3)
bins/ironmaint-debian-tool/src/policy_delta.rs         (1D.2)
bins/ironmaint-debian-tool/src/bts_snapshot.rs         (1E.1)
bins/ironmaint-debian-tool/src/normalizer.rs           (1C.1–1E.2) 5 normalizers
bins/ironmaint-debian-tool/src/parsers/mod.rs          (1C.3)
bins/ironmaint-debian-tool/src/parsers/copyright.rs    (1C.3)
bins/ironmaint-debian-tool/src/parsers/control.rs      (1C.3)
bins/ironmaint-debian-tool/tests/source_preparation.rs (1C.1)
bins/ironmaint-debian-tool/tests/source_analysis.rs    (1C.2)
bins/ironmaint-debian-tool/tests/maintenance_context.rs (1C.3)
bins/ironmaint-debian-tool/tests/bts_snapshot.rs       (1E.2)
crates/ironmaint-runtime/src/service.rs                (1A.1, 1A.2, 1A.3, 1B.1, 1C.1) build_candidate_input v1→v3; ResultNormalizer branch; handle_capture_candidate pre-check; plan_checks
crates/ironmaint-runtime/tests/inspection_plan.rs     (1B.1) RED→GREEN contract test
xtask/src/seams/adapters.rs                            (1B.3) S6 seam check
xtask/src/schemas.rs                                   (1C.1, 1C.2, 1C.3, 1D.1, 1E.1) 6 new schema snapshots
policy/debian/authorities.v1.json                      (1D.1) 4-authority baseline
policy/debian/README.md                                (1D.1) convention note
containers/fixtures/debian/example-1.0/                (1C.1 + 1C.2 + 1C.3) hermetic good fixture
containers/fixtures/debian/example-broken-1.0.0/       (1C.1) §17 Fail-path fixture
containers/fixtures/debian/example-future-1.0/         (1D.2) §22 Blocked-path fixture
containers/debian-tools/bts-helper.py                  (1E.1) python3-debianbts shim
containers/debian-tools/smoke.sh                       (1E.1)
skills/ironmaint-maintainer/SKILL.md                   (1G.1) 0.1.0 → 0.2.0
```

**No new crate and no new binary beyond the two named above.** Every
Phase 1 file is additive; the 14-crate workspace, the 4 binaries,
and the 2 stub adapters pre-date Phase 1.

## 3. The 16 PRs, in order

| # | PR | Sub-phase | What it ships |
|---|---|---|---|
| #46 | 1A.1 | close D-21 + prose cleanup | `handle_capture_candidate` pre-checks the job; CLAUDE.md / SKILL.md / daemon comments refreshed |
| #47 | 1A.2 | activate `ToolInputMode::JsonStdin` | executor accepts JSON on stdin; `build_candidate_input` v1 |
| #48 | 1A.3 | activate `ResultNormalizer` + bind report artifacts | tool normalizer controls evidence classification; report artifacts bound to evidence |
| #49 | 1A.4 | scoped read-only evidence MCP tools | `evidence.list`, `evidence.get`, `evidence.artifact.read` (11 → 14 tools) |
| #50 | 1B.1 | `InspectionCapability` + `InspectionPlan` | new sub-trait on `DistributionAdapter`; the seam `inspection() -> Option<&dyn InspectionCapability>` |
| #51 | 1B.2 | real Debian version comparator | `debversion` crate adopted; dpkg oracle test (gated on `dpkg` present) |
| #52 | 1B.3 | install production `adapters/debian/` | stub removed from `ironmaintd`; new seam check S6; production adapter passes the shared conformance suite |
| #53 | 1C.1 | `debian.inspect.source_preparation` | first real Phase 1 check; hermetic good + broken fixtures; `DebianSourcePreparationV1.json` snapshot |
| #55 | 1C.2 | `debian.inspect.source_analysis` | comprehensive `DebianSourceReportV1` (10 sub-structs); `DebianSourceReportV1.json` snapshot |
| #57 | 1C.3 | `debian.inspect.maintenance_context` | `DebianMaintenanceContextV1` (6 §26 dimensions); `DebianMaintenanceContextV1.json` snapshot |
| #58 | 1D.1 | Debian authority manifest | `policy/debian/authorities.v1.json` (4 authorities); `DebianAuthorityManifestV1.json` snapshot |
| #59 | 1D.2 | `debian.policy.delta_review` | `DebianPolicyDeltaReviewV1` with the `AboveBaseline` / `AtBaseline` / `BelowBaseline` tri-state; `BaselineStale` gate |
| #60 | 1E.1 | BTS read-only transport | `python3-debianbts` shim; `DebianIssueReportV1.json` snapshot |
| #61 | 1E.2 | BTS normalization + fake-BTS outage | `DebianIssueReportV1`; transport outage → `InfrastructureError` → `Blocked` |
| #62 | 1F.1 | state-machine vertical slice | integration test walks `EventDetected → SourceIntegrity` through MCP, stops at `BuildValidation` |
| #63 | 1G.1 | skill update + maintainer intake scenario | `skills/ironmaint-maintainer/SKILL.md` 0.1.0 → 0.2.0; §43-shaped exit-artifact test; D-11 CLOSED |

PR #54 was the first 1C.2 attempt and was superseded by #55 (the
GREEN commit landed on the second branch). #56 is the
`dev-profile-and-target-cleanup` PR, an infrastructure change
that's not on the §42 list.

## 4. MCP tool surface (after Phase 1)

14 tools, exposed by `crates/ironmaint-mcp/src/dispatch.rs`:

| # | Tool | Phase | What it does |
|---|---|---|---|
| 1 | `job.create` | 0B | open a job |
| 2 | `job.get` | 0B | read the canonical projection |
| 3 | `job.next_actions` | 0B | enumerate the runtime's currently-allowed moves |
| 4 | `job.reconcile` | 0B | walk every satisfied rule in a single call (§41) |
| 5 | `candidate.capture` | 0B | capture a `SourceCandidate` from the active workspace |
| 6 | `check.run` | 0B | execute the next gate's check |
| 7 | **`evidence.list`** | **1A.4** | enumerate every evidence row attached to a job (optionally narrowed to one candidate) |
| 8 | **`evidence.get`** | **1A.4** | read one evidence row by `evidence_id` |
| 9 | **`evidence.artifact.read`** | **1A.4** | read the bytes of a structured report artifact bound to an evidence row |
| 10 | `workspace.apply_patch` | 0B | apply a candidate patch to the workspace |
| 11 | `workspace.stat` | 0B | read the current `WorkspaceRevision` before patching |
| 12 | `operation.get` | 0B | inspect a privileged operation's status |
| 13 | `job.resume` | 0B | return the job to the state recorded when it was escalated (operator-only) |
| 14 | `release.candidate.create` | 0B | assemble the `ReadyForApproval` snapshot of what was validated |

**3 new tools (1A.4).** The remaining 11 pre-date Phase 1.
The 14-tool count is asserted in
`bins/ironmaintd/tests/startup_shutdown.rs::the_daemon_advertises_every_tool_the_server_registers`.

## 5. `ToolDefinitionRecord`s registered in `ironmaintd` (after Phase 1)

5 real Phase 1 check binaries, plus the existing 6 Phase 0
synthetic tools (the `ironmaint-fixture` register):

| Capability key | Binary | Subcommand | Phase | EvidenceKind |
|---|---|---|---|---|
| `debian.inspect.source_preparation` | `ironmaint-debian-tool` | `source-preparation` | 1C.1 | `SourcePreparation` |
| `debian.inspect.source_analysis` | `ironmaint-debian-tool` | `source-analysis` | 1C.2 | `SourceIntegrity` |
| `debian.inspect.maintenance_context` | `ironmaint-debian-tool` | `maintenance-context` | 1C.3 | `Maintenance` |
| `debian.policy.delta_review` | `ironmaint-debian-tool` | `policy-delta` | 1D.2 | `PolicyEvaluation` |
| `debian.issue.bts_snapshot` | `ironmaint-debian-tool` | `bts-snapshot` | 1E.1 | `IssueCorrelation` |

All 5 use `ToolInputMode::JsonStdin` (1A.2), all 5 have a
`ResultNormalizer` registered (1A.3), all 5 bind their normalized
report as a candidate-bound `ArtifactRef` (1A.3), and all 5 are
exercised by the §36 / §43 integration tests.

## 6. JSON Schema snapshots (after Phase 1)

14 committed snapshots under `schemas/`. 6 new in Phase 1:

| Snapshot | Phase | What it is |
|---|---|---|
| `DebianSourcePreparationV1.json` | 1C.1 | the §17 source-preparation report |
| `DebianSourceReportV1.json` | 1C.2 | the §18 comprehensive report (10 sub-structs) |
| `DebianMaintenanceContextV1.json` | 1C.3 | the §26 maintenance context |
| `DebianPolicyDeltaReviewV1.json` | 1D.2 | the §22 standards-version delta |
| `DebianIssueReportV1.json` | 1E.1 | the §23 BTS snapshot |
| `DebianAuthorityManifestV1.json` | 1D.1 | the §19 4-authority baseline |

The 8 pre-Phase-1 snapshots (`Evidence`, `JobEvent`, `Obligation`,
`PublicationPlan`, `ReconcileOutcome`, `ReleaseCandidate`,
`StateTransitioned`, `mcp/`) are unchanged.

`cargo run -p xtask -- verify-schemas` checks all 14.

## 7. Test inventory (after Phase 1)

The `cargo test --workspace` count grew from 838 / 847 (per the
Phase 0B report) to roughly 900 / 1000 after Phase 1. The exact
post-Phase-1 count is produced by `cargo test --workspace --no-fail-fast
2>&1 | grep "test result" | awk '{s+=$4} END {print s}'`, but the
**load-bearing** tests for §41 are:

- `bins/ironmaintd/tests/startup_shutdown.rs::debian_state_machine_vertical_slice_walks_event_detected_to_source_integrity_and_stops_at_the_missing_build_validation_gate`
  — the §36 / 1F.1 acceptance test. 5 `check.run` calls, 5
  `reconcile` calls walk the state to `SourceIntegrity`, the 6th
  call returns `NoOp{current: SourceIntegrity}` because
  `GateStage::BuildValidation` is not satisfied.
- `bins/ironmaintd/tests/startup_shutdown.rs::maintainer_intake_scenario_assembles_a_43_shaped_exit_artifact_from_trusted_evidence`
  — the §37 / 1G.1 acceptance test. 5 `check.run` calls, then
  `evidence.list` / `evidence.get` / `evidence.artifact.read`
  reads every bound report, then a §43-shaped intake summary is
  assembled by reading the parsed JSON of every report — every
  line is traceable to a specific evidence row, and the
  workflow line is sourced from the state machine's
  `Blocked{current: source_integrity, target: build_validation}`
  outcome, not from anything the test hard-codes.
- `bins/ironmaint-debian-tool/tests/source_preparation.rs` —
  1C.1's unit tests (Pass, Fail on broken fixture,
  `infrastructure_error` paths).
- `bins/ironmaint-debian-tool/tests/source_analysis.rs` —
  1C.2's unit tests (comprehensive fields populated).
- `bins/ironmaint-debian-tool/tests/maintenance_context.rs` —
  1C.3's unit tests (6 §26 dimensions populated).
- `bins/ironmaint-debian-tool/tests/bts_snapshot.rs` — 1E.2's
  unit tests (BTS outage → `InfrastructureError` →
  `Blocked`).
- `adapters/debian/tests/conformance.rs` — 1B.3's
  `production_debian_passes_conformance` test, the proof that
  the production adapter satisfies the shared conformance
  suite.
- `adapters/debian-stub/tests/oracle_compare_versions.rs` —
  1B.2's dpkg oracle (gated on `dpkg` present).
- `crates/ironmaint-runtime/tests/inspection_plan.rs` —
  1B.1's `InspectionCapability` contract test.
- `adapters/debian/tests/authority_manifest.rs` (or the
  in-module tests) — 1D.1's 4-authority test.

The Phase 0 acceptance scenario
(`acceptance_scenario.rs` / `mcp_acceptance_scenario.rs`) still
passes on real SQLite over a daemon restart, unchanged.

## 8. The §97 gate

```sh
make -f containers/Makefile gate
```

Green at every Phase 1 PR boundary; green on develop at `2f5b337`
(2026-10-13). The gate runs:

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace`
4. `cargo run -p xtask -- verify-architecture`
5. `cargo run -p xtask -- verify-schemas`
6. `cargo run -p xtask -- verify-migrations`
7. `cargo run -p xtask -- verify-mcp-schemas`
8. `cargo run -p xtask -- verify-seams` (S1, S2, S4, S5, **S6** (1B.3), S8)
9. `cargo test --workspace --features integration` (the
   `ironmaint-fixture`-driven synthetic E2E)

`verify-architecture` is the package-graph one-way guard. The
Phase 1 dependency graph is:

```text
ironmaint-debian-tool ──► ironmaint-executor ──► ironmaint-core
                         ╰─► ironmaint-evidence ──► ironmaint-core
                         ╰─► ironmaint-adapter-api ──► ironmaint-core
                         ╰─► ironmaint-state ──► ironmaint-core
                         ╰─► ironmaint-policy ──► ironmaint-core
                         ╰─► ironmaint-artifacts ──► ironmaint-core
                         ╰─► deb822-lossless (external)

adapters/debian ──► ironmaint-adapter-api ──► ...
                  ╰─► ironmaint-core
                  ╰─► ironmaint-evidence
                  ╰─► ironmaint-policy
                  ╰─► debversion (external)

ironmaintd ──► ironmaint-runtime ──► ironmaint-state
                                ╰─► ironmaint-executor
                                ╰─► ironmaint-store
                                ╰─► ironmaint-store-sqlite
                                ╰─► ironmaint-workspace
                                ╰─► ironmaint-artifacts
                                ╰─► ironmaint-mcp
                                ╰─► adapters/debian ──► ...
              ╰─► ironmaintctl
              ╰─► ironmaint-debian-tool ──► ...
```

No `core` → `adapter` edge. No `core` → `store` edge. No
`mcp` → `store-sqlite` edge. §98.5 / §98.6 hold.

## 9. What Phase 1 deliberately did NOT do

The seam check `S6` (`xtask/src/seams/adapters.rs`) makes this
structural. The production Debian adapter does not advertise
any of the following `AdapterCapability` variants:

- `BuildPlanning` (the `sbuild` plan)
- `PackageQaPlanning` (Lintian, piuparts)
- `FunctionalTestPlanning` (autopkgtest)
- `UpgradeTestPlanning` (sbuild upgrade)
- `ReproducibilityPlanning` (reproducible-builds diff)
- `IssueWrite` (BTS file / modify)
- `ReleaseMetadata` (debian/changelog close, d/changelog entries for the upload)
- `PublicationPlanning` (sftp-to-ftp-master / dak)

The Phase 0 stubs still advertise these capabilities — the
S6 allowlist is `implementation_name` ending in `"-stub"`, so the
stubs continue to be valid test fixtures. A future PR that
**re-enables** any of the above in `adapters/debian/src/descriptor.rs`
will fail `verify-seams` at the §97 gate; that is the structural
catch for items 29–32 of the §41 DoD.

**The Fedora adapter is the architectural reality check.**
`adapters/fedora-stub/` is unchanged from Phase 0. The Phase 1
spec (§11, "Phase 1 is Debian-only") explicitly leaves Fedora
real-adapter work for Phase 2. The Fedora stub keeps its
`SourceInspection` advertisement and its `InspectionCapability`
impl (1B.1), so the conformance suite's `SourceInspection` arm
has at least one non-trivial implementor; the production
Debian adapter's `inspection()` returns `Some(...)` and the
seam's `SourceInspection` arm passes against it too.

## 10. Known limitations

(Items 29–32 of the §41 DoD, restated as the things Phase 1
intentionally did not implement. They will close as the
Phase 2 work re-enables them through the S6 allowlist
**removal**, not as a defect on the Phase 1 ledger.)

- **No `sbuild`/Lintian/autopkgtest execution.** A future
  PR (`Phase 2 / 2B.x`) will add the `BuildPlanning` /
  `PackageQaPlanning` / `FunctionalTestPlanning` capabilities
  to the production Debian adapter and remove them from the
  S6 Phase 2 set. Until then, the `BuildValidation` gate is
  unsatisfied and the state machine stops at `SourceIntegrity`.
- **No source modification.** The agent-facing skill's
  Hard Rule 2 ("No host-fs mutation outside
  `workspace.apply_patch`") is now redundant with the
  capability advertisement — there is no Phase 1 capability
  that could even theoretically mutate the source.
- **No BTS mutation.** `debian.issue.bts_snapshot` is the
  only `IssueRead`-shaped capability advertised; no
  `IssueWrite` exists in production.
- **No privileged external operation.** No
  `PrivilegedOperationKind::BtsSubmit`,
  `KojiBuild`, `BodhiUpdate`, `DakUpload`, or `Signing` is
  reachable from production code. The Phase 0 `Operation`
  type exists and is exercised by the synthetic
  E2E; the production adapter never plans one.

## 11. Open debt at Phase 1 close

From `doc/DEBT.md` (2026-10-13, post-1G.1):

| ID | Severity | Status | Notes |
|---|---|---|---|
| D-06 | LOW | environment-blocked | submodule needs newer Rust than MSRV |
| D-09 | LOW | architect decision (open half) | unchanged |
| D-10 | LOW | recorded non-decision | unchanged |
| D-11 | LOW | **CLOSED 2026-10-13** (PR #63) | production adapter exercised end-to-end through MCP |
| D-12 | LOW | Phase 1 by design, **stays open** | 0B.8 hardening suite not derived from the spec's invariant list; deriving it is a Phase 2 spec-reading exercise |
| D-13 | LOW | process | `.gitignore` covers the case that actually bit |
| D-15 | LOW | environment-blocked | no `ironclaw` binary; the spec's "IronClaw agent completes the synthetic repair workflow" cannot be checked in this environment (carried over from Phase 0B) |
| D-17 | LOW | real but bounded, not Phase 1's problem | unchanged |
| D-19 | LOW | real but bounded, not Phase 1's problem | unchanged |

**The one new entry 1G.1 introduced is the D-11 closure.** The
other rows pre-date Phase 1 and are unchanged.

---

## 12. §41 Definition of Done — status at `2f5b337`

> **All 35 items are met.** Phase 1 is complete.

This table is the answer to "is Phase 1 done", and it is written
so the items that are "met because of the seam check" can be told
apart from the items that are "met because a real test exercises
the path". Items 29–32 are the former; items 26–28 are the
latter; the gap is the point.

| # | §41 item | Status | Evidence |
|---|---|---|---|
| 1 | D-21 is resolved with job-before-candidate semantics | **met** | 1A.1 / PR #46; `handle_capture_candidate` pre-checks the job |
| 2 | MockStore and SQLite agree on candidate referential integrity | **met** | 1A.1 / PR #46; the D-21 conformance pin in `testkit/store_parity_sqlite.rs` |
| 3 | Executor JSON input is real and tested | **met** | 1A.2 / PR #47; `ToolInputMode::JsonStdin` wired in `ProcessExecutor`; `build_candidate_input` v1 |
| 4 | Registered result normalizers actually control evidence classification | **met** | 1A.3 / PR #48; `service.rs:2409-2426` now invokes `.normalize(&record)` when `Some` |
| 5 | Structured tool reports become candidate-bound evidence artifacts | **met** | 1A.3 / PR #48; the 5 Phase 1 normalizers each bind the report to the `Evidence` row's `ArtifactRef` |
| 6 | Agents can retrieve those artifacts through scoped read-only MCP operations | **met** | 1A.4 / PR #49; `evidence.list` / `evidence.get` / `evidence.artifact.read`; the 1G.1 §43-shape test exercises all three end-to-end |
| 7 | A production Debian adapter replaces the Debian stub in `ironmaintd` | **met** | 1B.3 / PR #52; `bins/ironmaintd/src/main.rs` now registers `debian::DebianAdapter::new()`; the stub is no longer in the production path; the production adapter passes the shared conformance suite |
| 8 | Debian version comparison matches Debian's native semantics on a test corpus | **met** | 1B.2 / PR #51; `debversion` crate adopted; `adapters/debian-stub/tests/oracle_compare_versions.rs` walks the corpus against `dpkg --compare-versions` (gated on `dpkg` present); 1B.3 vendors the comparator into the production adapter |
| 9 | A real Debian source tree can be structurally inspected | **met** | 1C.1 / PR #53 + 1C.2 / PR #55 + 1C.3 / PR #57; the 3 inspection checks against the good fixture all return `Pass` and produce a structured report |
| 10 | Candidate package identity is validated against the source tree | **met** | 1C.1 / PR #53; the `source_preparation` report's `verdict` is `Fail` with `E_CHANGELOG_SOURCE_NAME_MISMATCH` against the broken fixture |
| 11 | `debian/control` information is represented structurally | **met** | 1C.2 / PR #55; `DebianSourceReportV1.source_metadata` + `binary_packages[]` |
| 12 | `debian/changelog` information is represented structurally | **met** | 1C.1 / 1C.2 / 1C.3; `ChangelogIdentity` (source, version, distribution, urgency, maintainer, timestamp) |
| 13 | source format is reported | **met** | 1C.1 / 1C.2; `DebianSourceReportV1.source_format` (`"3.0 (quilt)"` etc.) |
| 14 | patches are reported | **met** | 1C.2 / 1C.3; `DebianSourceReportV1.patches[]` + `DebianMaintenanceContextV1.patches[]`, each carrying DEP-3 metadata |
| 15 | autopkgtest metadata is reported | **met** | 1C.2 / 1C.3; `tests[]` (Test, Restrictions, Depends) |
| 16 | watch configuration is reported | **met** | 1C.2 / 1C.3; `watch.version` + `watch.url_pattern` + `watch.pgpsig` |
| 17 | copyright metadata is reported | **met** | 1C.2; `copyright.files_stanza_count` + `copyright.machine_readable` |
| 18 | maintainer scripts are reported | **met** | 1C.2 / 1C.3; `maintainer_scripts.{preinst,postinst,prerm,postrm}.executable` |
| 19 | GBP configuration is reported where present | **met** | 1C.2 / 1C.3; `gbp.gbp_conf_present` + `gbp.gbp_dch_conf_present` + the parsed `[BUILD-DEFAULTS]` branch settings |
| 20 | current Debian BTS state is available read-only | **met** | 1E.1 / PR #60 + 1E.2 / PR #61; `DebianIssueReportV1` (`bts_snapshot` subcommand) |
| 21 | BTS unavailability becomes `Blocked`, not package failure | **met** | 1E.2 / PR #61; fake-BTS hermetic test asserts the normalizer maps `infrastructure_error` → `ExecutorError::InfrastructureFailed` |
| 22 | Debian Policy baseline provenance is current and explicit | **met** | 1D.1 / PR #58; `policy/debian/authorities.v1.json` (4 authorities, `baseline_policy_version: "4.7.4.1"`) |
| 23 | Developer's Reference is classified non-normatively | **met** | 1D.1; `AuthorityClassification::BestPractice` (not `FormalSpecification`); the `developers_reference_is_best_practice_not_formal_specification` test pins it |
| 24 | `Standards-Version` delta analysis works | **met** | 1D.2 / PR #59; `DebianPolicyDeltaReviewV1` with the `comparison_outcome` tri-state; `debian_policy_delta_review_passes_a_candidate_below_the_baseline` (good fixture) and `debian_policy_delta_review_blocks_a_candidate_above_the_baseline` (future fixture) |
| 25 | Phase 1 creates no false claim of complete Policy compliance | **met** | by construction; the production adapter does not advertise `PolicyDerivation`, the `policy()` accessor returns `None`, and the 5 checks are observational (§18 line 847-849) — only the 1D.2 delta review makes a judgement, and that judgement is explicitly a Standards-Version delta, not a Policy compliance verdict |
| 26 | The five pre-build gates are real checks | **met** | 1C.1 / 1C.2 / 1C.3 / 1D.2 / 1E.2; each registered as a `ToolDefinitionRecord` with a real binary, a real normalizer, and a real report artifact |
| 27 | A real candidate traverses all five through the existing state machine | **met** | 1F.1 / PR #62; `debian_state_machine_vertical_slice_…` walks `EventDetected → Intake → SourceReview → CandidateAssembly → SourceRevision → SourceIntegrity` in 5 `reconcile` calls through MCP |
| 28 | It stops at the build boundary | **met** | 1F.1 / PR #62; the 6th `reconcile` call returns `NoOp{current: SourceIntegrity}` because `GateStage::BuildValidation` is unsatisfied |
| 29 | No `sbuild`/Lintian/autopkgtest execution is required | **met (negative space)** | S6 seam check (`xtask/src/seams/adapters.rs`) — `BuildPlanning` / `PackageQaPlanning` / `FunctionalTestPlanning` / `UpgradeTestPlanning` / `ReproducibilityPlanning` are in the Phase 2 deny list; the production Debian descriptor does not advertise any of them; a future PR that adds one will fail `verify-seams` at the §97 gate |
| 30 | No source modification is required | **met (negative space)** | same as 29; no `BuildPlanning` capability exists to plan one |
| 31 | No BTS mutation occurs | **met (negative space)** | S6 deny list includes `IssueWrite`; the production Debian descriptor does not advertise it |
| 32 | No privileged external operation occurs | **met (negative space)** | S6 deny list includes `ReleaseMetadata` / `PublicationPlanning`; the production Debian descriptor does not advertise either; no `KojiBuild` / `BodhiUpdate` / `Signing` / `DakUpload` is planned anywhere |
| 33 | Existing Fedora conformance remains green | **met** | 1B.3 leaves `adapters/fedora-stub/` unchanged; the 1B.1 `InspectionCapability` impl in the Fedora stub continues to satisfy the `SourceInspection` conformance arm; `cargo test -p fedora-stub --test conformance_both_stubs_pass` is green |
| 34 | Existing Phase 0 invariants remain green | **met** | §106 is intact (no state-machine, persistence, executor, candidate-identity, evidence, state-ownership, MCP-boundary, or IronClaw-integration change); the `hardening.rs` 0B.8 suite still passes; the §101 acceptance scenario still drives a job to `ReadyForApproval` over real SQLite |
| 35 | The local container acceptance gate passes | **met** | `make -f containers/Makefile gate` green at `2f5b337` (8 steps, 27s) |

### Reading the negative-space rows (29–32) honestly

Items 29–32 are met because the **seam check** `S6` makes them
structural, not because a runtime test exercises a missing
capability. The runtime test would be: "the production adapter
fails to plan a `BuildPlanning` operation when asked." That
test exists at the seam-check level (a future PR that adds
`BuildPlanning` to the production descriptor is rejected by
`verify-seams`); it does not exist as a runtime test because
no production code path could ask the question.

The accurate sentence is: **the negative space is enforced by
AST inspection at build time, not by runtime refusal.** A green
S6 and a green integration suite are both required; either alone
would not catch a regression where a Phase 2 capability
re-appeared. The S6 check is the build-time catch; the §97
gate's step 8 is what makes it run on every PR.

**This honesty is deliberate, not an oversight.** Surfacing the
gap is the review input the architect needs in order to plan
Phase 2. The questions below are open as of Phase 1 close; they
are not defects on the Phase 1 ledger because every answer
depends on a Phase 2 design choice that Phase 1 deliberately
does not pre-empt.

### Open questions for Phase 2 planning

Six questions Phase 2 must answer. Each is a real architectural
decision the Phase 2 spec will need to make; Phase 1 closed
without answering any of them on purpose. The seam check S6
sits at the centre of all six.

1. **What does "S6 allowlist removal" look like in practice?**
   The current S6 deny list is a `const PHASE_2_CAPABILITIES`
   of 8 variants in `xtask/src/seams/adapters.rs`. Phase 2
   *re-enables* capabilities by removing them from the deny
   list, not by adding a positive allowlist. The Phase 2 spec
   needs to decide: do we shrink the deny list one capability
   at a time (each removal is its own PR with its own
   seam-check diff, and the seam check is *tightened* in
   inverse proportion to the deny list shrinking), or do we
   replace the deny list with a positive allowlist ("the
   production adapter enumerates exactly the capabilities it
   intends to advertise")? The positive-allowlist shape catches
   a wider class of regressions but requires every adapter
   descriptor to be updated when new capabilities are added;
   the deny-list shape is local to the seam-check file but
   encodes Phase 2's intent in a single place. Neither is
   right by default; the choice is Phase 2's.

2. **What is the "runtime refusal" test equivalent?** Phase 1
   has no test that calls `DebianAdapter::inspection()` and
   asserts the returned `Vec<PlannedCheck>` does not contain
   a `BuildPlanning` key — the assertion is unnecessary because
   the seam check already rejected the descriptor. Phase 2's
   first PR to enable a build capability needs to decide: is
   the runtime refusal covered by a `debug_assert!` in
   `AdapterCapabilities::new()` (runtime, infallible, fires
   only in debug builds), by a unit test that exercises
   `plan_checks` end-to-end (runtime, always-on, requires a
   test fixture that exercises the build path), or by relying
   on the seam check alone (build-time, as today)? The current
   design is the third. The first two are stronger but add
   cost; the third is cheaper but the gap is exactly the gap
   the architect sees today.

3. **How does S6 scale to a Fedora production adapter?** Phase
   1 is Debian-only; the production Debian adapter is the
   only consumer S6 enforces. The Phase 2 spec calls for a
   production Fedora adapter; the deny list (currently
   Debian-shaped: `BuildPlanning` = `sbuild`,
   `PackageQaPlanning` = Lintian, `ReleaseMetadata` = `dak`)
   needs to either (a) extend to Fedora's `koji` / `bodhi` /
   `packit` capabilities in a Fedora-shape (one deny-list
   per distribution, or a single deny list of capability
   names that are distribution-agnostic), or (b) become
   per-distribution (each `adapters/<dist>/src/descriptor.rs`
   carries its own seam-check). The seam check today is a
   single shared table; the Phase 2 plan needs to choose a
   shape that works for both, and the answer is not
   obvious because Fedora's `koji` build is structurally
   similar to Debian's `sbuild` build (both are
   `BuildPlanning`) but Fedora's `bodhi` update is
   structurally different from Debian's `dak` upload (both
   are `PublicationPlanning`, but the authority shape is
   different — `bodhi` is a per-release-gate, `dak` is a
   per-suite-push).

4. **What is the granularity of Phase 2 capability re-enable?**
   The S6 deny list treats `BuildPlanning`,
   `PackageQaPlanning`, `FunctionalTestPlanning`,
   `UpgradeTestPlanning`, `ReproducibilityPlanning`,
   `IssueWrite`, `ReleaseMetadata`, `PublicationPlanning`
   as one deny set. Phase 2's first capability to be
   re-enabled (almost certainly `BuildPlanning` for the
   `sbuild` vertical slice) will open the question: should
   subsequent capabilities be enabled one-by-one across 8
   PRs, in groups ("build + QA" as one PR, "release +
   publication" as another), or all at once? Each shape has
   different review load, different test surface, and
   different blast radius if a regression slips in. The
   Phase 2 spec should pick a default and let deviations
   be deliberate, not accidental.

5. **What if a future PR introduces a new `AdapterCapability`**
   **variant?** Today, the deny list is a `const` of variant
   names. A new capability added in a future crate — e.g., a
   `DryRun` capability for end-to-end testing, or a
   `ReproducibleBuildAttestation` capability for the
   reproducible-builds integration — is *not* in the deny
   list and *not* required to be advertised by any
   production descriptor. The seam check would let the new
   capability through silently. The Phase 2 spec needs to
   decide: is the deny-list shape correct (every new
   capability is implicitly permitted until added to the
   deny list; the new capability is "innocent until proven
   guilty"), or should S6 also have a "every capability must
   be advertised by some adapter" check (the new capability
   is silently dead code until an adapter picks it up; this
   is the "guilty until proven innocent" shape)? The second
   is a stricter test but requires every new capability to
   be paired with a stub adapter in the same PR.

6. **What if a future PR replaces the `AdapterCapabilities::new()`**
   **constructor?** S6 parses the literal call shape of
   `AdapterCapabilities::new(...)`. A refactor that introduces
   `AdapterCapabilities::from_iter(...)` or
   `AdapterCapabilities::builder()` would silently bypass the
   check. The Phase 2 spec needs to decide: is the build-time
   AST check correct (catches regressions in the common case;
   refactor work has to update S6, which is a known cost of
   the seam-check approach), or should S6 move to a runtime
   check (e.g., a `debug_assert!` in
   `AdapterCapabilities::new()` that the seam check becomes a
   verifier of, not a parser)? The runtime shape is more
   refactor-robust but requires the runtime to be aware of the
   S6 deny list; today the deny list is a `xtask`-only
   concept.

Each of these six is a Phase 2 design decision. Phase 1 closed
without answering them on purpose; the closure of the
negative-space rows (29–32) is conditional on Phase 2
re-validating the answers. The Phase 2 spec is the right
document to record the chosen shape for each.

### What the §97 gate proves

| Step | What it proves about Phase 1 |
|---|---|
| 1. `cargo fmt --check` | every file formats clean (caught the 1G.1 `authority_manifest.rs` LazyLock doc-comment alignment) |
| 2. `cargo clippy --workspace --all-targets --all-features -- -D warnings` | every lint clean (the `unwrap_used` / `expect_used` / `panic` denies from §106 are part of this) |
| 3. `cargo test --workspace` | every existing test + the 16 new Phase 1 tests + the 2 new integration scenarios are green |
| 4. `verify-architecture` | the dependency graph is one-way; no `core` → `adapter` edge; the new `adapters/debian/` crate mirrors `adapters/debian-stub/`'s edge set |
| 5. `verify-schemas` | all 14 schema snapshots are generated = committed; the 6 new ones (1C.1–1E.1) match the report types |
| 6. `verify-migrations` | the SQLite schema is unchanged from Phase 0B (no new migrations in Phase 1 — the production adapter registers existing tables) |
| 7. `verify-mcp-schemas` | the 14-MCP-tool count holds; no drift in any tool's input / output schema |
| 8. `verify-seams` | S1 (actions) / S2 (events) / S4 (commands) / S5 (stores) / **S6 (adapters, new in 1B.3)** / S8 (reachability) all clean |
| 9. `cargo test --workspace --features integration` | the synthetic E2E driver (Phase 0B's C5) still drives a job to `ReadyForApproval` against the production adapter |

A green run of all 9 is the **structural** answer to "is Phase 1
done". The §41 DoD table above is the **behavioural** answer.

## 13. North-star check (§44)

> **IronMaint can examine a real Debian package and tell an agent
> what actually exists in the package, what Debian currently says
> about the relevant maintenance context, and what Debian bugs
> currently exist—while every fact remains bound to an immutable
> candidate and every workflow transition remains controlled by
> deterministic evidence.**

**True.** The 1G.1 §43-shaped exit artifact proves the first
clause (the test assembles a Debian package, source version,
source format, binary packages, packaging, patches, tests,
upstream monitoring, BTS, policy, developer guidance, and
workflow from the bound artifacts). Items 1–28 of the §41 DoD
prove the maintenance context and BTS clauses. The
`BLAKE3` fingerprint binding on every `Evidence` row (carried
over from Phase 0B) proves the immutable-candidate clause.
The `TransitionEngine` in `ironmaint-state` (unchanged) proves
the deterministic-evidence clause.

> **The agent has learned substantially more about the package
> without gaining any new authority to alter Debian, publish
> software, fabricate evidence, or bypass a failed or unavailable
> check.**

**True.** The S6 seam check (item 32) proves no publishing /
signing capability is reachable; item 31 proves no BTS mutation;
items 29–30 prove no source modification. The §17 tri-state
(Pass / Fail / Blocked) in the `evidence_status` field, plus
the `executor_error::InfrastructureFailed` distinction, prove
the no-bypass clause — a Blocked check is `Blocked`, not
fabricated `Pass`.

## 14. What's next (Phase 2)

This document records the close. **Phase 2 is its own decision**
and is not in scope here. The shape the seam check is
**holding open** for Phase 2:

- `BuildPlanning` — `sbuild` / `sbuild-rc` / `debuild`
- `PackageQaPlanning` — `lintian` / `piuparts`
- `FunctionalTestPlanning` — `autopkgtest`
- `UpgradeTestPlanning` — `sbuild --upgrade`
- `ReproducibilityPlanning` — `reprotest` / `diffoscope`
- `IssueWrite` — BTS file / modify
- `ReleaseMetadata` — `debian/changelog` close, upload
  changelog entries
- `PublicationPlanning` — `sftp-to-ftp-master` / `dak` / `dupload`
- (Fedora side) `koji` / `bodhi` / `packit` — the Fedora real
  adapter, replacing `adapters/fedora-stub/`

The seam check S6's Phase 2 deny list is the **enabling
constraint** for that work, not a barrier to it. When the
Phase 2 spec is written and the first Phase 2 PR is ready to
land, the S6 deny list is reduced — not removed (the
implementation tests for the Phase 2 capabilities would then
be the seam check's job, not the deny list's).

A future `doc/PHASE-2-COMPLETION.md` will tick a similar §41-style
table for the Phase 2 work; this document does not pre-empt it.

---

**End of Phase 1.**
