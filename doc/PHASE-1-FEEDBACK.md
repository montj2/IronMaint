# IronMaint Phase 1 review

I would not sign off Phase 1 as complete yet. The architecture is progressing well, and the completion report is unusually useful: it identifies the PR sequence, tests, invariants, and open design questions. But reviewing the implementation against its own Phase 1 specification exposed several gaps that the hermetic acceptance tests don't catch.

I reviewed `master` at `21b76ac`, following the Phase 1 release merge, and compared the completion report with the production adapter, runtime, workspace, Debian tools, and Phase 1 specification. I have not executed the local container gate myself, so the findings below are from code and specification review, not a fresh test run.

## 1. Blocking findings

P1 — Candidate versions are still hard-coded to `0+ironmaint`

[`capture.rs`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-workspace/src/capture.rs#L80-L105) constructs every candidate with version `0+ironmaint`. The production [`candidate.capture`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-mcp/src/tools/candidate.rs#L1-L20)[ input](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-mcp/src/tools/candidate.rs#L1-L20) doesn't accept a version, and the [`source-preparation`](https://github.com/montj2/IronMaint/blob/master/bins/ironmaint-debian-tool/src/source_preparation.rs#L145-L230)[ check](https://github.com/montj2/IronMaint/blob/master/bins/ironmaint-debian-tool/src/source_preparation.rs#L145-L230) compares the changelog version against the candidate version.

The good fixture deliberately declares `0+ironmaint` in [`debian/changelog`](https://github.com/montj2/IronMaint/blob/master/containers/fixtures/debian/example-1.0/debian/changelog#L1-L10), so it passes. A normal Debian package with a changelog version such as `1.4.2-3` will fail this identity check through the production MCP path.

Why it matters: Phase 1's headline capability is inspecting a real Debian package. The current acceptance scenario proves that the pipeline works for a specially prepared fixture, not that an ordinary package can enter it successfully.

Required correction: Resolve the package version before constructing the immutable candidate fingerprint, or require the caller to supply an expected version and independently verify it against `debian/changelog`. Add a test using a normal Debian version, not the sentinel.

P1 — Evidence can describe a different tree from the candidate it claims to represent

[`handle_run_check`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-runtime/src/service.rs#L2521-L2610) verifies that the check belongs to the active candidate. However, [`build_candidate_input`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-runtime/src/service.rs#L3168-L3224) passes the tool the current mutable workspace path. It does not verify that the workspace is clean or that its current Git tree matches the candidate's captured tree.

Meanwhile, [`workspace.apply_patch`](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-workspace/src/apply.rs#L23-L68) can modify the workspace after capture. The check can therefore read changed files while its report and evidence remain bound to the old fingerprint.

Why it matters: This violates the Phase 1 north-star invariant that facts are bound to an immutable candidate. It also becomes dangerous in Phase 2: a build could compile different source from the source identified in the evidence ledger.

Required correction: Make checks operate on an immutable candidate snapshot, or enforce a verified snapshot/tree identity and prevent concurrent workspace mutation throughout execution. Merely checking the `dirty` flag once would still leave a race unless the workspace is locked or isolated.

P1 — The BTS check doesn't actually query the Debian BTS

The [`bts_snapshot::run`](https://github.com/montj2/IronMaint/blob/master/bins/ironmaint-debian-tool/src/bts_snapshot.rs#L100-L145) function returns a hard-coded three-bug fixture. [`main.rs`](https://github.com/montj2/IronMaint/blob/master/bins/ironmaint-debian-tool/src/main.rs#L206-L220) calls that function directly; it never invokes the Python BTS helper.

The helper's live path also appears broken. It calls `get_bugs(source_package)` and treats the results as dictionaries. The documented `python-debianbts` API returns bug numbers from `get_bugs`; `get_status` is the call that retrieves the detailed bug records.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://wiki.debian.org\&sz=32)

Debian Wiki

+1



Why it matters: The completion report marks “current Debian BTS state is available read-only” as met, but the production tool currently reports fixture data instead of the package's actual bugs.

Required correction: Wire the live helper into the production tool, fix its use of `get_bugs` and `get_status`, and test against a fake service that returns the real library's documented response shape. Keep the deterministic fixture for hermetic tests, but make the fixture/live distinction explicit in configuration and evidence.

P1 — Policy delta evaluation has incorrect baseline and failure semantics

The manifest stores `baseline_policy_version: "4.7.4.1"`, and [`policy_delta.rs`](https://github.com/montj2/IronMaint/blob/master/bins/ironmaint-debian-tool/src/policy_delta.rs#L155-L245) compares the package's `Standards-Version` against that value. But the Phase 1 specification explicitly distinguishes the Policy Manual document version, 4.7.4.1, from the `Standards-Version` target, 4.7.4. Debian's upgrading checklist confirms that the checklist entries correspond to Standards-Version values such as 4.7.4.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://www.debian.org\&sz=32)

Debian Policy Manual v4.7.4.1



There's a second, more consequential defect: when the candidate declares a version above the known baseline, the code emits `Verdict::Fail`. The specification's §22 says this must be `InfrastructureError / Blocked` because IronMaint's knowledge is stale—not because the package has violated policy. The report's wording says the gate is blocked, but the actual verdict tells the state machine that evaluation ran and the candidate failed.

Finally, the current report only compares version strings. It doesn't identify the intervening upgrading-checklist entries that §20 promised.

Required correction:

- Separate `policy_document_version` from `standards_version_target`.
- Make a stale knowledge baseline yield `InfrastructureError → Blocked`, not `Fail`.
- Include the relevant upgrading-checklist entries and their Policy section references in the delta report.
- Add tests that distinguish a current Standards-Version, an older one, a newer one, and a genuinely malformed value.

These are not cosmetic findings. They affect whether the system is inspecting the intended package, whether its evidence is trustworthy, and whether the agent responds correctly when its policy knowledge is insufficient.

## 2. Additional gaps in the completion report

| Priority | Finding                                                                                                                                                                                                                                                                                                                                                                                  | What needs to happen                                                                                                                                                                          |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P2       | The required pinned real-package integration fixture isn't present in the reported test inventory. The Phase 1 spec's [Live Integration Fixture section](https://github.com/montj2/IronMaint/blob/master/doc/phases/PHASE-1.md#L1245-L1265) calls for a pinned Debian source package and source hashes. The current tests rely on hand-crafted fixtures, including the version sentinel. | Add an opt-in test against a pinned real source package and compare important fields against Debian-native tools. Keep the normal gate hermetic.                                              |
| P2       | The DoD says MockStore and SQLite agree on candidate referential integrity. But the [D-21 implementation](https://github.com/montj2/IronMaint/blob/master/crates/ironmaint-runtime/src/service.rs#L1725-L1765) adds a runtime pre-check; it does not make MockStore enforce the same store-level constraint. The debt discussion acknowledges this distinction.                          | Either implement store-level parity or revise the DoD to say that the production runtime presents consistent behavior through the MCP boundary.                                               |
| P2       | The normal MCP workflow doesn't fetch or import the repository URL. `candidate.capture` captures the existing per-job workspace; workspace creation initializes a Git repository, rather than importing an arbitrary package.                                                                                                                                                            | Document source pre-provisioning as an explicit operator responsibility, or design a controlled source-import capability. Don't imply that the URL alone causes a real package to be fetched. |

The completion report itself is a good starting point, but its statement that all 35 DoD items are met is too strong given the findings above. In particular, the current BTS implementation and candidate-version behavior directly contradict that conclusion. See the [DoD table](https://github.com/montj2/IronMaint/blob/master/doc/PHASE-1-COMPLETION.md#L421-L455).

## 3. What is genuinely solid

The findings don't invalidate the overall architecture. Several important pieces are in place:

- The production Debian adapter is separate from the conformance stub.
- The runtime builds tool input from persisted candidate data rather than accepting arbitrary executable arguments from the agent.
- Structured reports are normalized and bound to evidence artifacts that agents can retrieve through MCP.
- The state machine is exercised through the MCP surface, rather than being tested only through direct runtime calls.
- The distinction between `Fail` and `Blocked` is part of the design, and the S6 seam check provides a useful guard against accidentally enabling Phase 2 capabilities.
- The completion report explicitly acknowledges the limits of its negative-space checks and the unresolved IronClaw integration debt.

Those are good foundations. The problem is that the acceptance scenario proves the control flow using fixtures that mask several missing production semantics.

## 4. Recommended order of work

I would do a small Phase 1.1 corrective pass before freezing the Phase 2 baseline.

1. Candidate identity and evidence integrity. Resolve the actual package version and ensure every check reads the exact source tree represented by its candidate fingerprint. These are prerequisites for trusting any build result.
2. Real BTS integration. Wire the production query path and correct the API use. Test both a successful snapshot and a genuine transport failure.
3. Policy baseline correctness. Separate the two version concepts, correct the stale-baseline verdict, and produce an actual upgrading-checklist delta report with provenance.
4. Acceptance and documentation correction. Add the pinned real-package test, resolve or accurately restate the MockStore parity item, update the completion report, and rerun `make -f containers/Makefile gate` on the final tree.

The Phase 1 report also correctly carries the unresolved D-06/D-15 debt: the MCP workflow has been exercised, but the actual IronClaw binary and extension installation have not been round-tripped in this environment. That limitation should remain explicit in the Phase 2 plan.

