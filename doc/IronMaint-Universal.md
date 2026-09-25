# IronMaint
## Agentic Linux Distribution Package Maintenance and Release Orchestration

**Status:** Architecture baseline / initial project charter  
**Version:** 0.2  
**Working name:** IronMaint  
**Primary orchestrator:** IronClaw  
**Core implementation:** Rust  
**Integration model:** MCP + typed deterministic tools  
**Initial distribution:** Debian  
**Second distribution:** Fedora  
**Long-term model:** Distribution-neutral maintenance core with policy-aware distribution adapters

---

# 1. Project Vision

IronMaint will be an agentic Linux distribution package-maintenance framework.

Its purpose is not merely to build packages.

Its purpose is to perform much of the ongoing work of a competent distribution maintainer:

- monitor upstream projects;
- monitor distribution bugs and maintenance events;
- understand distribution packaging policy;
- modify packaging;
- update packages to new upstream releases;
- maintain downstream patches;
- diagnose build failures;
- repair packaging;
- evaluate package-policy requirements;
- execute distribution-native tests;
- coordinate bug tracking;
- produce auditable release candidates;
- and safely transition validated package changes into the distribution's native release infrastructure.

The first implementation will target Debian.

The architecture will, however, be distribution-neutral from its first commit.

Fedora will be the second supported distribution and will validate that the abstraction is real rather than theoretical.

The fundamental invariant is:

> **Agents may investigate, reason, modify, and repair. Only deterministic gates and distribution-native authorities may advance package state.**

IronMaint must never release software merely because an LLM believes it is correct.

---

# 2. Why Distribution-Neutral From Day One

Debian and Fedora have substantially different packaging ecosystems.

Debian centers maintenance around mechanisms including:

```text
debian/*
dpkg
debhelper
git-buildpackage
sbuild
lintian
autopkgtest
piuparts
Debian BTS
Salsa CI
Debian Policy
tag2upload / Debian archive
```

Fedora centers maintenance around:

```text
RPM spec files
dist-git
fedpkg
rpmbuild
Mock
rpmlint
Packit
Testing Farm / TMT
Koji
Bugzilla
Bodhi
Fedora Packaging Guidelines
Fedora release infrastructure
```

Attempting to hide those differences behind generic shell commands would create a weak abstraction.

Instead IronMaint will separate:

```text
             IronMaint Core
                   │
                   ▼
        Distribution Contract
                   │
         ┌─────────┴─────────┐
         ▼                   ▼
     Debian Adapter      Fedora Adapter
```

The core understands concepts such as:

```text
Package
Maintenance Event
Source Candidate
Issue
Policy Obligation
Evidence
Gate
Build
Test
Release Candidate
Publication Plan
```

It does **not** inherently understand:

```text
Standards-Version
debian/control
BTS Closes:

or

.spec
Koji tag
Bodhi karma
```

Those belong to distribution adapters.

---

# 3. Current Authority Baselines

## Debian

The current Debian Policy Manual is **4.7.4.1**, released March 31, 2026. Debian describes Policy as defining technical requirements packages must meet for inclusion. :chatgpt-content-reference{index="1"}

The current Debian Developer's Reference is **14.16**, released September 16, 2026. It describes recommended procedures and resources for maintainers rather than replacing Debian Policy. :chatgpt-content-reference{index="2"}

Debian explicitly expects maintainers increasing `Standards-Version` to review the relevant upgrading checklist and make necessary changes before changing the field. :chatgpt-content-reference{index="3"}

## Fedora

Fedora's Packaging Guidelines and Package Review Guidelines form the principal packaging-rule corpus, with the Fedora Packaging Committee responsible for packaging guidelines and required exemptions. :chatgpt-content-reference{index="4"}

Fedora's package-review model makes an especially useful distinction for IronMaint:

```text
MUST   → blocking requirement
SHOULD → expected guidance
```

A failed MUST requirement is considered a package-review blocker. :chatgpt-content-reference{index="5"}

Fedora's current CI architecture uses Packit for Fedora dist-git CI by default. Tests continue to run through Fedora's existing testing infrastructure such as Testing Farm. :chatgpt-content-reference{index="6"}

Packit can also automate the release path:

```text
pull_from_upstream
       ↓
Koji build
       ↓
Bodhi update
```

through its `pull_from_upstream`, `koji_build`, and `bodhi_update` jobs. :chatgpt-content-reference{index="7"}

---

# 4. Core Trust Model

IronMaint consists of four trust domains.

```text
┌─────────────────────────────────────────────┐
│                AGENT DOMAIN                 │
│                                             │
│ IronClaw reasoning                          │
│ planning                                    │
│ diagnosis                                   │
│ code/package modification                   │
│ documentation consultation                  │
└─────────────────────┬───────────────────────┘
                      │ proposals
                      ▼
┌─────────────────────────────────────────────┐
│             DETERMINISTIC DOMAIN            │
│                                             │
│ state machine                               │
│ evidence validation                         │
│ policy obligations                          │
│ build/test gating                           │
│ evidence invalidation                       │
│ release assembly                            │
└─────────────────────┬───────────────────────┘
                      │ validated candidate
                      ▼
┌─────────────────────────────────────────────┐
│          DISTRIBUTION AUTHORITY DOMAIN      │
│                                             │
│ Debian archive / BTS / Salsa                │
│ Fedora Koji / Bodhi / Bugzilla / gating     │
└─────────────────────┬───────────────────────┘
                      │ privileged action
                      ▼
┌─────────────────────────────────────────────┐
│              HUMAN AUTHORITY                │
│                                             │
│ approval                                    │
│ credentials                                 │
│ signing                                     │
│ judgment-sensitive exceptions               │
└─────────────────────────────────────────────┘
```

