# IronMaint Phase 1
## Real Debian Package Intelligence and Read-Only Maintainer Intake

**Specification status:** Initial implementation plan  
**Depends on:** Phase 0A + Phase 0B, current `develop` through PR #45  
**Primary distribution:** Debian  
**Orchestrator:** IronClaw through existing MCP boundary  
**Phase character:** Read-only with respect to package source and external Debian systems  
**Publication:** Out of scope  
**Package modification:** Out of scope except synthetic/test fixtures  
**Build execution:** Out of scope for Phase 1  
**Target end state:** A real Debian source package can traverse the real IronMaint intake/source/issue/policy stages and stop cleanly at the build boundary.

---

# 1. Phase Objective

Phase 1 replaces the synthetic Debian understanding with real Debian package intelligence.

Given a real Debian packaging repository, IronMaint must be able to:

1. identify and validate the Debian source package;
2. parse its packaging metadata;
3. establish that the candidate identity agrees with the source tree;
4. describe its binaries, dependencies, source format, patches, tests, watch configuration, copyright metadata, maintainer scripts and Git/GBP configuration;
5. query the Debian BTS read-only;
6. identify the package's declared `Standards-Version`;
7. compare that declaration to the current Debian Policy baseline;
8. produce a Policy upgrading-checklist delta report with authoritative provenance;
9. expose all of those observations as candidate-bound IronMaint evidence;
10. make those reports readable to the agent through MCP;
11. drive the existing state machine through all read-only stages;
12. stop at the first build-validation requirement without running `sbuild`.

The fundamental acceptance scenario is:

```text
real Debian source tree
        ↓
job.create
        ↓
candidate.capture
        ↓
SourcePreparation
        ↓
SourceAnalysis
        ↓
IssueAnalysis
        ↓
Maintenance
        ↓
PolicyEvaluation
        ↓
SourceIntegrity
        ↓
STOP:
BuildValidation has no Phase-1 build gate
```

Phase 1 therefore proves that the architecture can carry **real distribution knowledge** without yet giving the agent package-building or package-modifying authority.

---

# 2. What Phase 1 Is Not

Phase 1 does not implement:

```text
sbuild
lintian
autopkgtest
piuparts
reprotest
diffoscope
debdiff

uscan-driven upstream updates
GBP upstream import
quilt patch modification
debian/changelog editing

BTS mutation
reportbug submission
BTS control commands

signing
tag2upload
Debian upload
```

The Debian tools container may already contain many of these tools.

Their presence is infrastructure preparation, not authorization to use them in this phase.

---

# 3. Current Runtime Invariants Remain Frozen

Phase 1 must not redesign:

```text
candidate identity
candidate fingerprinting
evidence candidate-binding
JobState graph
TransitionEngine ownership
SQLite/event-store model
artifact CAS model
workspace optimistic concurrency
MCP authentication boundary
IronClaw separation
publication authorization model
```

Phase 1 may make **additive** changes to:

```text
adapter capabilities
tool execution input
result normalization
evidence artifact attachment
read-only MCP queries
Debian-specific crates/tools
```

Those are extension points Phase 0 created for precisely this purpose.

---

# 4. First Architectural Decision: D-21

A `MaintenanceJob` is the outer maintenance transaction.

The required ordering is:

```text
job.create
    ↓
job exists durably
    ↓
workspace exists
    ↓
candidate.capture
```

`candidate.capture` must never create a job implicitly.

Before candidate persistence, the runtime/tool path must verify that the referenced job exists.

Unknown job:

```text
candidate.capture
    ↓
typed NotFound
```

not:

```text
raw SQLite FOREIGN KEY error
```

`MockStore` and `SqliteStore` must enforce compatible semantics.

Tests that use a `SourceCandidate` in isolation should create a lightweight fixture job first rather than relying on MockStore's missing referential integrity.

Candidate fingerprints remain independent of `JobId`.

Do **not** add `JobId` to the fingerprint merely to solve this issue.

---

# 5. Second Architectural Decision: Real Check Input

Phase 0 already defined:

