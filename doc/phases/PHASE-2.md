# Phase 2 — Real Debian Build and Validation

**Status:** Proposed  
**Baseline:** Phase 1.1 release, commit `6218fc9`  
**Implementation strategy:** One narrow, end-to-end capability at a time

## 1. Objective

Extend IronMaint's production Debian adapter from read-only source inspection into real package build and validation.

The target workflow is:

1. Capture and identify an exact source candidate.
2. Complete the existing pre-build inspection gates.
3. Build that candidate in an isolated Debian build environment.
4. Run package QA and applicable functional, upgrade, and reproducibility tests.
5. Perform release review and final validation.
6. Reach `ReadyForApproval` with durable evidence bound to the exact candidate.

The initial Phase 2 acceptance boundary is `ReadyForApproval`. Human approval remains a human decision. Signing, upload, publication, and write-side BTS operations are not part of the initial acceptance criterion.

## 2. Phase 2.0 — Readiness and architecture

Complete these items before implementing the real build runner.

### 2.0-A. Executor environment isolation

- Clear inherited process environment before adding explicitly approved variables.
- Add explicit, trusted per-tool environment configuration.
- Wire the BTS helper through that configuration rather than relying on ambient variables.
- Keep agent-controlled input out of subprocess environment, executable paths, and arbitrary command arguments.
- Add regression tests proving that an ambient secret is not inherited and that approved tool-specific variables work.

Relevant code: [`ProcessExecutor`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-executor/src/process.rs).

### 2.0-B. Correct the policy checklist data

- Populate the upgrading-checklist manifest with actual relevant entries and provenance.
- Define the interval-selection semantics against the Standards-Version target.
- Test an older candidate against the target and assert the actual returned entries, not merely that the list is nonempty.
- Preserve `InfrastructureError`/`Blocked` when the authority baseline is stale or unusable.