An LLM cannot override a lower trust boundary.

---

# 5. Major Design Principle

The system should be **agentic around deterministic infrastructure**, not agentic instead of deterministic infrastructure.

Bad design:

```text
LLM:
"I examined the RPM and it seems Fedora compliant."

→ release
```

Good design:

```text
LLM modifies package
        ↓
native package parser
        ↓
policy obligations
        ↓
Mock / sbuild
        ↓
distribution QA
        ↓
release infrastructure
        ↓
validated evidence
        ↓
state machine permits progression
```

---

# 6. Distribution-Neutral Core Domain Model

The initial Rust domain model should revolve around a relatively small number of stable entities.

## DistributionId

```text
DistributionId {
    family
    release
    architecture_set
}
```

Examples:

```text
debian:unstable
debian:trixie
fedora:rawhide
fedora:45
```

---

## PackageIdentity

```text
PackageIdentity {
    distribution
    source_name
    version
    repository
}
```

Distribution adapters may attach additional metadata.

---

## MaintenanceEvent

Something causes maintenance work to begin.

```text
MaintenanceEvent {
    id
    package
    type
    source
    detected_at
    metadata
}
```

Possible event types:

```text
UPSTREAM_RELEASE
BUG_REPORTED
BUILD_FAILURE
TEST_FAILURE
POLICY_CHANGE
SECURITY_ADVISORY
DEPENDENCY_TRANSITION
MANUAL_REQUEST
RELEASE_TRANSITION
```

---

## MaintenanceJob

Represents one durable package-maintenance transaction.

```text
MaintenanceJob {
    id
    package
    initiating_event
    base_source
    workspace
    current_state
    active_candidate
    obligations
    evidence
    issues
    audit_log
}
```

---

## SourceCandidate

Represents an exact proposed source state.

```text
SourceCandidate {
    id
    job_id
    repository
    commit
    tree_hash
    package_version
    created_at
}
```

A candidate is immutable.

Any source modification creates a new candidate identity.

---

## Obligation

A policy or procedural requirement.

```text
Obligation {
    id
    authority
    authority_version
    reference
    strength
    applicability
    requirement
    evidence
    status
}
```

Strength examples:

```text
MANDATORY
RECOMMENDED
BEST_PRACTICE
PROCEDURAL
LEGAL_REVIEW
LOCAL_POLICY
```

Statuses:

```text
NOT_EVALUATED
NOT_APPLICABLE
PASS
FAIL
REQUIRES_REVIEW
EXCEPTION_APPROVED
```

---

## Gate

A deterministic requirement for state progression.

```text
Gate {
    id
    gate_type
    candidate
    requirements
    result
}
```

Examples:

```text
BUILD
STATIC_PACKAGE_QA
INSTALL_TEST
UPGRADE_TEST
FUNCTIONAL_TEST
REPRODUCIBILITY
POLICY
ISSUE_CORRELATION
RELEASE_ASSEMBLY
```

---

## Evidence

Everything important is evidence-backed.

```text
Evidence {
    id
    candidate_id
    producer
    producer_version
    environment
    result
    artifact_hashes
    timestamp
    freshness_scope
}
```

Evidence is tied to an exact source candidate.

---

## Issue

Generic representation of an external maintenance issue.

```text
Issue {
    provider
    external_id
    package
    title
    status
    severity
    tags
    relationships
}
```

Examples:

```text
Debian BTS issue
Fedora Bugzilla issue
upstream GitHub issue
upstream GitLab issue
```

---

## IssueAction

A proposed or authorized external operation.

```text
IssueAction {
    issue
    operation
    proposed_state
    rationale
    evidence
    authorization_state
}
```

---

## ReleaseCandidate

```text
ReleaseCandidate {
    package
    source_candidate
    obligations
    gates
    issue_actions
    release_metadata
    manifest_hash
}
```

---

## PublicationPlan

Distribution-specific publication instructions.

```text
PublicationPlan {
    distribution
    release_candidate
    operations
    privileges_required
    approvals_required
}
```

Examples:

Debian:

```text
signed tag
git-debpush
```

Fedora:

```text
dist-git merge
Koji build
Bodhi update
```

---

# 7. Core State Machine

The generic lifecycle should avoid Debian- or Fedora-specific terminology.

```text
EVENT_DETECTED
       ↓
INTAKE
       ↓
SOURCE_PREPARATION
       ↓
SOURCE_ANALYSIS
       ↓
ISSUE_ANALYSIS
       ↓
MAINTENANCE
       ↓
POLICY_EVALUATION
       ↓
BUILD_VALIDATION
       ↓
PACKAGE_QA
       ↓
FUNCTIONAL_VALIDATION
       ↓
UPGRADE_VALIDATION
       ↓
RELEASE_REVIEW
       ↓
CANDIDATE_ASSEMBLY
       ↓
FINAL_VALIDATION
       ↓
READY_FOR_APPROVAL
       ↓
APPROVED
       ↓
PUBLICATION_PENDING
       ↓
PUBLISHED
```