```rust
ExecutionRequest {
    job_id,
    tool_key,
    retry_class,
    input: serde_json::Value,
    ...
}
```

but the process executor currently ignores `input`.

Phase 1 activates that contract.

Add an executor-owned input transport concept such as:

```text
ToolInputMode

None
JsonStdin
```

The mode belongs to the trusted `ToolDefinition`.

IronClaw cannot choose it.

For `JsonStdin`:

```text
Runtime
   ↓
constructs input from durable candidate
   ↓
ProcessExecutor
   ↓
serializes canonical JSON
   ↓
child stdin
```

The agent still invokes only:

```text
check.run(check_id)
```

It cannot provide:

```text
arbitrary argv
arbitrary executable
arbitrary workspace path
arbitrary URL
arbitrary stdin
```

This preserves the executor's central security property.

---

# 6. Candidate Execution Context

The runtime should generate a versioned check context.

Conceptually:

```json
{
  "schema_version": 1,
  "job_id": "...",
  "candidate": {
    "fingerprint": "...",
    "package": {
      "family": "debian",
      "release": "unstable",
      "source_name": "foo",
      "version": "1.2.3-1"
    },
    "repository": "...",
    "commit": "...",
    "tree": "..."
  }
}
```

Do not accept these values from the MCP caller during `check.run`.

They come from the persisted active candidate.

The Debian tool also needs the source directory.

Rather than accepting an arbitrary path, register the tool with the configured IronMaint workspace root and derive:

```text
workspace root
   +
JobId
   →
job source directory
```

using one path-construction implementation shared with `WorkspaceManager`.

No duplicated `<root>/<uuid>/source` string-building contract.

---

# 7. Third Architectural Decision: Activate Result Normalizers

`ToolDefinition::normalizer()` already exists.

Phase 1 makes it authoritative.

The runtime should stop treating every real tool as:

```text
exit 0     → Pass
nonzero    → Fail
```

when a registered normalizer exists.

Instead:

```text
ExecutionRecord
       ↓
trusted ResultNormalizer
       ↓
NormalizedResult
       ↓
EvidenceStatus
observations
structured artifact classification
```

Fallback exit-code behavior remains for synthetic/simple tools.

A Debian network query, for example, can therefore produce:

```text
BTS responded successfully
→ Pass

BTS request timed out
→ InfrastructureError

BTS response malformed
→ InfrastructureError

valid BTS response says package has 12 RC bugs
→ Pass
```

The last case is important.

The purpose of the check is to **obtain the BTS state**, not to declare that “having bugs” means the check execution failed.

---

# 8. Structured Report Artifacts

A real inspection check produces two conceptually different outputs:

```text
diagnostic log
structured report
```

Do not force the agent to scrape stdout.

The normalizer must identify the canonical structured report and bind it to the Evidence record as an `ArtifactRef`.

Suggested artifact characteristics:

```text
kind:
  Report

media type:
  application/vnd.ironmaint.debian.<report-name>+json

digest:
  SHA-256

candidate:
  inherited from Evidence
```

The report bytes live in the existing content-addressed artifact store.

Evidence is the provenance link.

---

# 9. Read-Only Agent Evidence Surface

Synthetic checks only needed a verdict.

Debian maintenance requires the agent to read what was discovered.

Add read-only MCP capabilities approximately equivalent to:

```text
evidence.list
evidence.get
evidence.artifact.read
```

They must not permit arbitrary CAS reads.

An artifact read should be scoped through:

```text
job
   ↓
evidence belonging to job/candidate
   ↓
ArtifactRef belonging to evidence
   ↓
CAS digest
```

not:

```text
"Here is a SHA-256. Give me whatever bytes you have."
```

`evidence.artifact.read` initially supports bounded UTF-8 text / JSON report artifacts only.

Binary package retrieval is not part of Phase 1.

These are queries, not state-changing agent powers.

---

# 10. Debian Adapter Structure

Create the real production Debian adapter.

Preferred final structure:

```text
adapters/
├── debian/
└── fedora-stub/
```