Relevant files: [`policy_delta.rs`](https://github.com/montj2/IronMaint/blob/master/bins/ironmaint-debian-tool/src/policy_delta.rs) and [`upgrading-checklist.v1.json`](https://github.com/montj2/IronMaint/blob/master/policy/debian/upgrading-checklist.v1.json).

### 2.0-C. Lock the build execution model

The executor currently launches local subprocesses; it does not provide a container-worker protocol. The checked-in Compose file also does not yet implement the full multi-container topology described in the container documentation.

**Recommendation:** preserve the local-process executor model for the first build slice. Run `ironmaintd` and its Debian tool wrapper in an execution image containing the Debian tools, while `sbuild` provides the package-build chroot. Do not add an implicit `docker run` per check; that would change the existing execution architecture.

Before coding, document and verify:

- Where `ironmaintd` and `ironmaint-debian-tool` execute.
- Where persistent sbuild buildroots live and how they are provisioned.
- Which Debian suite and architecture are selected for a candidate.
- How the build environment is isolated from agent credentials and unrelated host files.
- Whether user namespaces are sufficient for the required sbuild sandbox, without blanket `--privileged` access.

### 2.0-D. Make the candidate/build boundary explicit

The candidate's captured Git tree is the source identity. The build process must not mutate the maintenance workspace or change the source represented by its evidence.

- Stage the exact candidate tree into a separate, disposable build input.
- Define how a captured Git tree becomes a valid Debian source-package input, including any required upstream orig tarball and `.dsc` metadata. Verify the chosen path against a real package rather than assuming `sbuild` accepts an arbitrary working directory.
- Keep generated build outputs outside the immutable candidate snapshot.
- Bind the build report and all resulting artifacts to the original candidate fingerprint, commit, and tree.

The current frozen-worktree check reads `HEAD^{tree}`; that is not a sufficient detector of ordinary working-tree modifications. Do not use it as the build's write-detection mechanism.

### 2.0-E. Guardrails for later validation stages

Before introducing multiple mandatory checks at one gate stage, fix or explicitly redesign gate aggregation. [`plan_to_materialised`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-runtime/src/check.rs) creates a gate per planned check, while [`check_gates`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-state/src/engine.rs) selects a mandatory gate for each required stage. Add a regression test proving that every mandatory check at a stage is enforced.

Before adding functional, upgrade, and reproducibility planning, also reconcile the declared adapter capability variants with the capabilities the runtime actually invokes. Do not advertise a capability that has no production path.

For remote source ingestion, replace the current string-prefix allowlist with a parsed URL policy and test repeated imports before treating `source.import` as a general-purpose production ingress path. The first build acceptance test can use a pinned local input while that work is completed.

## 3. Phase 2A — BuildValidation vertical slice

This is the first implementation milestone. It should deliver one working build path, not a collection of tool stubs.

### PR 2A.1 — Define the build contract

Add a versioned, structured sbuild report and normalizer.

The report should contain:

- Candidate fingerprint, commit, and tree.
- Source package name and version.
- Target suite and architecture.
- sbuild version and build-environment identity.
- Start/end timestamps, exit information, and classified outcome.
- References and SHA-256 digests for the build log and generated artifacts.
- Stable diagnostics for package failures and infrastructure failures.

Pin the report schema and add tests for successful builds, package build failures, infrastructure failures, malformed output, and missing tools. Unknown failure modes must not be silently classified as package defects.

### PR 2A.2 — Add the production Debian build plan

Implement the real Debian adapter's build capability.

The initial plan contains exactly one mandatory check:

| Field | Value |
|---|---|
| Tool key | `debian.build.sbuild` |
| Evidence kind | `Build` |
| Gate stage | `BuildValidation` |
| Mandatory | Yes |

Return an empty QA plan for this first slice. Register the production tool wrapper with fixed command configuration and bounded execution limits. Narrowly update the architecture-seam allowlist to permit `BuildPlanning`; do not enable unrelated Phase 2 capabilities.

### PR 2A.3 — Execute sbuild against the exact candidate

Implement the wrapper and build-input preparation.

- Use a pinned source-package input with verified hashes.
- Select and record the intended target suite and architecture.
- Provision or reuse a buildroot according to the documented policy.
- Execute sbuild with controlled arguments and environment.
- Capture the complete log and generated `.buildinfo`, `.changes`, and binary package artifacts as applicable.
- Store artifacts through the existing artifact store and verify their digests on read.
- Preserve the original candidate identity even if the disposable build directory changes.

The ordinary local gate must remain hermetic. The real sbuild run should be an opt-in integration test using a pinned package and a real buildroot.

### PR 2A.4 — Prove the complete MCP workflow

Add an end-to-end acceptance scenario using the real daemon path, SQLite, and MCP tools—not a direct `RuntimeService` handle or direct store access.

The scenario should:

1. Create a job.
2. Import or provision the pinned source.
3. Capture the candidate.
4. Run the existing pre-build checks.
5. Read their evidence and confirm the job reaches `SourceIntegrity`.
6. Run `debian.build.sbuild`.
7. Read the structured build report and artifacts through the existing evidence tools.
8. Reconcile and assert the correct transition.

Expected outcomes:

- **Build passes:** the job can advance from `SourceIntegrity` to `BuildValidation`.
- **Package build fails:** the build gate fails and the job does not advance.
- **Build infrastructure fails:** the build gate is blocked; the failure is not misrepresented as a package defect.
- **QA is not yet implemented:** the workflow stops at the `PackageQaValidation` boundary because its mandatory gate is missing. It must not advance merely because no QA check was planned.

## 4. Phase 2B — Package QA

Add QA only after the build artifacts and their handoff contract are stable.

### PR 2B.1 — Lintian

- Add `debian.qa.lintian` as a mandatory package-QA check.
- Run it against the artifacts produced by the recorded build, not a silently rebuilt or different candidate.
- Persist the structured result and log.
- Test pass, package findings, missing tool, malformed output, and infrastructure failure.

### PR 2B.2 — Piuparts

- Add `debian.qa.piuparts` as an advisory/optional check initially.
- Run it in the documented disposable installation environment.
- Preserve installation, removal, and upgrade logs.
- Keep infrastructure problems distinct from package defects.

Before expanding the number of mandatory checks at any stage, close the gate-aggregation issue identified in Phase 2.0-E.

## 5. Phase 2C — Functional and upgrade validation

### PR 2C.1 — Autopkgtest

- Add the production `debian.test.autopkgtest` path.
- Consume the exact package artifacts from the successful build.
- Define how packages without applicable tests are represented. Do not equate “no tests found” with meaningful test coverage.
- Distinguish a failing test from an unavailable testbed, dependency download failure, or runner failure.
- Add the functional-test planning/accessor contract before advertising that capability.

### PR 2C.2 — Upgrade validation

- Define the supported existing-install upgrade scenario.
- Use a disposable, reproducible test environment.
- Record the starting package version, candidate version, repository configuration, and test artifacts.
- Keep unsupported or inapplicable scenarios explicit rather than treating them as passes.

## 6. Phase 2D — Reproducibility and final validation

### PR 2D.1 — Reproducibility

- Run two builds using the documented `reprotest` strategy.
- Compare resulting artifacts and preserve the comparison report.
- Use `diffoscope` for meaningful differences.
- Record the environment and perturbations needed to reproduce the comparison.

### PR 2D.2 — Release review and final validation

- Wire release-review and final-validation checks into the existing state machine.
- Ensure all mandatory evidence belongs to the same active candidate fingerprint.
- Re-run required checks after a candidate change; prior evidence must not authorize advancement of a different candidate.
- Reach `ReadyForApproval` only when all required gates and obligations are satisfied.
- Keep human approval separate from deterministic validation.

## 7. Fedora adapter follow-on

After the Debian execution and evidence contracts are stable, implement the real Fedora adapter as a separate, reviewable track using the existing Fedora tool plan: `mock`, `rpmlint`, and the applicable functional-test tooling.

Do not mix Fedora support into the first Debian sbuild vertical slice. Reuse the shared contracts and acceptance patterns, but retain distribution-specific buildroot, failure-classification, and evidence tests.

## 8. Explicit decisions and non-goals

- **Exceptional-state question (D-09 / §17 Q8):** leave it open for the first build slice. A blocked build gate can represent ordinary build infrastructure failures. Do not add an automatic transition into `InfrastructureBlocked` unless a real failure mode appears that cannot be represented by a gate.
- **External side effects:** no automatic BTS mutation, signing, upload, or publication in the initial Phase 2 acceptance criterion. If BTS writes are included later, give them a separate subphase with explicit authorization, idempotency, and audit requirements.
- **Live IronClaw acceptance:** keep the real-agent integration items environment-blocked until the actual IronClaw binary is used to complete the workflow. Passing the deterministic MCP acceptance scenario is not proof of a live-agent run.
- **State machine:** reuse the existing transitions and gate semantics for Phase 2A. Change the state machine only when a demonstrated requirement or failing acceptance test justifies it.
- **Hermetic gate:** normal tests must not depend on live BTS, Debian archive availability, or a successful real sbuild run. Live integration tests remain explicit opt-ins.

## 9. Phase 2 definition of done

- A real pinned Debian source package builds through the production tool path.
- Build, QA, functional-test, upgrade-test, and reproducibility results are represented by structured, candidate-bound evidence where applicable.
- Logs and outputs are durable artifacts with verifiable digests.
- Package failures and infrastructure failures produce different, tested outcomes.
- The complete supported workflow is driven through MCP tools and reaches `ReadyForApproval` only when required checks pass.
- Candidate changes invalidate the authority of prior evidence.
- The architecture verifier permits only capabilities implemented in production.
- The local gate remains hermetic, and the opt-in real-build acceptance test proves the toolchain and buildroot actually work.

**Recommended implementation order:** Phase 2.0 readiness → BuildValidation → Lintian → Piuparts → Autopkgtest → Upgrade validation → Reproducibility → Final validation → Fedora follow-on.