Failure branches include:

```text
SOURCE_FAILED
POLICY_FAILED
BUILD_FAILED
QA_FAILED
TEST_FAILED
ISSUE_REVIEW_REQUIRED
LEGAL_REVIEW_REQUIRED
RELEASE_REJECTED
INFRASTRUCTURE_FAILED
HUMAN_REVIEW_REQUIRED
```

A failed gate never automatically advances.

---

# 8. Iterative Repair Semantics

Failures should produce structured repair jobs.

Example:

```text
BUILD_VALIDATION
       ↓
FAIL
       ↓
BuildFailure {
    category
    diagnostics
    logs
    environment
    candidate
}
       ↓
IronClaw investigation
       ↓
controlled modification
       ↓
new SourceCandidate
       ↓
affected evidence invalidated
       ↓
retry
```

The agent is expected to iterate.

A repair loop ends only when:

```text
gate passes

or

retry policy exhausted

or

human judgment required

or

external/infrastructure failure prevents progress
```

The agent cannot classify its own failure as acceptable.

---

# 9. Evidence Invalidation Engine

One of the most important core components will be an explicit dependency graph describing which evidence becomes stale after each category of source change.

Conceptually:

```text
SourceChange
     │
     ▼
AffectedDomains
     │
     ▼
EvidenceInvalidation
```

Examples:

```text
build dependency change
→ build
→ package QA
→ install tests
→ functional tests
→ release candidate

test-only change
→ affected test evidence
→ final validation

license metadata change
→ licensing obligation
→ package QA
→ release review

upstream source replacement
→ essentially all downstream evidence
```

Distribution adapters may extend this graph.

For Debian:

```text
debian/control change
debian/rules change
debian/tests change
debian/copyright change
quilt patch change
```

For Fedora:

```text
BuildRequires change
.spec scriptlet change
%files change
Patch change
Source change
Packit config change
```

The LLM never decides whether stale evidence can remain valid.

---

# 10. Knowledge and Authority Architecture

IronMaint needs something stronger than generic RAG.

The knowledge subsystem should preserve:

```text
document
distribution
authority
version/revision
section
strength
source URI
retrieval time
content hash
```

Generic interface:

```text
knowledge.search(...)
knowledge.get(...)
knowledge.authorities(...)
knowledge.changes_since(...)
knowledge.requirements_for(...)
```

A returned statement might look conceptually like:

```text
Authority:
  Debian Policy

Version:
  4.7.4.1

Classification:
  NORMATIVE_POLICY

Section:
  ...

Requirement:
  ...
```

or:

```text
Authority:
  Fedora Packaging Guidelines

Classification:
  PACKAGING_REQUIREMENT

Strength:
  MUST
```

The agent must therefore reason about both **content and authority**.

---

# 11. Authority Precedence

There is no single universal distribution-policy hierarchy.

Each adapter defines one.

## Debian

Initial precedence:

```text
Debian Policy
      ↓
formal Debian specifications
      ↓
archive/release requirements
      ↓
BTS protocol/procedures
      ↓
Developer's Reference
      ↓
Debian tooling diagnostics
      ↓
package-team policy
      ↓
local repository convention
      ↓
agent judgment
```

## Fedora

Initial precedence:

```text
Fedora legal/licensing requirements
      ↓
Fedora Packaging Guidelines
      ↓
Package Review Guidelines
      ↓
approved FPC exceptions
      ↓
FESCo policy
      ↓
Fedora update/release policy
      ↓
language-specific packaging guidance
      ↓
tooling diagnostics
      ↓
SIG/team policy
      ↓
repository convention
      ↓
agent judgment
```

Fedora's FPC explicitly owns packaging guidelines and can approve exemptions, making exceptions a first-class policy concept rather than something the agent may invent. :chatgpt-content-reference{index="8"}

---

# 12. Policy Obligation Engine

The obligation engine remains distribution-neutral.

A distribution adapter asks it to instantiate requirements applicable to the candidate.

Example:

```text
OBL-142

distribution:
  fedora

authority:
  Packaging Review Guidelines

strength:
  MUST

requirement:
  Source contents match authenticated upstream source.

evidence:
  source digest verification

status:
  PASS
```

or:

```text
OBL-281

distribution:
  debian

authority:
  Debian Policy 4.7.4.1

requirement:
  ...

status:
  PASS
```

The engine should understand:

```text
mandatory obligation
recommended obligation
explicit exception
not applicable
human judgment required
```

---

# 13. Distribution Adapter Contract

The `DistributionAdapter` is the critical architectural boundary.

Conceptually:

```rust
trait DistributionAdapter {
    fn identify_package(...);
    fn discover_authorities(...);
    fn inspect_source(...);
    fn detect_updates(...);

    fn derive_obligations(...);

    fn prepare_source(...);
    fn build_plan(...);
    fn qa_plan(...);
    fn test_plan(...);

    fn issue_provider(...);

    fn assemble_release_metadata(...);
    fn publication_plan(...);

    fn invalidate_evidence(...);
}
```

This is conceptual rather than the final Rust API.