The existing Debian stub may temporarily coexist during implementation, but `ironmaintd` must register exactly one production adapter for family:

```text
debian
```

By Phase 1 completion, production code should no longer call something named `DebianStubAdapter`.

Synthetic adapter behavior belongs in test fixtures.

---

# 11. Debian Adapter Capabilities in Phase 1

The real Phase 1 adapter should advertise only capabilities it actually implements.

Expected:

```text
SourceInspection
VersionComparison
IssueRead
```

Policy-context inspection is also implemented, but formal policy obligation derivation is deliberately limited as described below.

Do **not** advertise yet:

```text
BuildPlanning
PackageQaPlanning
FunctionalTestPlanning
UpgradeTestPlanning
ReproducibilityPlanning
IssueWrite
ReleaseMetadata
PublicationPlanning
```

The important consequence is:

> The Phase 1 adapter does not materialize `debian.build.sbuild`.

That avoids telling `job.next_actions` that the agent can perform a check whose tool is intentionally unavailable in this phase.

---

# 12. Extend the Adapter Contract for Read-Only Intake

The current adapter contract has a build planner but no corresponding source/intake planner despite already having `SourceInspection` in `AdapterCapability`.

Add an optional capability such as:

```rust
trait InspectionCapability {
    fn inspection_plan(
        &self,
        context: &CandidateContext
    ) -> Result<InspectionPlan, AdapterError>;
}
```

And:

```rust
struct InspectionPlan {
    checks: Vec<PlannedCheck>
}
```

`DistributionAdapter` gains:

```text
inspection() -> Option<&dyn InspectionCapability>
```

The runtime aggregates inspection checks during candidate capture exactly as it already aggregates build/QA checks.

The state engine remains untouched.

---

# 13. Required Phase 1 Debian Checks

The production Debian adapter should plan exactly the read-only checks necessary to satisfy the pre-build state graph.

## Source Preparation

```text
debian.inspect.source_preparation
EvidenceKind::SourcePreparation
mandatory
```

Purpose:

```text
confirm Debian packaging tree exists
parse source identity
verify candidate package identity
verify candidate version
identify source format
ensure basic mandatory packaging files parse
```

---

## Source Analysis

```text
debian.inspect.source_analysis
EvidenceKind::SourceIntegrity
mandatory
```

Purpose:

Produce the comprehensive source/package intake report.

---

## Issue Analysis

```text
debian.issue.bts_snapshot
EvidenceKind::IssueCorrelation
mandatory
```

Purpose:

Capture current Debian BTS state for the source package.

---

## Maintenance Context

```text
debian.inspect.maintenance_context
EvidenceKind::Maintenance
mandatory
```

Purpose:

Describe the existing maintenance topology:

```text
patches
tests
watch configuration
GBP configuration
maintainer scripts
upstream relationship metadata
```

This does not modify anything.

---

## Policy Evaluation

```text
debian.policy.delta_review
EvidenceKind::PolicyEvaluation
mandatory
```

Purpose:

Establish the authoritative Policy baseline and generate the package's `Standards-Version` delta report.

---

# 14. Phase 1 State-Machine Exit

After those five checks pass and the caller invokes `job.reconcile`, the existing transition engine should walk:

```text
EventDetected
→ Intake
→ SourceReview
→ CandidateAssembly
→ SourceRevision
→ SourceIntegrity
```

It must then stop.

The next transition requires:

```text
GateStage::BuildValidation
```

and Phase 1 deliberately provides no build-validation gate.

That is the Phase 1 architectural boundary.

A read-only Debian package should **not** reach `ReadyForApproval`.

---

# 15. Real Debian Version Semantics

Delete the stub version comparator.

It currently rejects `~` and performs bytewise comparisons, which is intentionally not Debian behavior.

The real adapter must support Debian version semantics including at minimum:

```text
epoch
upstream version
Debian revision
~
+
:
binNMU-style versions
numeric/non-numeric ordering
```

Do not reimplement Debian version ordering casually.