The important rule is:

> Core crates must not depend directly on Debian or Fedora implementation crates.

Dependencies flow:

```text
ironmaint-core
       ↑
       │ implements contracts
       │
 ┌─────┴─────┐
 │           │
debian     fedora
adapter    adapter
```

Never:

```text
ironmaint-core
      ↓
debian-specific type
```

---

# 14. Tool Execution Contract

Agents should normally invoke typed capabilities rather than unrestricted shell commands.

Generic interface:

```text
tool.execute(
    capability,
    candidate,
    arguments
)
```

Result:

```text
ToolResult {
    invocation_id
    candidate_id
    tool
    tool_version
    environment
    exit_status
    classification
    structured_output
    log_digest
    artifacts
}
```

Raw command execution may exist inside trusted implementation components.

It should not be the public IronClaw capability model.

---

# 15. Debian Adapter

The first adapter remains Debian.

## Debian Knowledge

Corpus:

```text
Debian Policy
Policy upgrading checklist
Developer's Reference
copyright-format specification
autopkgtest specification
BTS documentation
Debian manpages
team-specific policies
```

Current baseline:

```text
Policy: 4.7.4.1
Developer's Reference: 14.16
```

---

## Debian Source Tooling

Preferred capabilities:

```text
dpkg
debhelper
git-buildpackage
quilt / gbp pq
uscan
dpkg-source
dpkg-buildpackage
```

---

## Debian QA

Initial deterministic suite:

```text
sbuild
lintian
autopkgtest
piuparts
reprotest
diffoscope
debdiff
blhc
```

Salsa CI should be used as an external reference for the Debian validation model.

---

## Debian Issues

Provider:

```text
Debian BTS
```

Capabilities eventually include:

```text
query
retrieve
search duplicates
severity
tags
versions
pending
fixed
forwarded
merge
block
reassign
report preparation
```

Writes remain controlled side effects.

---

## Debian Release Assembly

A Debian candidate includes:

```text
debian/changelog
verified Closes relationships
Policy obligations
Standards-Version review
source identity
QA evidence
candidate commit
release manifest
```

Traditional upload and tag2upload should eventually be supported.

---

# 16. Fedora Adapter

Fedora becomes the second implementation.

Its purpose is not merely to add RPM support.

It validates that IronMaint Core contains no hidden Debian assumptions.

---

# 17. Fedora Source Model

Primary representation:

```text
Fedora dist-git repository
       │
       ├── package.spec
       ├── sources
       ├── patches
       ├── Packit configuration where present
       └── supporting packaging files
```

Upstream source blobs should remain outside normal dist-git history according to Fedora's standard source/lookaside model.

The adapter should understand:

```text
Name
Version
Release
Epoch
License
URL
Source*
Patch*
BuildRequires
Requires
subpackage definitions
scriptlets
%prep
%build
%install
%check
%files
%changelog
```

through a proper RPM-aware parser rather than LLM regex extraction.

---

# 18. Fedora Knowledge Corpus

Initial Fedora authority corpus:

```text
Fedora Packaging Guidelines
Package Review Guidelines
Licensing Guidelines
language-specific Packaging Guidelines
FPC decisions and approved exceptions
relevant FESCo policies
Fedora update policies
Fedora package-maintainer documentation
Koji documentation
Mock documentation
Bodhi documentation
Packit documentation
Testing Farm/TMT documentation
Bugzilla/Fedora bug procedures
Fedora Change documentation where applicable
```

The current Package Review Guidelines explicitly distinguish MUST requirements—which block approval—from SHOULD items. :chatgpt-content-reference{index="9"}

That distinction should map directly to obligation strength.

---

# 19. Fedora Build and QA Backend

Initial local pipeline:

```text
spec validation
       ↓
source verification
       ↓
SRPM generation
       ↓
rpmlint
       ↓
Mock
       ↓
functional tests
```

Once local gates pass:

```text
Packit / dist-git CI
       ↓
Koji scratch build where appropriate
       ↓
Testing Farm / TMT
```

The agent can diagnose failures from these systems.

It cannot mark their results successful.

---

# 20. Packit Integration

Packit should be treated as a native Fedora execution and automation backend.

Current Fedora CI uses Packit by default for dist-git repositories, while the test execution itself remains in Fedora's test infrastructure. :chatgpt-content-reference{index="10"}

IronMaint should therefore not attempt to replace Packit.

Instead:

```text
IronMaint
     │
     ├── understand
     ├── modify
     ├── diagnose
     └── coordinate
             │
             ▼
           Packit
             │
       ┌─────┼─────────┐
       ▼     ▼         ▼
     CI    Koji      Bodhi
```

Packit currently supports downstream jobs including:

```text
pull_from_upstream
koji_build
bodhi_update
```

and can trigger a Koji build after qualifying dist-git changes and a Bodhi update after the corresponding successful build. :chatgpt-content-reference{index="11"}

---

# 21. Fedora Issue Management

Initial provider:

```text
Red Hat Bugzilla / Fedora product
```

Generic IronMaint operations:

```text
issues.for_package()
issues.get()
issues.search()

issues.propose_comment()
issues.propose_close()
issues.propose_reassign()
issues.propose_dependency()
issues.propose_new()
```

Fedora-specific adapter logic translates those into Bugzilla semantics.

As with Debian, issue mutation should initially be treated as a controlled side effect.

---

# 22. Fedora Bug-to-Update Correlation

The same core concept used for Debian bug closures applies.

```text
Bugzilla bug
     │
candidate fix identified
     │
test demonstrates fix
     │
FixEvidence generated
     │
candidate passes required gates
     │
Bodhi update may reference bug
```

Bodhi allows updates to carry related Bugzilla bugs and can close associated bugs when an update reaches stable. :chatgpt-content-reference{index="12"}

Therefore IronMaint must verify the relationship before permitting that metadata into the publication plan.

---

# 23. Fedora Release Path

The Fedora adapter's eventual publication graph should generally resemble:

```text
validated source candidate
        ↓
dist-git candidate
        ↓
local deterministic validation
        ↓
Packit CI
        ↓
Koji build
        ↓
Bodhi update
        ↓
Fedora gating/testing
        ↓
stable
```

Bodhi explicitly manages Fedora package updates after Koji builds and incorporates automated and human testing feedback into the update lifecycle. :chatgpt-content-reference{index="13"}

IronMaint may troubleshoot failures occurring in that pipeline.

It does not substitute its judgment for Fedora's infrastructure.

---

# 24. Publication Authority Differences

One major reason publication cannot be implemented generically as “sign and upload” is that Debian and Fedora establish trust differently.

## Debian

Typical authority centers around the maintainer-authorized source upload or signed tag.

The privileged boundary includes:

```text
maintainer OpenPGP operation
upload/tag authorization
Debian archive interaction
```

## Fedora

Publication authority is substantially more infrastructure-driven.

The sensitive boundaries instead include:

```text
Fedora identity
dist-git modification
Koji submission
Bodhi update creation
Bugzilla modification
```

Package signing and repository publication are handled through Fedora infrastructure rather than by handing an autonomous maintainer agent the distribution signing key.

Therefore the generic core knows only:

```text
PublicationPlan
PrivilegedOperation
Authorization
ExternalAuthority
```

The adapters define the mechanics.

---

# 25. Side-Effect Control Plane

All externally visible actions must cross a unified side-effect boundary.

Examples:

```text
push canonical repository
modify BTS
modify Bugzilla
file upstream issue
send upstream email
submit Koji build
create Bodhi update
invoke Debian upload
sign release
```

Every operation receives:

```text
PROPOSED
AUTHORIZED
EXECUTING
SUCCEEDED
FAILED
CANCELLED
```

Authorization policy can eventually vary by action.

For example:

```text
query Bugzilla
→ autonomous

comment on Bugzilla
→ policy-controlled

close Bugzilla issue
→ initially human approved

run Mock
→ autonomous

submit Koji scratch build
→ potentially autonomous

create Bodhi production update
→ approval-controlled
```

---

# 26. Release Assembler

`ironmaint-release` remains distribution-neutral.

It does not generate one universal package format.

It freezes a validated candidate and delegates packaging-specific release metadata to the adapter.

Inputs:

```text
MaintenanceJob
SourceCandidate
ObligationSet
GateEvidence
IssueEvidence
AdapterReleaseMetadata
```

Outputs:

```text
ReleaseCandidate
ReleaseManifest
PublicationPlan
RequiredApprovals
```

The core guarantees:

```text
candidate immutability
evidence freshness
gate completion
issue correlation
manifest integrity
```

The adapter guarantees:

```text
distribution-specific release correctness
```

---

# 27. Final Validation Replay

Before publication, the system freezes:

```text
candidate C
```

and performs the final required validation against exactly C.

```text
C
│
├── build
├── package QA
├── tests
├── policy obligations
├── issue correlation
└── release metadata
```

If anything changes:

```text
C → invalid

new source
   ↓
C2
   ↓
required validation repeats
```

No “tiny post-validation edit” exception exists.

---

# 28. Generic Release Manifest

Example:

```json
{
  "schema": "ironmaint.release/v1",
  "distribution": {
    "family": "debian",
    "release": "unstable"
  },
  "package": {
    "source": "foo",
    "version": "1.9.0-1"
  },
  "candidate": {
    "commit": "abc123",
    "tree": "def456"
  },
  "policy": {
    "complete": true,
    "obligation_set": "..."
  },
  "gates": {
    "build": "PASS",
    "package_qa": "PASS",
    "functional": "PASS",
    "upgrade": "PASS"
  },
  "issues": [],
  "publication": {
    "adapter": "debian",
    "ready": true
  },
  "required_approvals": []
}
```

Fedora produces the same outer schema with Fedora-specific publication metadata.

---

# 29. Audit and Provenance

Every job maintains an append-only activity trail.

Record:

```text
event received
repository/base commit
distribution
package version
documentation versions
policy queries
agent reasoning outputs required for audit
tool calls
source changes
candidate creation
builds
tests
failures
repair iterations
issue queries
proposed issue actions
obligation changes
release assembly
candidate hashes
approvals
publication operations
external system results
```

The system must answer:

> Why was this file changed?

> Which rule justified the change?

> Which candidate was actually tested?

> Which tests permitted release?

> Which external bugs did this release claim to fix?

> Who or what authorized the external action?

---

# 30. Security Model

## IronClaw Worker May