A current Rust candidate is the `debversion` crate, which is specifically designed for Debian version parsing/comparison and is actively maintained. It should be evaluated against the project's MSRV and dependency requirements before adoption.

Acceptance must compare a corpus against:

```text
dpkg --compare-versions
```

Debian's native implementation remains the reference oracle.

---

# 16. Debian Source Inspection

Create a dedicated deterministic tool executable, conceptually:

```text
ironmaint-debian-tool
```

Subcommands may include:

```text
source-preparation
source-analysis
maintenance-context
policy-delta
```

The executable accepts only the runtime-generated JSON context on stdin.

It emits a versioned JSON report on stdout.

Diagnostics belong on stderr.

Use actual Debian-aware parsers.

Candidate Rust components currently worth evaluating include focused crates for:

```text
Debian versions
debian/control
debian/changelog
debian/copyright
debian/watch
DEP-3 patch metadata
deb822
```

Prefer focused parser crates over pulling a large Debian workspace framework into IronMaint unless the larger dependency materially reduces implementation risk.

---

# 17. Source Preparation Rules

`debian.inspect.source_preparation` should fail when the candidate is not a valid representation of the source tree.

Check at least:

```text
debian/ exists
debian/control parseable
debian/changelog parseable

source name from changelog/control
  ==
SourceCandidate package source_name

current changelog version
  ==
SourceCandidate package version

source format identifiable

mandatory metadata structurally parseable
```

`dpkg-parsechangelog` is useful as an oracle because Debian provides machine-readable output for an unpacked source tree.

A mismatch is:

```text
EvidenceStatus::Fail
```

It is a problem with the candidate/package representation.

A parser binary being missing is:

```text
InfrastructureError
```

Those remain distinct.

---

# 18. Debian Source Report

Define a versioned structured report such as:

```text
DebianSourceReportV1
```

It should contain at minimum:

```text
identity
  source package
  version
  distribution
  urgency

maintainers
  Maintainer
  Uploaders

source metadata
  Standards-Version
  Rules-Requires-Root
  Section
  Priority
  Homepage
  Vcs-Git
  Vcs-Browser

build metadata
  Build-Depends
  Build-Depends-Indep
  Build-Conflicts
  debhelper compatibility

binary packages
  Package
  Architecture
  Multi-Arch
  dependencies
  Provides
  Breaks
  Conflicts
  Replaces

source format
  3.0 (quilt)
  3.0 (native)
  other/legacy

patches
  series ordering
  patch names
  DEP-3 metadata where present

tests
  debian/tests/control
  test names
  Restrictions
  Features
  dependencies

watch
  presence
  format/version
  upstream pattern summary

copyright
  machine-readable / non-machine-readable
  Format declaration
  Files stanza count
  License identifiers

maintainer scripts
  preinst
  postinst
  prerm
  postrm
  triggers
  other recognized scripts

rules
  dh-style detection
  override targets
  executable status

GBP
  config presence
  branch/tag settings where defined

provenance
  candidate fingerprint
  Git commit
  Git tree
  inspector version
```

The report is observational.

It does not claim Policy compliance merely because fields parsed.

---

# 19. Debian Policy Knowledge Baseline

Phase 1 establishes a versioned Debian authority manifest.

Initial current baseline:

```text
Debian Policy Manual
document version: 4.7.4.1
Standards-Version target: 4.7.4

Debian Developer's Reference
version: 14.17
released: 2026-09-26
```

Policy is normative.

The Developer's Reference is not.

Its own scope states that it contains recommended procedures and agreed best practices, not formal policy.

The authority model must therefore represent them differently.

Suggested classification:

```text
Debian Policy
→ NormativePolicy

copyright-format 1.0
→ FormalSpecification

BTS operational protocol
→ DistributionProcedure

Developer's Reference
→ BestPractice / DistributionProcedure
  depending on the referenced material
```

Never classify the Developer's Reference globally as `FormalSpecification`.

---

# 20. Standards-Version Delta Review

The policy tool reads the package's declared:

```text
Standards-Version
```

and compares it to the current supported target.

If the package declares an older version:

```text
4.6.2
```

the report identifies the relevant upgrading-checklist entries through the current target.

Debian explicitly instructs maintainers to review the intervening checklist and make required changes before updating `Standards-Version`.

The report should distinguish:

```text
policy document version: 4.7.4.1

Standards-Version field target:
4.7.4
```

Those are related but not the same string.

---

# 21. Do Not Pretend Phase 1 Performs Complete Policy Compliance

This boundary is important.

Phase 1 produces:

```text
Policy baseline
Policy delta
relevant Policy references
Developer's Reference guidance
mechanically detectable observations
```

It does **not** convert the complete Debian Policy Manual into hundreds of automatically satisfied obligations.

Therefore the real Phase 1 Debian adapter should not copy the current stub behavior of inventing obligations such as:

```text
"binary must be reproducible from source"
"debian/patches must use quilt series"
```

and then passing all of them because one generic policy check returned `Pass`.

That would create false confidence.

Formal package-policy obligation derivation remains a later dedicated phase.

The Phase 1 `PolicyEvaluation` gate means:

> **The authoritative Policy context and upgrade delta were successfully established.**

It does not mean:

> **This package has been proven fully Policy-compliant.**

---

# 22. Policy Baseline Staleness

If IronMaint knows Policy only through:

```text
4.7.4.1
```

and encounters a package declaring a Standards-Version newer than the known baseline, the package must not be marked bad.

The result is:

```text
InfrastructureError / Blocked
```

with a diagnostic equivalent to:

```text
IronMaint's Debian Policy knowledge is stale;
refresh the authority snapshot before evaluating this candidate.
```

This is an inability to evaluate, not a package violation.

PR #45's corrected gate semantics are exactly what makes this safe.

---

# 23. Debian BTS Read-Only Integration

Phase 1 performs no BTS writes.

The check:

```text
debian.issue.bts_snapshot
```

queries open bugs associated with the source package and normalizes them into candidate-bound structured evidence.

Desired fields include:

```text
bug number
subject
severity
tags
status
found versions
fixed versions
forwarded status
owner where present
merged relationships where available
```

Highlight, but do not reinterpret:

```text
critical
grave
serious
security
patch
pending
forwarded
```

The Debian BTS documentation says web access is the primary method, while Debian also ships `python3-debianbts`, an interface to the BTS SOAP service. That package is a strong candidate for the Phase 1 transport rather than hand-rolling Debian's SOAP protocol.

Implementation preference:

```text
fixed helper
+
python3-debianbts
+
canonical JSON output
```

is acceptable behind the deterministic executor boundary.

IronMaint's core remains Rust.

---

# 24. BTS Failure Semantics

These conditions:

```text
DNS failure
TLS failure
timeout
SOAP service unavailable
malformed service response
```

produce:

```text
EvidenceStatus::InfrastructureError
GateStatus::Blocked
```

They do not produce:

```text
Fail
```

A package having 50 open bugs is also **not** a tool failure.

If the snapshot was obtained correctly:

```text
Pass
```

with those 50 bugs represented in the report.

The next layer decides what the observations mean.

---

# 25. No BTS Mutation Yet

Do not expose:

```text
comment
close
severity change
tags
pending
fixed
forwarded
merge
reassign
reportbug
```

in Phase 1.

Debian's control server operations change externally visible bug state and are logged/notify maintainers, so they stay on the privileged side-effect boundary.

---

# 26. Maintenance Context Check

`debian.inspect.maintenance_context` should answer questions an eventual maintainer agent needs before making changes.

Example output:

```text
source format:
  3.0 (quilt)

patch stack:
  4 patches
  all listed in series
  3 have DEP-3 Origin
  2 have Bug-Debian
  1 has Forwarded

tests:
  autopkgtest present

watch:
  present
  upstream monitoring configured

GBP:
  configuration present
  upstream branch = upstream
  pristine-tar = true

maintainer scripts:
  postinst
  prerm

package appears:
  team maintained
```

Again:

```text
observation ≠ compliance verdict
```

---

# 27. Tool Container Changes

Extend `ironmaint/debian-tools` only with dependencies Phase 1 actually needs.

Potential additions:

```text
python3-debianbts
focused runtime dependencies for Debian inspection
```

Do not use Phase 1 as a reason to enable privileged `sbuild`.

The current container architecture/smoke/gate work should remain intact.

Any added Phase 1 tool must receive a smoke assertion.

Every new smoke assertion must be teeth-tested once.

---

# 28. Update the IronClaw Maintainer Skill

The current skill still contains Phase 0-era assumptions in places.

Phase 1 should make its loop:

```text
job.next_actions
      ↓
run read-only Debian check
      ↓
check.run returns evidence_id
      ↓
evidence.get
      ↓
read structured report artifact
      ↓
understand package
      ↓
continue/reconcile
```

The skill must explicitly say:

```text
Pass means the check successfully established its stated fact.

It does not necessarily mean the package is "good."

A BTS snapshot with serious bugs can Pass.

A Policy-delta report identifying required review can Pass.

Blocked means the check could not establish its fact.

Fail means the candidate itself violated the check contract.
```

This distinction becomes central once observations are richer than synthetic booleans.

---

# 29. Phase 1 Test Fixtures

Use two levels of fixtures.

## Hermetic Fixtures

Checked into the repository.

At least:

```text
one simple Debian packaging tree

one richer 3.0 (quilt) tree containing:
  patches
  tests
  watch
  machine-readable copyright
  multiple binary stanzas
```

These drive mandatory CI/local-gate tests without external network dependency.

---

## Live Integration Fixture

Provide an opt-in test that obtains a pinned real Debian source package/version.

It should verify the inspector against Debian's own tooling.

The package/version and source hashes must be pinned.

Do not make the normal §97 gate depend on the external archive remaining reachable.

---

# 30. Oracle Tests

Where Debian already has the authoritative implementation, test against it.

Examples:

```text
IronMaint version comparison
vs
dpkg --compare-versions

IronMaint changelog identity
vs
dpkg-parsechangelog

IronMaint source metadata
vs
Debian control-file semantics

IronMaint source-format result
vs
dpkg-source behavior where practical
```

The point is not to independently recreate Debian semantics and then test our implementation against our own interpretation.

---

# 31. Phase 1A — Handoff and Runtime Hardening

Implement:

```text
close D-21

make job-before-candidate invariant explicit

fix MockStore/SQLite candidate referential parity

update stale README / CLAUDE.md / SKILL.md / daemon comments

set workspace project metadata to Phase 1

activate JsonStdin execution input

activate registered ResultNormalizer

bind normalized report artifacts into Evidence

add candidate/job-scoped read-only evidence MCP tools
```

### Exit checkpoint

A synthetic structured check can:

```text
receive runtime-generated candidate context
emit JSON
normalize it
store it as a report artifact
bind the artifact to Evidence
return evidence_id
have an MCP client read the report
```

No Debian logic yet.

This is the most important precondition for the real adapter.

---

# 32. Phase 1B — Real Debian Adapter Foundation

Implement:

```text
production adapters/debian

real Debian version semantics

real package-name validation

InspectionCapability

read-only InspectionPlan

production daemon registration
```

The real adapter must **not** yet advertise build/QA capabilities.

### Exit checkpoint

Capturing a Debian candidate materializes exactly the Phase 1 read-only checks and no synthetic or future build checks.

Fedora stub remains green against the adapter conformance suite.

---

# 33. Phase 1C — Debian Source Intelligence

Implement:

```text
source-preparation check
source-analysis check
maintenance-context check
Debian report schemas
parser/oracle tests
```

### Exit checkpoint

Given the hermetic rich Debian fixture, the three checks produce candidate-bound deterministic JSON reports whose key fields agree with Debian-native parsing tools.

No source files change.

---

# 34. Phase 1D — Debian Policy Context

Implement:

```text
authority manifest
Policy 4.7.4.1 baseline
Standards-Version target 4.7.4
Developer's Reference 14.17 provenance
upgrading-checklist delta model
policy-delta check
```