```text
read job repository
modify isolated worktree
query approved knowledge systems
invoke approved deterministic tools
query issue trackers
read build/test results
write job artifacts
```

## IronClaw Worker May Not

```text
modify orchestration policy
alter historical evidence
declare failed gates passing
access signing material
publish arbitrary source
push arbitrary canonical branches
silence mandatory policy failures
create policy exceptions
modify production issue trackers outside approved interfaces
perform unrestricted network activity
escape build sandbox
```

---

# 31. Human Review Categories

## Autonomous

Appropriate for:

```text
source inspection
upstream release detection
documentation retrieval
normal package editing
ordinary dependency repairs
patch refresh
build diagnosis
test diagnosis
issue analysis
release-note/changelog drafting
```

## Review-sensitive

Likely requires human review:

```text
policy ambiguity
new exception request
Lintian/rpmlint suppression
licensing ambiguity
major downstream behavioral patch
unclear upstream-vs-distribution ownership
maintainer-script/scriptlet risk
major dependency transition
NMU-like actions
package retirement
security-sensitive procedural decision
```

## Privileged

Requires explicit authorization:

```text
release signing where applicable
canonical publication
external issue mutation based on configured policy
distribution upload/update creation
privileged branch/tag operations
```

---

# 32. Rust Workspace Direction

Initial conceptual workspace:

```text
ironmaint/
│
├── crates/
│   ├── ironmaint-core/
│   ├── ironmaint-state/
│   ├── ironmaint-evidence/
│   ├── ironmaint-policy/
│   ├── ironmaint-artifacts/
│   ├── ironmaint-executor/
│   ├── ironmaint-release/
│   ├── ironmaint-audit/
│   └── ironmaint-mcp/
│
├── adapters/
│   ├── debian/
│   │   ├── debian-core/
│   │   ├── debian-policy/
│   │   ├── debian-build/
│   │   ├── debian-bts/
│   │   └── debian-release/
│   │
│   └── fedora/
│       ├── fedora-core/
│       ├── fedora-policy/
│       ├── fedora-build/
│       ├── fedora-bugzilla/
│       └── fedora-release/
│
├── services/
│   ├── orchestrator/
│   ├── knowledge-mcp/
│   ├── package-mcp/
│   ├── issue-mcp/
│   └── build-mcp/
│
├── schemas/
│
├── fixtures/
│   ├── debian/
│   └── fedora/
│
└── docs/
```

The final Phase 0 design will determine whether several of these should collapse into fewer crates.

We should avoid premature microservice decomposition.

---

# 33. Persistence

Start with SQLite.

Persist:

```text
jobs
events
candidates
obligations
gates
evidence metadata
issue snapshots
issue actions
tool executions
approvals
release manifests
```

Large immutable material:

```text
build logs
source archives
RPMs
DEBs
SRPMs
diffoscope output
test artifacts
tool reports
```

should live in artifact storage and be referenced by cryptographic digest.

PostgreSQL becomes an option only when multi-user or centralized operation requires it.

---

# 34. Development Phases

## Phase 0 — Distribution-Neutral Architecture

Define before substantive implementation:

```text
Rust workspace
core entities
adapter contract
state-transition model
evidence schema
artifact model
policy model
authority model
side-effect model
audit model
capability/security model
```

Create compile-time dependency rules preventing core crates from depending on Debian/Fedora crates.

### Exit checkpoint

A dummy distribution adapter can implement the complete core interface without changing core code.

Codex should be able to scaffold the system from stable contracts.

---

# 35. Phase 1 — Debian Read-Only Vertical Slice

Implement:

```text
Debian repository inspection
package metadata extraction
GBP discovery
Policy retrieval
Developer's Reference retrieval
authority provenance
Standards-Version inspection
Policy delta report
BTS read-only access
```

### Exit checkpoint

Given a real Debian package repository:

```text
ironmaint inspect .
```

produces an evidence-backed maintainer intake report without modifying anything.

---

# 36. Phase 2 — Core Deterministic Gate Engine

Implement the distribution-neutral:

```text
Gate
Evidence
Candidate
Invalidation
state transition
artifact hashing
audit trail
```

Then wire Debian:

```text
sbuild
lintian
autopkgtest
piuparts
reprotest
debdiff
```

No LLM required.

### Exit checkpoint

A Debian repository can traverse the deterministic validation state machine and failed gates cannot be bypassed.

---

# 37. Phase 3 — IronClaw Repair Loop

Introduce IronClaw.

Workflow:

```text
failure
   ↓
structured diagnostics
   ↓
IronClaw
   ↓
knowledge lookup
   ↓
controlled edit
   ↓
new candidate
   ↓
evidence invalidation
   ↓
retry
```

### Exit checkpoint

IronClaw repairs multiple deliberately introduced Debian packaging failures while the state machine prevents unauthorized progression.

---

# 38. Phase 4 — Debian Upstream Maintenance

Implement:

```text
uscan
upstream verification
GBP import
upstream diff analysis
patch refresh
copyright analysis
changelog preparation
```

### Exit checkpoint

A selected package moves from:

```text
old upstream
→ detected release
→ imported source
→ repaired packaging
→ validated candidate
```

---

# 39. Phase 5 — Debian Issue Workflow

Implement:

```text
BTS package intake
bug correlation
candidate fix association
pending proposal
Closes verification
control-action proposal
bug-report preparation
upstream-forward proposal
```

### Exit checkpoint

A release candidate can correctly correlate package changes with real BTS issues without performing uncontrolled external mutation.

---

# 40. Phase 6 — Debian Policy Obligation Engine

Formalize:

```text
Debian Policy obligations
Developer's Reference guidance
Standards-Version delta
copyright obligations
package-specific applicability
```

### Exit checkpoint

The release state machine refuses candidates with unsatisfied applicable mandatory obligations.

---

# 41. Phase 7 — Debian Atomic Release Candidate

Implement:

```text
ironmaint-release
candidate freezing
manifest generation
final gate replay
exact Git-tree binding
approval bundle
```

Stop at:

```text
READY_FOR_APPROVAL
```

### Exit checkpoint

The first complete Debian vertical slice exists without autonomous publication.

---

# 42. Phase 8 — Fedora Read-Only Adapter

Now test the abstraction.

Implement:

```text
dist-git inspection
RPM spec parsing
Fedora Packaging Guidelines retrieval
Review Guidelines retrieval
FPC/FESCo authority model
Bugzilla read-only intake
Packit configuration inspection
Koji/Bodhi state inspection
```

### Critical checkpoint

**No change to an IronMaint Core public model should be necessary merely because Fedora exists.**

If adding Fedora requires Debian-specific core types to be reinterpreted or generalized, stop and correct the architecture before continuing.

---

# 43. Phase 9 — Fedora Deterministic Validation

Wire Fedora tools into existing generic gates.

Example mapping:

```text
BUILD
  → Mock

PACKAGE_QA
  → rpmlint

FUNCTIONAL_TEST
  → TMT / Testing Farm

REMOTE_BUILD
  → Koji scratch

DISTRO_CI
  → Packit Fedora CI
```

### Exit checkpoint

The same core state machine that validates Debian can validate a Fedora package through a different adapter.

---

# 44. Phase 10 — Fedora Agent Repair Loop

Allow IronClaw to handle:

```text
spec errors
BuildRequires failures
patch failures
%files problems
rpmlint failures
Mock build failures
TMT failures
Koji failures
```

### Exit checkpoint

A Fedora package can undergo the same:

```text
FAIL
→ diagnose
→ repair
→ invalidate evidence
→ retry
```

cycle without distribution-specific logic leaking into IronMaint Core.

---

# 45. Phase 11 — Fedora Release Maintenance

Integrate:

```text
upstream monitoring
Packit
dist-git candidate management
Koji
Bugzilla correlation
Bodhi release preparation
```

Initially stop before production publication.

### Exit checkpoint

A Fedora package can progress from upstream release to a fully validated Bodhi-ready update plan.

---

# 46. Phase 12 — Privileged Publication

Only after both Debian and Fedora maintenance flows are mature should production publication be enabled.

## Debian

Potential paths:

```text
traditional signed upload

or

signed Git tag
→ tag2upload
```

## Fedora

Potential operations:

```text
canonical dist-git merge/push
Koji production build
Bodhi update
```

Each operation uses the generic side-effect/authorization framework.

---

# 47. Debian First Vertical Slice

Use one representative Debian package.

Scenario:

```text
new upstream release
        ↓
uscan detection
        ↓
source verification
        ↓
GBP import
        ↓
BTS intake
        ↓
upstream diff
        ↓
intentional build incompatibility
        ↓
sbuild FAIL
        ↓
IronClaw repair
        ↓
Lintian
        ↓
autopkgtest
        ↓
piuparts
        ↓
reprotest
        ↓
Policy review
        ↓
verified BTS fix
        ↓
release assembly
        ↓
final replay
        ↓
READY_FOR_APPROVAL
```

No signing or upload.

---

# 48. Fedora Second Vertical Slice

After the Debian vertical slice stabilizes:

```text
new upstream release
        ↓
Packit/upstream detection
        ↓
dist-git candidate
        ↓
Bugzilla intake
        ↓
spec update
        ↓
intentional BuildRequires failure
        ↓
Mock FAIL
        ↓
IronClaw repair
        ↓
rpmlint
        ↓
Mock
        ↓
TMT/Testing Farm
        ↓
Koji scratch
        ↓
Fedora obligation review
        ↓
verified Bugzilla fix
        ↓
candidate assembly
        ↓
Bodhi publication plan
        ↓
READY_FOR_APPROVAL
```

No production Bodhi update during the initial vertical slice.

---

# 49. Definition of Successful v0.1

v0.1 remains Debian-focused.

It succeeds when:

1. The distribution-neutral core exists.
2. Core crates contain no Debian-specific package semantics.
3. A Debian adapter implements the distribution contract.
4. One real Debian repository can be inspected.
5. Policy knowledge is versioned and attributable.
6. The deterministic state machine operates independently of an LLM.
7. Failed gates cannot advance.
8. Evidence becomes stale appropriately after source changes.
9. IronClaw can repair controlled Debian packaging failures.
10. BTS issues can be correlated with candidate fixes.
11. Mandatory obligations block release when unsatisfied.
12. A final immutable candidate can be assembled.
13. Required validation is bound to its exact Git tree.
14. A release manifest is produced.
15. The job terminates at `READY_FOR_APPROVAL`.
16. No release credential is accessible to the agent.