Do not implement comprehensive Policy obligations.

### Exit checkpoint

For fixtures declaring different Standards-Version values, IronMaint produces the correct version interval and source references.

A declared version newer than the known baseline blocks as an inability to evaluate rather than failing the package.

---

# 35. Phase 1E — Debian BTS Read-Only

Implement:

```text
BTS client/helper
IssueSnapshot normalization
candidate-bound BTS report
fake-BTS hermetic integration test
optional live BTS integration test
```

### Exit checkpoint

A real source package's current bugs can be retrieved and represented without performing any BTS mutation.

A simulated BTS outage yields `InfrastructureError → Blocked`.

---

# 36. Phase 1F — State-Machine Vertical Slice

Run the real workflow using the real Debian adapter.

Required path:

```text
create job
capture real Debian candidate

run SourcePreparation
PASS

reconcile
→ Intake

run SourceAnalysis
PASS

reconcile
→ SourceReview

run BTS IssueAnalysis
PASS

reconcile
→ CandidateAssembly

run MaintenanceContext
PASS

reconcile
→ SourceRevision

run PolicyEvaluation
PASS

reconcile
→ SourceIntegrity

reconcile again
→ BLOCKED:
missing BuildValidation gate
```

This should happen through:

```text
MCP
→ Runtime
→ Executor
→ real Debian tools
→ Evidence
→ TransitionEngine
```

No privileged shortcut.

---

# 37. Phase 1G — Agent Intake Scenario

Run an MCP client—and where the IronClaw environment is available, IronClaw itself—against the real Debian fixture.

The agent should be able to produce a maintainer intake summary based only on trusted evidence artifacts.

Expected summary categories:

```text
package identity

current version

binary packages

source format

build/dependency shape

patch stack

test coverage

watch/upstream configuration

copyright/licensing metadata

open Debian bugs

Policy baseline

Standards-Version delta

notable maintainer guidance

next authoritative workflow boundary
```

The agent may explain the findings.

It may not create its own evidence.

---

# 38. Required Phase 1 Failure Scenarios

Before declaring Phase 1 complete, deliberately exercise:

```text
candidate package name ≠ debian/changelog
→ Fail

candidate version ≠ changelog version
→ Fail

malformed debian/control
→ Fail

malformed debian/changelog
→ Fail

unknown/newer Standards-Version
→ Blocked, baseline stale

BTS unavailable
→ Blocked

BTS returns severe bugs
→ Pass with severe bug observations

structured report malformed
→ InfrastructureError / contract failure

missing Debian helper binary
→ InfrastructureError

old candidate's inspection evidence
→ cannot satisfy new candidate
```

Each must have a test proving the expected distinction.

---

# 39. Phase 1 Seam Verification

Extend `verify-seams` so that:

```text
every planned Debian capability
→ registered tool

every registered Phase 1 tool
→ dispatcher/executor reachable

every structured report tool
→ registered normalizer

every EvidenceKind required by the read-only transition path
→ planned by the Debian adapter

no Phase 2 build capability
→ accidentally registered/planned
```

Continue the project's current rule:

> Every new checker must be deliberately broken once before it is trusted.

---

# 40. Local Acceptance Gate

The existing local container gate remains the authoritative developer gate.

By Phase 1 completion it should validate:

```text
fmt
clippy
unit tests
integration tests
architecture
schemas
migrations
MCP schemas
seams
Debian report schemas
Debian hermetic intake scenario
```

A separate opt-in target may run network-dependent Debian/BTS integration.

Do not silently turn the local-gate decision into hosted CI.

---

# 41. Phase 1 Definition of Done

Phase 1 is complete when all are true:

1. D-21 is resolved with job-before-candidate semantics.
2. MockStore and SQLite agree on candidate referential integrity.
3. Executor JSON input is real and tested.
4. Registered result normalizers actually control evidence classification.
5. Structured tool reports become candidate-bound evidence artifacts.
6. Agents can retrieve those artifacts through scoped read-only MCP operations.
7. A production Debian adapter replaces the Debian stub in `ironmaintd`.
8. Debian version comparison matches Debian's native semantics on a test corpus.
9. A real Debian source tree can be structurally inspected.
10. Candidate package identity is validated against the source tree.
11. `debian/control` information is represented structurally.
12. `debian/changelog` information is represented structurally.
13. source format is reported.
14. patches are reported.
15. autopkgtest metadata is reported.
16. watch configuration is reported.
17. copyright metadata is reported.
18. maintainer scripts are reported.
19. GBP configuration is reported where present.
20. current Debian BTS state is available read-only.
21. BTS unavailability becomes `Blocked`, not package failure.
22. Debian Policy baseline provenance is current and explicit.
23. Developer's Reference is classified non-normatively.
24. `Standards-Version` delta analysis works.
25. Phase 1 creates no false claim of complete Policy compliance.
26. The five pre-build gates are real checks.
27. A real candidate traverses all five through the existing state machine.
28. It stops at the build boundary.
29. No `sbuild`/Lintian/autopkgtest execution is required.
30. No source modification is required.
31. No BTS mutation occurs.
32. No privileged external operation occurs.
33. Existing Fedora conformance remains green.
34. Existing Phase 0 invariants remain green.
35. The local container acceptance gate passes.

---

# 42. Recommended PR / Agent Sequence

Do not hand one Codex agent all of Phase 1.

Use independent reviewable increments:

```text
1A.1  close D-21 and clean Phase-0 handoff prose

1A.2  activate structured executor input

1A.3  activate normalizers + evidence report artifacts

1A.4  expose scoped evidence-read MCP surface

1B.1  add InspectionCapability contract

1B.2  replace Debian version stub with real semantics

1B.3  install production Debian adapter

1C.1  source-preparation tool

1C.2  comprehensive source-analysis report

1C.3  maintenance-context report

1D.1  Debian authority baseline

1D.2  Standards-Version delta evaluator

1E.1  BTS read-only transport

1E.2  BTS normalization and outage behavior

1F.1  real Debian state-machine vertical slice

1G.1  agent-facing skill update + maintainer intake scenario
```

Every PR should state:

```text
the contract it implements
the invariant it preserves
the deliberately broken test/check proving the guard has teeth
the exact §97/gate results
```

---

# 43. Phase 1 Exit Artifact

At completion, IronMaint should be able to produce an evidence-backed package intake resembling:

```text
Debian package: foo
Candidate: <fingerprint>
Source version: 1.4.2-3
Source format: 3.0 (quilt)

Binary packages:
  foo
  foo-data

Packaging:
  debhelper-compat 13
  Standards-Version 4.6.2
  Rules-Requires-Root: no

Patches:
  4 patches
  3 with DEP-3 Origin
  1 forwarded upstream

Tests:
  autopkgtest present
  3 tests

Upstream monitoring:
  debian/watch present

BTS:
  12 open
  1 serious
  2 patch-tagged
  1 forwarded

Policy:
  evaluated against Debian Policy 4.7.4.1
  declared Standards-Version 4.6.2
  upgrade review required through 4.7.4
  relevant checklist entries: [...]

Developer guidance:
  Developer's Reference 14.17
  advisory, not normative

Workflow:
  read-only intake complete
  next unimplemented stage: BuildValidation
```

Every substantive statement in that report must be traceable to:

```text
candidate
check
evidence
artifact
producer
authority/version
```

—not to the LLM's memory.

---

# 44. Phase 1 North-Star Requirement

At the end of Phase 1, this statement must be true:

> **IronMaint can examine a real Debian package and tell an agent what actually exists in the package, what Debian currently says about the relevant maintenance context, and what Debian bugs currently exist—while every fact remains bound to an immutable candidate and every workflow transition remains controlled by deterministic evidence.**

And this must also be true:

> **The agent has learned substantially more about the package without gaining any new authority to alter Debian, publish software, fabricate evidence, or bypass a failed or unavailable check.**