---

# 50. Definition of Successful v0.2

v0.2 validates the abstraction with Fedora.

It succeeds when:

1. A Fedora adapter can be added without redesigning IronMaint Core.
2. A dist-git repository can be inspected.
3. RPM spec metadata is structurally parsed.
4. Fedora packaging obligations can be generated.
5. Bugzilla can be queried.
6. Packit configuration can be understood.
7. Mock/rpmlint results populate generic IronMaint gates.
8. IronClaw can repair controlled Fedora packaging failures.
9. Testing Farm/Packit/Koji results can be incorporated as evidence.
10. A Fedora release candidate can be produced.
11. Bugzilla fixes can be correlated with the candidate.
12. A Bodhi-ready publication plan can be generated.
13. Production publishing remains behind explicit authorization.

---

# 51. Codex Handoff Requirements

Before implementation begins in earnest, the following specifications should exist.

```text
A. Repository and Rust workspace architecture

B. Core domain types

C. DistributionAdapter contract

D. Generic state-transition table

E. Evidence schema

F. Evidence invalidation model

G. Artifact/provenance model

H. Authority and policy schema

I. Knowledge MCP contract

J. Tool execution contract

K. Side-effect authorization contract

L. Audit-event schema

M. IronClaw capability model

N. Debian adapter contract

O. Debian build/QA mappings

P. Debian BTS contract

Q. Release assembler specification

R. Security/threat model

S. Test architecture

T. Debian first vertical-slice fixture
```

The Fedora adapter contracts do not need to be fully implemented before Codex begins.

However, the **Fedora workflow should be used as a design test against every Phase 0 core interface**.

For every major core type, we should ask:

```text
Does this make sense for Debian?

Does this make sense for Fedora?

Does it contain a hidden assumption from either?
```

---

# 52. Phase 0 Design Rule

Before implementing any abstraction, test it mentally against both distributions.

For example, do not define:

```text
struct Release {
    signed_tag: ...
}
```

because Fedora does not necessarily use the same authorization model.

Define:

```text
struct PublicationPlan {
    operations: Vec<PrivilegedOperation>,
    approvals: Vec<ApprovalRequirement>,
}
```

Likewise, do not define:

```text
BugClosure
```

as the core abstraction.

Use:

```text
IssueAction
```

because Debian and Fedora issue lifecycle semantics differ.

Do not define:

```text
StandardsVersion
```

in core.

Use:

```text
PolicyBaseline
```

with Debian providing `Standards-Version` as adapter metadata.

This discipline is essential.

---

# 53. What Should Remain Distribution-Specific

We should resist excessive abstraction.

These belong in adapters:

```text
package metadata syntax
version formatting details
source artifact conventions
changelog format
patch management details
issue tracker semantics
policy hierarchy
release branching conventions
build tools
test tools
publication systems
credentials
release-specific lifecycle rules
```

IronMaint Core should orchestrate their meaning without pretending they are identical.

---

# 54. What Should Be Universal

The following concepts are sufficiently common to belong in core:

```text
maintenance event
package identity
source candidate
immutable candidate hash
worktree
agent task
obligation
authority
evidence
gate
failure
repair
evidence invalidation
issue
issue action
approval
release candidate
publication plan
audit event
artifact
privileged operation
```

That is the abstraction boundary around which the platform should be built.

---

# 55. North-Star Architecture

```text
                         IronClaw
                    Reasoning / Repair
                           │
                           ▼
                 ┌───────────────────┐
                 │  IronMaint Core   │
                 │                   │
                 │ State             │
                 │ Policy            │
                 │ Evidence          │
                 │ Audit             │
                 │ Release           │
                 └─────────┬─────────┘
                           │
                 DistributionAdapter
                           │
              ┌────────────┴────────────┐
              ▼                         ▼
       ┌─────────────┐           ┌─────────────┐
       │   Debian    │           │   Fedora    │
       │             │           │             │
       │ Policy      │           │ Guidelines  │
       │ GBP         │           │ dist-git    │
       │ BTS         │           │ Bugzilla    │
       │ sbuild      │           │ Mock        │
       │ Salsa       │           │ Packit      │
       │ tag2upload  │           │ Koji/Bodhi  │
       └──────┬──────┘           └──────┬──────┘
              │                         │
              ▼                         ▼
       Debian authority          Fedora authority
```

---

# 56. North-Star Requirement

The finished architecture should make this statement true:

> **IronMaint is not an AI that packages software. It is a policy-aware maintenance control plane in which AI performs the reasoning-heavy work of a distribution maintainer while deterministic software, authoritative distribution documentation, native packaging tools, distribution infrastructure, cryptographic provenance, and human-controlled privileges determine whether that work is allowed to advance.**

And the architecture should make this equally true:

> **Adding support for a new distribution requires implementing that distribution's maintenance contract—not rewriting IronMaint's fundamental trust, evidence, orchestration, or release model.**

That is the foundation on which implementation should begin.

The next design checkpoint should be **Phase 0A: the Rust domain model and `DistributionAdapter` contract**. That is the highest-leverage document to finish before Codex touches the repository, because it will determine whether Debian and Fedora genuinely share a core or merely appear to.

