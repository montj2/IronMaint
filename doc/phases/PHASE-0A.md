Below is the implementation-grade Phase 0A specification. It deliberately stops before Debian tooling, MCP servers, IronClaw integration, persistence engines, or package builds; its job is to make the **core architecture difficult to accidentally corrupt later**.

# IronMaint Phase 0A
## Core Domain Model and Distribution Adapter Contract

**Specification version:** 0.1  
**Parent architecture:** IronMaint Project Plan v0.2  
**Implementation language:** Rust  
**Rust edition:** 2024  
**Implementation target:** Codex  
**Phase objective:** Establish the distribution-neutral domain model, state ownership rules, provenance model, evidence contracts, adapter API, and architectural dependency boundaries before implementing any Debian- or Fedora-specific behavior.

---

# 1. Purpose

Phase 0A establishes the architectural foundation of IronMaint.

No package should be built in this phase.

No Debian Policy document should be downloaded.

No RPM should be parsed.

No MCP server should be implemented.

No IronClaw agent should run.

Instead, Phase 0A answers the questions that everything else depends upon:

- What is a maintenance job?
- What uniquely identifies a package?
- What uniquely identifies a candidate source state?
- What does evidence mean?
- How is evidence bound to the source that produced it?
- What is a gate?
- What is a policy obligation?
- Who is allowed to change job state?
- How does a distribution adapter contribute behavior without controlling the state machine?
- How are Debian- and Fedora-specific concepts prevented from leaking into the core?
- How are external side effects represented?
- What is durable and serializable?
- What is immutable?
- What errors mean “repair the package” versus “the infrastructure failed”?
- How will later services communicate without exposing arbitrary internal Rust types?

The principal acceptance criterion is:

> A Debian-shaped workflow and a Fedora-shaped workflow must both fit the Phase 0A contracts naturally without adding distribution-specific concepts to the IronMaint core.

---

# 2. Architectural Invariants

The following rules are mandatory.

## 2.1 Core is distribution-neutral

`ironmaint-core` and every other core crate must not import a Debian- or Fedora-specific crate.

The dependency direction is:

```text
                         adapters
                            │
                            ▼
                    ironmaint-adapter-api
                            │
                 ┌──────────┼───────────┐
                 ▼          ▼           ▼
              policy     evidence      core
                 │          │
                 └────┬─────┘
                      ▼
                    core
```

Distribution implementations depend on core contracts.

Core contracts never depend on distribution implementations.

---

## 2.2 Candidates are immutable

A `SourceCandidate` describes one exact source state.

Once created, its identity cannot change.

Any modification to source or packaging creates a new candidate.

```text
Candidate A
commit/tree = X
     │
 source edit
     ▼
Candidate B
commit/tree = Y
```

Candidate B does not inherit evidence automatically from Candidate A.

---

## 2.3 Evidence is candidate-bound

Evidence must reference exactly one candidate fingerprint.

```text
Evidence E
    │
    └── candidate = C
```

Evidence obtained from candidate C must never authorize candidate D.

---

## 2.4 Agents do not mutate workflow state

Later IronClaw workers may request operations and create source modifications.

They may not directly change:

```text
JobState
GateStatus
ObligationStatus
ApprovalStatus
PublicationStatus
```

Only deterministic components own those state changes.

---

## 2.5 Adapters do not own state transitions

A Debian or Fedora adapter may say:

```text
"For BUILD_VALIDATION, these gates are required."
```

It may not say:

```text
"Advance the job to PACKAGE_QA."
```

Only the state-transition engine decides that.

---

## 2.6 A tool failure and an infrastructure error are different things

This distinction must exist in the type system.

Example:

```text
sbuild executes correctly
package fails to build
```

is a valid tool execution producing:

```text
GateResult = FAIL
```

It is **not** an application error.

By contrast:

```text
worker disappeared
sandbox could not start
database unavailable
```

is an infrastructure/system error.

This distinction will matter enormously to IronClaw repair behavior later.

---

## 2.7 External side effects are explicit objects

Operations such as:

```text
modify Debian BTS
modify Bugzilla
push canonical Git branch
submit Koji build
create Bodhi update
sign release
upload package
```

must be represented as `PrivilegedOperation` objects with explicit authorization state.

They may never be hidden inside ordinary adapter methods.

---

# 3. Phase 0A Workspace

Create this initial workspace:

```text
ironmaint/
│
├── Cargo.toml
├── rust-toolchain.toml
├── README.md
│
├── crates/
│   ├── ironmaint-core/
│   ├── ironmaint-evidence/
│   ├── ironmaint-policy/
│   ├── ironmaint-state/
│   ├── ironmaint-adapter-api/
│   └── ironmaint-testkit/
│
├── adapters/
│   ├── debian-stub/
│   └── fedora-stub/
│
├── xtask/
│
└── docs/
    └── architecture/
```

No production Debian or Fedora implementation belongs in these adapter crates yet.

The stub adapters exist only to prove the abstraction.

---

# 4. Crate Responsibilities

## 4.1 `ironmaint-core`

Owns universal value types and domain identities.

It may contain:

```text
IDs
distribution identity
package identity
versions as opaque values
maintenance events
maintenance jobs
source candidates
issues
issue actions
approval types
publication concepts
artifact references
digests
timestamps
repository-relative paths
```

It must not contain:

```text
Debian package fields
RPM tags
BTS semantics
Bugzilla semantics
build commands
policy evaluation
state-transition logic
tool execution
network clients
database clients
MCP
LLM code
IronClaw code
```

This crate should be deliberately boring.

---

## 4.2 `ironmaint-evidence`

Owns:

```text
Evidence
EvidenceId
EvidenceKind
EvidenceStatus
EvidenceProducer
EvidenceScope
ArtifactEvidence
Gate
GateDefinition
GateResult
GateStatus
InvalidationRule
InvalidationCause
EvidenceGraph
```

It depends on `ironmaint-core`.

It does not know Debian or Fedora.

---

## 4.3 `ironmaint-policy`

Owns:

```text
Authority
AuthorityClassification
Obligation
ObligationStrength
ObligationStatus
Applicability
PolicyBaseline
PolicyReference
ObligationSet
```

It may reference evidence IDs.

It does not retrieve policy documents in Phase 0A.

It models them.

---

## 4.4 `ironmaint-state`

Owns:

```text
JobState
Transition
TransitionRule
TransitionDecision
TransitionBlocker
TransitionEngine
JobProjection
JobEvent
```

It is the only crate allowed to determine whether a job can move from state X to state Y.

Adapters cannot bypass it.

---

## 4.5 `ironmaint-adapter-api`

Defines the contract implemented by distributions.

It owns:

```text
DistributionAdapter
AdapterDescriptor
AdapterCapabilities
AdapterContext
BuildPlan
QaPlan
PolicyPlan
IssuePlan
ReleaseMetadataPlan
PublicationPlanBuilder
AdapterError
```

It may depend on:

```text
ironmaint-core
ironmaint-evidence
ironmaint-policy
```

It should avoid depending directly on `ironmaint-state`.

The adapter describes what a distribution requires.

The state engine decides what those requirements mean for progression.

---

## 4.6 `ironmaint-testkit`

Owns fixtures and reusable architectural tests.

It should provide:

```text
fake package identities
fake candidates
fake evidence
fake authorities
fake gates
adapter conformance tests
serialization round-trip tests
transition test fixtures
```

The Debian and Fedora stub adapters should both be run through the same conformance suite.

---

# 5. Dependency Rules

The intended dependency graph is:

```text
ironmaint-core
     ▲
     │
ironmaint-evidence
     ▲
     │
ironmaint-policy

ironmaint-core
     ▲
     │
ironmaint-state
     │
     ├──── evidence
     └──── policy

ironmaint-adapter-api
     │
     ├──── core
     ├──── evidence
     └──── policy

debian-stub
     │
     └──── adapter-api

fedora-stub
     │
     └──── adapter-api
```

`xtask verify-architecture` must enforce forbidden dependencies.

At minimum it must fail if any crate under:

```text
crates/
```

depends on anything under:

```text
adapters/
```

It must also fail if `ironmaint-core` begins depending upon:

```text
ironmaint-state
ironmaint-adapter-api
ironmaint-policy
ironmaint-evidence
```

`ironmaint-core` is the bottom of the domain dependency graph.

---

# 6. Core Identifier Types

Do not use raw `String` or raw `Uuid` for entity IDs.

Create typed newtypes:

```rust
pub struct JobId(Uuid);
pub struct CandidateId(Uuid);
pub struct EvidenceId(Uuid);
pub struct GateId(Uuid);
pub struct ObligationId(Uuid);
pub struct ReleaseCandidateId(Uuid);
pub struct IssueActionId(Uuid);
pub struct ApprovalId(Uuid);
pub struct OperationId(Uuid);
pub struct ArtifactId(Uuid);
```

Requirements:

```text
Clone
Copy where appropriate
Eq
PartialEq
Ord
PartialOrd
Hash
Serialize
Deserialize
Display
FromStr
Debug
```

New IDs should use UUIDv7.

Reason:

```text
opaque identity
globally unique
roughly time sortable
not distribution specific
```

Do not encode business semantics into UUIDs.

---

# 7. Distribution Identity

Do not make distributions a Rust enum.

This would require modifying core whenever a distribution is added.

Use:

```rust
pub struct DistributionFamily(String);
```

Examples:

```text
debian
fedora
ubuntu
epel
opensuse
arch
```

Validation:

```text
lowercase ASCII
[a-z0-9][a-z0-9._-]*
maximum length: 64
```

Define:

```rust
pub struct DistributionRef {
    pub family: DistributionFamily,
    pub release: DistributionRelease,
}
```

`DistributionRelease` is also opaque.

Examples:

```text
unstable
trixie
rawhide
45
epel10
```

The adapter understands release semantics.

Core does not.

---

# 8. Package Versions

Package version semantics differ dramatically among distributions.

Therefore:

```rust
pub struct PackageVersion(String);
```

Core permits storage, equality and serialization.

Core does **not** provide:

```text
version ordering
epoch parsing
Debian version comparison
RPM EVR comparison
```

The adapter owns version comparison.

Required adapter operation:

```rust
fn compare_versions(
    &self,
    left: &PackageVersion,
    right: &PackageVersion,
) -> Result<Ordering, AdapterError>;
```

This is intentionally distribution-specific.

---

# 9. Package Identity

Define:

```rust
pub struct PackageIdentity {
    pub distribution: DistributionRef,
    pub source_name: PackageName,
}
```

Do not include the package version here.

The same logical package may exist at multiple versions.

Define:

```rust
pub struct PackageRevision {
    pub package: PackageIdentity,
    pub version: PackageVersion,
}
```

`PackageName` is an opaque validated string.

Core does not enforce Debian or RPM naming rules.

Adapter-specific validation happens separately.

---

# 10. Repository Identity

Define a generic repository reference:

```rust
pub struct RepositoryRef {
    pub vcs: VcsKind,
    pub canonical_url: Url,
}
```

Initial `VcsKind`:

```rust
pub enum VcsKind {
    Git,
}
```

This enum may later expand because it describes a technical mechanism, not a distribution.

Source location within a repository must use:

```rust
pub struct RepoPath(String);
```

Requirements:

```text
UTF-8
repository-relative
no leading /
no ..
no NUL
normalized /
```

Never persist host-local absolute paths as package identity.

---

# 11. Digests

Introduce a generic digest type:

```rust
pub struct Digest {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}
```

Initial algorithms:

```rust
pub enum DigestAlgorithm {
    Blake3,
    Sha256,
    GitSha1,
    GitSha256,
}
```

Validation must ensure hexadecimal length matches the algorithm where applicable.

Artifacts should normally use SHA-256.

IronMaint-internal fingerprints should normally use BLAKE3.

Git object identities retain the Git hash algorithm used by the repository.

---

# 12. Source Candidate

Define:

```rust
pub struct SourceCandidate {
    pub id: CandidateId,
    pub job_id: JobId,
    pub package: PackageRevision,
    pub repository: RepositoryRef,
    pub commit: GitObjectId,
    pub tree: GitObjectId,
    pub fingerprint: CandidateFingerprint,
    pub created_at: OffsetDateTime,
    pub parent_candidate: Option<CandidateId>,
}
```

`GitObjectId`:

```rust
pub struct GitObjectId {
    pub algorithm: GitHashAlgorithm,
    pub value: String,
}
```

Candidate fingerprint must be deterministic.

It should be computed over a versioned byte representation containing at least:

```text
schema marker
distribution family
distribution release
package source name
package version
repository canonical URL
commit identity
tree identity
```

Example conceptual input:

```text
ironmaint-candidate-v1\0
debian\0
unstable\0
foo\0
1.2.3-1\0
https://salsa...\0
sha1:abc...\0
sha1:def...\0
```

Hash with BLAKE3.

Do not derive the fingerprint by serializing arbitrary JSON.

The fingerprint function itself must have unit tests with fixed vectors.

---

# 13. Candidate Immutability

`SourceCandidate` fields should be private where practical and constructed through validated constructors.

Do not expose setters.

If package contents change:

```text
Candidate C1
   ↓
worktree modification
   ↓
commit/tree generated
   ↓
Candidate C2
```

C2 references:

```text
parent_candidate = C1
```

The source candidate itself remains immutable.

---

# 14. Maintenance Event

Define:

```rust
pub struct MaintenanceEvent {
    pub id: MaintenanceEventId,
    pub package: PackageIdentity,
    pub event_type: MaintenanceEventType,
    pub source: EventSource,
    pub observed_at: OffsetDateTime,
    pub external_reference: Option<String>,
}
```

Initial event types:

```rust
pub enum MaintenanceEventType {
    UpstreamRelease,
    IssueReported,
    BuildFailure,
    TestFailure,
    PolicyChange,
    SecurityAdvisory,
    DependencyTransition,
    ReleaseTransition,
    ManualRequest,
    Other(String),
}
```

No event type contains Debian/Fedora semantics.

---

# 15. Maintenance Job

Define the durable identity:

```rust
pub struct MaintenanceJob {
    pub id: JobId,
    pub package: PackageIdentity,
    pub initiating_event: MaintenanceEventId,
    pub created_at: OffsetDateTime,
}
```

Do **not** put mutable current workflow state directly into this immutable domain identity type.

Current state belongs to a `JobProjection` owned by `ironmaint-state`.

This supports event sourcing and prevents random callers from mutating state.

---

# 16. Job State

Define:

```rust
pub enum JobState {
    EventDetected,
    Intake,
    SourcePreparation,
    SourceAnalysis,
    IssueAnalysis,
    Maintenance,
    PolicyEvaluation,
    BuildValidation,
    PackageQa,
    FunctionalValidation,
    UpgradeValidation,
    ReleaseReview,
    CandidateAssembly,
    FinalValidation,
    ReadyForApproval,
    Approved,
    PublicationPending,
    Published,

    HumanReviewRequired,
    InfrastructureBlocked,
    Cancelled,
}
```

Do not create states such as:

```text
LintianPassed
KojiBuilt
BtsReviewed
```

Those belong to distribution-specific gates/evidence.

---

# 17. Job Projection

Define:

```rust
pub struct JobProjection {
    pub job: MaintenanceJob,
    pub state: JobState,
    pub active_candidate: Option<CandidateId>,
    pub version: u64,
    pub updated_at: OffsetDateTime,
}
```

`version` is a monotonic optimistic-concurrency number.

Every accepted state-changing event increments it.

Later persistence can use:

```text
WHERE version = expected_version
```

to prevent concurrent transitions from overwriting each other.

---

# 18. State Ownership

Only `ironmaint-state::TransitionEngine` may produce an accepted `StateTransitioned` event.

External callers submit:

```rust
pub struct TransitionRequest {
    pub job_id: JobId,
    pub expected_version: u64,
    pub target: JobState,
}
```

The transition engine evaluates:

```text
current state
required gates
current gate results
required obligations
obligation status
human approvals
infrastructure blockers
adapter-provided requirements
```

and returns:

```rust
pub enum TransitionDecision {
    Allowed(Transition),
    Blocked(Vec<TransitionBlocker>),
}
```

---

# 19. Transition Blockers

Initial blocker types:

```rust
pub enum TransitionBlocker {
    InvalidStatePath,
    MissingGate(GateId),
    FailedGate(GateId),
    IncompleteGate(GateId),
    MissingObligation(ObligationId),
    FailedObligation(ObligationId),
    ReviewRequired(ObligationId),
    MissingApproval(ApprovalId),
    StaleEvidence(EvidenceId),
    CandidateMismatch,
    InfrastructureBlocked,
}
```

This list is generic.

---

# 20. State Transition Graph

Phase 0A should implement this default linear path:

```text
EventDetected
    ↓
Intake
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
BuildValidation
    ↓
PackageQa
    ↓
FunctionalValidation
    ↓
UpgradeValidation
    ↓
ReleaseReview
    ↓
CandidateAssembly
    ↓
FinalValidation
    ↓
ReadyForApproval
    ↓
Approved
    ↓
PublicationPending
    ↓
Published
```

A package need not perform substantive work at every stage.

Instead the adapter may establish that a required gate is `NotApplicable`.

This preserves a common lifecycle without distribution-specific state shortcuts.

---

# 21. Exceptional States

At any nonterminal state, deterministic orchestration may move a job to:

```text
HumanReviewRequired
InfrastructureBlocked
Cancelled
```

Returning from `HumanReviewRequired` or `InfrastructureBlocked` requires a recorded event containing the resume state.

Do not infer the previous state from history at runtime.

Record it explicitly.

---

# 22. Evidence

Define:

```rust
pub struct Evidence {
    pub id: EvidenceId,
    pub candidate: CandidateFingerprint,
    pub kind: EvidenceKind,
    pub producer: EvidenceProducer,
    pub status: EvidenceStatus,
    pub observed_at: OffsetDateTime,
    pub environment: Option<EnvironmentFingerprint>,
    pub artifacts: Vec<ArtifactRef>,
}
```

Evidence contains facts produced by deterministic tools or trusted authorities.

It should not contain unrestricted LLM reasoning as authoritative evidence.

LLM-generated analysis may later be attached as an informational artifact, but not as a gate-passing evidence type by default.

---

# 23. Evidence Status

Define:

```rust
pub enum EvidenceStatus {
    Pass,
    Fail,
    NotApplicable,
    Inconclusive,
    InfrastructureError,
}
```

Do not use booleans.

The distinction matters.

---

# 24. Evidence Producer

```rust
pub struct EvidenceProducer {
    pub name: String,
    pub version: Option<String>,
    pub execution_id: Option<OperationId>,
}
```

Examples later:

```text
sbuild 0.x
lintian 2.x
mock 6.x
rpmlint 2.x
ironmaint-policy-engine 0.1
```

Core does not special-case any producer.

---

# 25. Artifact Reference

Define:

```rust
pub struct ArtifactRef {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub digest: Digest,
    pub media_type: Option<String>,
    pub size_bytes: Option<u64>,
}
```

The artifact bytes do not live inside the domain object.

Future storage may be:

```text
filesystem
S3-compatible store
database blob store
remote artifact service
```

Domain logic only needs identity and integrity.

---

# 26. Evidence Kind

Do not encode individual tools into the enum.

Use general categories:

```rust
pub enum EvidenceKind {
    SourceIntegrity,
    Build,
    PackageQa,
    FunctionalTest,
    UpgradeTest,
    Reproducibility,
    PolicyEvaluation,
    LicenseReview,
    IssueCorrelation,
    ReleaseAssembly,
    PublicationValidation,
    Other(String),
}
```

---

# 27. Gate

A gate expresses a deterministic condition required by the workflow.

```rust
pub struct GateDefinition {
    pub id: GateId,
    pub candidate: CandidateFingerprint,
    pub stage: GateStage,
    pub requirement: GateRequirement,
    pub mandatory: bool,
}
```

`GateStage` should correspond conceptually to workflow stages but remain separate from `JobState`.

Initial values:

```rust
pub enum GateStage {
    Source,
    Policy,
    Build,
    PackageQa,
    Functional,
    Upgrade,
    Release,
    Final,
    Publication,
}
```

---

# 28. Gate Requirement

A gate requirement points to acceptable evidence.

```rust
pub struct GateRequirement {
    pub evidence_kind: EvidenceKind,
    pub minimum_status: RequiredEvidenceStatus,
}
```

Initial required status:

```rust
pub enum RequiredEvidenceStatus {
    Pass,
    PassOrNotApplicable,
}
```

A gate does not invoke tools.

It describes what must be true.

---

# 29. Gate Result

```rust
pub struct GateResult {
    pub gate_id: GateId,
    pub candidate: CandidateFingerprint,
    pub status: GateStatus,
    pub evidence: Vec<EvidenceId>,
    pub evaluated_at: OffsetDateTime,
}
```

Statuses:

```rust
pub enum GateStatus {
    NotEvaluated,
    Pass,
    Fail,
    NotApplicable,
    Blocked,
    ReviewRequired,
}
```

Only deterministic gate evaluation logic may create a final gate result.

---

# 30. Evidence Freshness

Every evidence object is candidate-bound by default.

Phase 0A should deliberately **not** implement cross-candidate evidence reuse.

That optimization can be added later only when the evidence invalidation model has matured.

For Phase 0A:

```text
candidate changed
→ previous candidate evidence does not satisfy new candidate gates
```

This is conservative and correct.

---

# 31. Evidence Invalidation Model

Define the structure now even though sophisticated invalidation arrives later.

```rust
pub struct InvalidationRule {
    pub change_domain: ChangeDomain,
    pub invalidates: Vec<EvidenceKind>,
}
```

Generic change domains:

```rust
pub enum ChangeDomain {
    UpstreamSource,
    PackagingMetadata,
    BuildConfiguration,
    RuntimeDependencies,
    Tests,
    PatchSet,
    LicensingMetadata,
    DocumentationOnly,
    ReleaseMetadata,
    AdapterSpecific(String),
}
```

Adapters may supply additional rules.

Core itself must not understand `debian/control` or `%files`.

The Debian adapter may later map:

```text
debian/control
→ BuildConfiguration
```

The Fedora adapter may map:

```text
BuildRequires
→ BuildConfiguration
```

---

# 32. Policy Authority

Define:

```rust
pub struct Authority {
    pub id: AuthorityId,
    pub distribution: Option<DistributionFamily>,
    pub name: String,
    pub version: Option<String>,
    pub classification: AuthorityClassification,
}
```

Classification:

```rust
pub enum AuthorityClassification {
    NormativePolicy,
    FormalSpecification,
    DistributionProcedure,
    BestPractice,
    ToolDocumentation,
    TeamPolicy,
    LocalPolicy,
    Informational,
}
```

Do not hard-code Debian precedence or Fedora precedence here.

Adapters provide ordering rules.

---

# 33. Policy Reference

```rust
pub struct PolicyReference {
    pub authority: AuthorityId,
    pub section: Option<String>,
    pub title: Option<String>,
    pub source: Option<Url>,
    pub content_digest: Option<Digest>,
}
```

The actual document content is not required in the obligation.

The reference is provenance.

---

# 34. Obligation

Define:

```rust
pub struct Obligation {
    pub id: ObligationId,
    pub candidate: CandidateFingerprint,
    pub reference: PolicyReference,
    pub strength: ObligationStrength,
    pub applicability: Applicability,
    pub requirement: String,
    pub status: ObligationStatus,
    pub evidence: Vec<EvidenceId>,
}
```

Strength:

```rust
pub enum ObligationStrength {
    Mandatory,
    Recommended,
    BestPractice,
    Procedural,
    LegalReview,
    LocalPolicy,
}
```

Applicability:

```rust
pub enum Applicability {
    Unknown,
    Applicable,
    NotApplicable,
}
```

Status:

```rust
pub enum ObligationStatus {
    NotEvaluated,
    Pass,
    Fail,
    RequiresReview,
    ExceptionApproved,
}
```

---

# 35. Obligation Invariant

A mandatory applicable obligation with:

```text
Fail
RequiresReview
NotEvaluated
```

must block any transition requiring policy completion.

`ExceptionApproved` is valid only if an explicit approval record exists.

The agent itself cannot create an approved exception.

---

# 36. Policy Baseline

Generic:

```rust
pub struct PolicyBaseline {
    pub distribution: DistributionRef,
    pub authorities: Vec<AuthorityVersionRef>,
}
```

Examples later:

Debian:

```text
Debian Policy 4.7.4.1
Developer's Reference 14.16
```

Fedora:

```text
Packaging Guidelines revision X
FPC decisions snapshot Y
FESCo policy snapshot Z
```

Core does not assume a single version number exists.

---

# 37. Issue Model

Define generic external issue identity:

```rust
pub struct IssueRef {
    pub provider: IssueProviderId,
    pub external_id: String,
}
```

Examples:

```text
debian-bts / 1234567
redhat-bugzilla / 7654321
github / owner/repo#123
```

Define:

```rust
pub struct IssueSnapshot {
    pub issue: IssueRef,
    pub package: Option<PackageIdentity>,
    pub title: String,
    pub state: IssueState,
    pub severity: Option<String>,
    pub labels: Vec<String>,
    pub observed_at: OffsetDateTime,
}
```

Core does not interpret arbitrary severity strings.

---

# 38. Issue Actions

Represent proposed external mutations:

```rust
pub struct IssueAction {
    pub id: IssueActionId,
    pub issue: IssueRef,
    pub candidate: Option<CandidateFingerprint>,
    pub kind: IssueActionKind,
    pub rationale: String,
    pub evidence: Vec<EvidenceId>,
    pub authorization: AuthorizationState,
}
```

Generic kinds:

```rust
pub enum IssueActionKind {
    Comment,
    Close,
    Reopen,
    SetSeverity,
    AddLabels,
    RemoveLabels,
    Reassign,
    Link,
    MarkFixed,
    MarkPending,
    Forward,
    CreateRelated,
    Other(String),
}
```

Adapters translate these into provider-specific semantics.

---

# 39. Authorization State

Define once and reuse for all side effects.

```rust
pub enum AuthorizationState {
    Proposed,
    Authorized,
    Executing,
    Succeeded,
    Failed,
    Cancelled,
}
```

An agent may normally create:

```text
Proposed
```

It may not create:

```text
Authorized
Succeeded
```

without going through the appropriate deterministic/privileged service.

---

# 40. Approval

Define:

```rust
pub struct ApprovalRequirement {
    pub id: ApprovalId,
    pub category: ApprovalCategory,
    pub description: String,
}
```

Categories:

```rust
pub enum ApprovalCategory {
    HumanReview,
    PolicyException,
    ExternalMutation,
    Publication,
    Signing,
    SecuritySensitive,
}
```

Resolution:

```rust
pub enum ApprovalDecision {
    Approved,
    Rejected,
}
```

An approval record should include actor identity later.

Phase 0A may use an opaque `ActorId`.

---

# 41. Privileged Operation

Define:

```rust
pub struct PrivilegedOperation {
    pub id: OperationId,
    pub kind: PrivilegedOperationKind,
    pub candidate: CandidateFingerprint,
    pub required_approvals: Vec<ApprovalId>,
    pub authorization: AuthorizationState,
}
```

Generic kinds:

```rust
pub enum PrivilegedOperationKind {
    IssueTrackerMutation,
    CanonicalRepositoryPush,
    RemoteBuildSubmission,
    DistributionUpdateCreation,
    Signing,
    Publication,
    ExternalCommunication,
    Other(String),
}
```

Again, no Debian/Fedora mechanics appear here.

---

# 42. Release Candidate

Define:

```rust
pub struct ReleaseCandidate {
    pub id: ReleaseCandidateId,
    pub job_id: JobId,
    pub source: CandidateFingerprint,
    pub policy_baseline: PolicyBaseline,
    pub obligation_ids: Vec<ObligationId>,
    pub gate_ids: Vec<GateId>,
    pub issue_actions: Vec<IssueActionId>,
    pub created_at: OffsetDateTime,
}
```

Release candidate creation itself does not mean releasable.

The state engine determines whether it may become `ReadyForApproval`.

---

# 43. Publication Plan

Define:

```rust
pub struct PublicationPlan {
    pub release_candidate: ReleaseCandidateId,
    pub distribution: DistributionRef,
    pub operations: Vec<PrivilegedOperation>,
}
```

Examples later:

Debian:

```text
create signed tag
git-debpush
```

Fedora:

```text
push dist-git
submit Koji build
create Bodhi update
```

Same core structure.

Different adapter-generated operations.

---

# 44. Adapter Descriptor

Define:

```rust
pub struct AdapterDescriptor {
    pub family: DistributionFamily,
    pub implementation_name: String,
    pub implementation_version: String,
    pub capabilities: AdapterCapabilities,
}
```

Capabilities should use a set:

```rust
pub enum AdapterCapability {
    SourceInspection,
    VersionComparison,
    UpstreamDiscovery,
    PolicyDerivation,
    BuildPlanning,
    PackageQaPlanning,
    FunctionalTestPlanning,
    UpgradeTestPlanning,
    ReproducibilityPlanning,
    IssueRead,
    IssueWrite,
    ReleaseMetadata,
    PublicationPlanning,
}
```

Capability declaration does not grant privilege.

It only describes functionality.

---

# 45. Distribution Adapter Contract

Do not begin with one enormous method.

Use one root trait plus capability-specific traits.

Root:

```rust
pub trait DistributionAdapter:
    Send + Sync + 'static
{
    fn descriptor(&self) -> AdapterDescriptor;

    fn versioning(&self) -> &dyn VersioningCapability;

    fn package_model(&self) -> &dyn PackageModelCapability;

    fn policy(&self) -> Option<&dyn PolicyCapability>;

    fn build(&self) -> Option<&dyn BuildCapability>;

    fn issues(&self) -> Option<&dyn IssueCapability>;

    fn release(&self) -> Option<&dyn ReleaseCapability>;
}
```

Phase 0A need not execute external I/O.

These capabilities generate plans and normalize data.

Actual execution belongs to later tool/executor layers.

---

# 46. Versioning Capability

```rust
pub trait VersioningCapability: Send + Sync {
    fn validate(
        &self,
        version: &PackageVersion,
    ) -> Result<(), AdapterError>;

    fn compare(
        &self,
        left: &PackageVersion,
        right: &PackageVersion,
    ) -> Result<Ordering, AdapterError>;
}
```

Debian stub and Fedora stub must implement visibly different comparison behavior in tests.

This proves core isn't secretly comparing versions.

---

# 47. Package Model Capability

```rust
pub trait PackageModelCapability: Send + Sync {
    fn validate_name(
        &self,
        name: &PackageName,
    ) -> Result<(), AdapterError>;

    fn classify_changes(
        &self,
        changes: &[ChangedPath],
    ) -> Result<Vec<ChangeDomain>, AdapterError>;
}
```

For Phase 0A, stub implementations may classify fake paths.

Example Debian stub:

```text
debian/control → BuildConfiguration
debian/tests/* → Tests
```

Fedora stub:

```text
foo.spec → PackagingMetadata
tests/* → Tests
```

The purpose is architectural verification.

---

# 48. Policy Capability

```rust
pub trait PolicyCapability: Send + Sync {
    fn authority_order(
        &self,
    ) -> Vec<AuthorityClassification>;

    fn derive_obligation_plan(
        &self,
        context: &PolicyContext,
    ) -> Result<PolicyPlan, AdapterError>;
}
```

`PolicyPlan`:

```rust
pub struct PolicyPlan {
    pub baseline: PolicyBaseline,
    pub obligation_templates: Vec<ObligationTemplate>,
}
```

Phase 0A stubs may generate fake obligations.

No real Debian/Fedora policy retrieval yet.

---

# 49. Build Capability

The adapter does not execute commands.

It produces gate/tool requirements.

```rust
pub trait BuildCapability: Send + Sync {
    fn build_plan(
        &self,
        context: &CandidateContext,
    ) -> Result<BuildPlan, AdapterError>;

    fn qa_plan(
        &self,
        context: &CandidateContext,
    ) -> Result<QaPlan, AdapterError>;
}
```

A `BuildPlan` contains abstract tool requirements:

```rust
pub struct PlannedCheck {
    pub key: ToolCapabilityKey,
    pub evidence_kind: EvidenceKind,
    pub mandatory: bool,
}
```

Examples produced by Debian stub:

```text
debian.build.clean
debian.qa.static
```

Fedora stub:

```text
fedora.build.mock
fedora.qa.rpmlint
```

Core sees opaque capability keys.

---

# 50. Tool Capability Key

Define:

```rust
pub struct ToolCapabilityKey(String);
```

Validation:

```text
lowercase
dot-separated
adapter namespace required
```

Examples:

```text
debian.build.sbuild
debian.qa.lintian
fedora.build.mock
fedora.qa.rpmlint
```

This avoids embedding tool names into universal enums.

Later MCP/executor layers will bind capability keys to actual implementations.

---

# 51. Issue Capability

Phase 0A only describes semantics.

```rust
pub trait IssueCapability: Send + Sync {
    fn provider_id(&self) -> IssueProviderId;

    fn supported_actions(&self) -> Vec<IssueActionKind>;

    fn validate_action(
        &self,
        action: &IssueAction,
    ) -> Result<(), AdapterError>;
}
```

Debian stub may support:

```text
MarkPending
Forward
SetSeverity
```

Fedora stub may support a different subset.

The core issue model remains unchanged.

---

# 52. Release Capability

```rust
pub trait ReleaseCapability: Send + Sync {
    fn release_metadata_plan(
        &self,
        context: &ReleaseContext,
    ) -> Result<ReleaseMetadataPlan, AdapterError>;

    fn publication_plan(
        &self,
        context: &PublicationContext,
    ) -> Result<PublicationPlanTemplate, AdapterError>;
}
```

Templates become concrete publication plans only after candidate IDs and approval requirements are created.

---

# 53. Adapter Context

Adapters should receive explicit immutable contexts rather than global service locators.

Example:

```rust
pub struct CandidateContext<'a> {
    pub package: &'a PackageRevision,
    pub candidate: &'a SourceCandidate,
}
```

Policy context:

```rust
pub struct PolicyContext<'a> {
    pub package: &'a PackageRevision,
    pub candidate: &'a SourceCandidate,
    pub requested_baseline: Option<&'a PolicyBaseline>,
}
```

No adapter method receives:

```text
database connection
HTTP client
filesystem root
LLM client
IronClaw context
```

in Phase 0A.

Those belong to later execution/integration layers.

---

# 54. Adapter Error Taxonomy

Define:

```rust
pub struct AdapterError {
    pub kind: AdapterErrorKind,
    pub message: String,
}
```

Kinds:

```rust
pub enum AdapterErrorKind {
    Unsupported,
    InvalidPackage,
    InvalidVersion,
    InvalidConfiguration,
    MissingRequiredMetadata,
    PolicyUnavailable,
    HumanReviewRequired,
    ExternalTransient,
    ExternalPermanent,
    InternalAdapterFailure,
}
```

Rules:

```text
Unsupported
→ capability cannot perform requested operation

Invalid*
→ package/input problem

HumanReviewRequired
→ deterministic automation must stop

ExternalTransient
→ retry may succeed

ExternalPermanent
→ retry without changed conditions is inappropriate

InternalAdapterFailure
→ implementation defect or invariant failure
```

Never encode expected package build failures as `AdapterError`.

---

# 55. Execution Failure Model

Prepare now for later execution:

```rust
pub struct ExecutionOutcome {
    pub operation: OperationId,
    pub completion: ExecutionCompletion,
}
```

```rust
pub enum ExecutionCompletion {
    Completed {
        exit_code: i32,
    },
    TimedOut,
    InfrastructureFailed {
        reason: String,
    },
}
```

A process returning exit code `1` is:

```text
Completed { exit_code: 1 }
```

not:

```text
InfrastructureFailed
```

The tool-specific normalizer later determines whether that means a gate failure.

---

# 56. Job Events

Phase 0A should establish an append-only domain event representation.

Define:

```rust
pub struct EventEnvelope {
    pub event_id: DomainEventId,
    pub job_id: JobId,
    pub sequence: u64,
    pub occurred_at: OffsetDateTime,
    pub event: JobEvent,
}
```

Initial `JobEvent` variants:

```text
JobCreated
StateTransitioned
CandidateCreated
CandidateActivated
EvidenceRecorded
EvidenceInvalidated
GateDefined
GateEvaluated
ObligationDefined
ObligationEvaluated
IssueObserved
IssueActionProposed
ApprovalRequested
ApprovalResolved
ReleaseCandidateCreated
PublicationPlanCreated
JobBlocked
JobResumed
JobCancelled
```

Event history is append-only.

---

# 57. Projection Rule

`JobProjection` must be reconstructable from:

```text
MaintenanceJob
+
ordered JobEvents
```

Implement:

```rust
impl JobProjection {
    pub fn apply(
        &mut self,
        event: &JobEvent,
    ) -> Result<(), ProjectionError>;
}
```

Tests must verify that:

```text
same event sequence
→ same projection
```

No nondeterminism.

---

# 58. Event Sequence Rules

Per job:

```text
sequence starts at 1
strictly increases by 1
duplicates rejected
gaps rejected
```

The persistence layer will eventually enforce this transactionally.

Phase 0A testkit should enforce it in-memory.

---

# 59. Serialization Contract

Every durable/wire-level structure must implement:

```text
Serialize
Deserialize
```

using `serde`.

Wire representation:

```text
JSON
UTF-8
snake_case fields
snake_case enum discriminators
RFC3339 UTC timestamps
```

Do not serialize Rust type names as contract semantics.

---

# 60. Schema Versioning

Top-level durable records should contain:

```rust
pub struct SchemaVersion(pub u16);
```

Initial version:

```text
1
```

Examples requiring explicit schema versions later:

```text
event envelope
release manifest
publication plan
tool result envelope
MCP responses
```

Internal value objects need not each contain a schema version.

---

# 61. JSON Schema

Use `schemars` for public wire structures where practical.

Phase 0A should generate schemas for at least:

```text
EventEnvelope
Evidence
Obligation
ReleaseCandidate
PublicationPlan
```

Store generated schema snapshots under:

```text
schemas/
```

Tests must fail when generated schemas differ from committed schemas unless intentionally updated.

This makes accidental API changes visible.

---

# 62. Extension Data

Avoid unrestricted JSON blobs throughout core.

Where adapter-specific metadata is unavoidable, use:

```rust
pub struct AdapterMetadata {
    pub namespace: DistributionFamily,
    pub values: BTreeMap<String, serde_json::Value>,
}
```

Rules:

```text
core may store it
core may serialize it
core may hash it where explicitly specified
core must not branch on its contents
```

Adapter metadata should be used sparingly.

If a concept is genuinely universal, model it explicitly instead.

---

# 63. Time

Use:

```text
time::OffsetDateTime
```

internally.

All persisted timestamps:

```text
UTC
RFC3339
```

Business behavior must never depend on local timezone.

---

# 64. Determinism Rules

Domain calculations must not implicitly call:

```text
current time
random number generation
filesystem
network
environment variables
hostname
```

Constructors needing time or IDs receive them explicitly or through clearly isolated factories.

This makes tests deterministic.

UUID generation may live in an ID factory outside pure domain operations.

---

# 65. No Hidden Global Configuration

Phase 0A should contain no:

```text
lazy_static configuration
process-global mutable state
global HTTP client
global adapter registry
environment-variable reads inside domain types
```

Configuration will enter through explicit composition roots later.

---

# 66. Debian Stub Adapter

Implement enough behavior to prove Debian fits.

Descriptor:

```text
family: debian
version comparison: Debian-shaped fake implementation for test purposes
issue provider: debian-bts
build capability keys:
  debian.build.sbuild
  debian.qa.lintian
  debian.test.autopkgtest
publication operations:
  canonical repository push
  signing
  publication
```

For the stub only, version comparison may implement a deliberately minimal deterministic comparator sufficient for fixture tests.

Do **not** attempt to implement real `dpkg --compare-versions` semantics in Phase 0A.

---

# 67. Fedora Stub Adapter

Implement enough behavior to prove Fedora fits.

Descriptor:

```text
family: fedora
issue provider: redhat-bugzilla
build capability keys:
  fedora.build.mock
  fedora.qa.rpmlint
  fedora.test.testing_farm
publication operations:
  canonical repository push
  remote build submission
  distribution update creation
```

Again, real RPM version comparison is out of scope.

Use fixture logic demonstrating that comparison belongs in the adapter.

---

# 68. Required Cross-Distribution Fit Test

Create one generic test function:

```rust
fn assert_distribution_adapter_conformance(
    adapter: &dyn DistributionAdapter
)
```

Run it against:

```text
DebianStubAdapter
FedoraStubAdapter
```

It should verify at minimum:

```text
descriptor family is valid
package names can be validated
versions can be compared
change paths can be classified
policy plan can be produced
build plan can be produced
issue provider can be described
release plan can be produced
publication operations remain generic
no adapter mutates JobState
```

---

# 69. Debian Fixture Scenario

Represent this entirely using generic core types:

```text
Distribution:
  debian / unstable

Package:
  foo

Version:
  1.9.0-1

Candidate:
  exact Git tree

Obligations:
  mandatory policy obligation
  recommended maintainer guidance

Gates:
  Build
  PackageQa
  Functional

Issue:
  debian-bts / 123456

Issue action:
  MarkPending

Publication:
  CanonicalRepositoryPush
  Signing
  Publication
```

There must be no Debian-specific additions to `ironmaint-core` to express it.

---

# 70. Fedora Fixture Scenario

Represent:

```text
Distribution:
  fedora / rawhide

Package:
  foo

Version:
  1.9.0-1.fc45

Candidate:
  exact Git tree

Obligations:
  mandatory packaging guideline
  recommended packaging guidance

Gates:
  Build
  PackageQa
  Functional

Issue:
  redhat-bugzilla / 123456

Issue action:
  Comment

Publication:
  CanonicalRepositoryPush
  RemoteBuildSubmission
  DistributionUpdateCreation
```

Again, no Fedora-specific core types.

---

# 71. Architecture Failure Test

Add a test or `xtask` command that scans workspace metadata.

It must reject forbidden dependency edges.

Expected command:

```text
cargo xtask verify-architecture
```

It should return nonzero if:

```text
ironmaint-core depends on any adapter

ironmaint-core depends on state/evidence/policy

a core crate depends on debian-stub

a core crate depends on fedora-stub
```

Commit this command as part of the CI acceptance contract.

---

# 72. State Engine Tests

Required tests include:

### Valid progression

```text
EventDetected
→ Intake
```

allowed.

### Invalid skip

```text
Intake
→ BuildValidation
```

blocked.

### Failed mandatory gate

```text
Build gate = Fail
```

prevents progression.

### Missing mandatory gate

prevents progression.

### Not-applicable gate

may satisfy `PassOrNotApplicable`.

### Candidate mismatch

Evidence from C1 cannot satisfy gate for C2.

### Mandatory failed obligation

blocks policy completion.

### Recommended failed obligation

behavior should be configurable later; Phase 0A records it but does not automatically treat it as equivalent to Mandatory.

### Human review

moves workflow into `HumanReviewRequired`.

---

# 73. Candidate Integrity Tests

Required fixed-vector tests:

```text
same candidate identity
→ same CandidateFingerprint

different tree
→ different fingerprint

different package version
→ different fingerprint

different distribution release
→ different fingerprint

different repository
→ different fingerprint
```

Candidate fingerprints must be stable across process runs.

---

# 74. Serialization Tests

Every public durable type needs:

```text
serialize
deserialize
equality
```

round-trip tests.

Add snapshot fixtures for:

```text
Debian MaintenanceJob
Fedora MaintenanceJob
Evidence
Obligation
PublicationPlan
EventEnvelope
```

A serialization change should be intentional and visible in code review.

---

# 75. Error Tests

Verify that:

```text
package build exit code 1
≠ infrastructure error
```

and:

```text
worker failed to start
= infrastructure error
```

Adapters must not use `InternalAdapterFailure` for normal invalid package conditions.

---

# 76. Phase 0A.1 — Workspace Skeleton

Codex task:

Create:

```text
workspace
crates
stub adapters
xtask
basic CI-compatible commands
dependency graph
```

No substantive domain behavior yet.

### Acceptance

```text
cargo check --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

all succeed.

`cargo xtask verify-architecture` succeeds.

---

# 77. Phase 0A.2 — Core Identities and Candidates

Implement:

```text
typed IDs
DistributionFamily
DistributionRef
PackageName
PackageVersion
PackageIdentity
PackageRevision
RepositoryRef
RepoPath
Digest
GitObjectId
SourceCandidate
CandidateFingerprint
MaintenanceEvent
MaintenanceJob
```

### Acceptance

All validation and fingerprint tests pass.

No distribution-specific strings appear in core except test fixtures demonstrating arbitrary values.

---

# 78. Phase 0A.3 — Evidence, Policy and State

Implement:

```text
Evidence
Artifacts
Gates
Obligations
Authorities
JobState
JobProjection
JobEvent
TransitionEngine
```

### Acceptance

State engine tests and candidate/evidence mismatch tests pass.

A failed mandatory gate cannot advance state.

---

# 79. Phase 0A.4 — Adapter API

Implement:

```text
DistributionAdapter
VersioningCapability
PackageModelCapability
PolicyCapability
BuildCapability
IssueCapability
ReleaseCapability
AdapterError
AdapterDescriptor
capability declarations
plan structures
```

### Acceptance

Neither `ironmaint-core` nor the state engine changes to support an adapter.

---

# 80. Phase 0A.5 — Debian and Fedora Stub Adapters

Implement both stubs.

They must expose meaningfully different:

```text
version rules
path classifications
tool capability keys
issue provider IDs
policy plans
publication plans
```

### Acceptance

Both pass the same generic adapter conformance suite.

No conditional logic such as:

```rust
if distribution == "debian" { ... }
```

may appear inside core crates.

---

# 81. Phase 0A.6 — Schema and Architecture Hardening

Implement:

```text
JSON schema generation
schema snapshots
architecture verification
serialization fixtures
event projection tests
public API documentation
```

### Acceptance Commands

Codex must leave the repository in a state where all of the following pass:

```text
cargo fmt --check

cargo clippy \
  --workspace \
  --all-targets \
  --all-features \
  -- \
  -D warnings

cargo test --workspace

cargo xtask verify-architecture

cargo xtask verify-schemas
```

---

# 82. Documentation Requirements

Every public trait and major type must have Rustdoc explaining:

```text
what it represents
who owns mutation
what invariants apply
what it explicitly does not represent
```

Particularly important:

```text
SourceCandidate
Evidence
Gate
Obligation
DistributionAdapter
TransitionEngine
PublicationPlan
PrivilegedOperation
```

Documentation should describe architecture, not implementation trivia.

---

# 83. Dependency Policy

Keep dependencies conservative.

Expected foundational libraries may include:

```text
serde
serde_json
schemars
uuid
time
url
blake3
sha2
thiserror
```

Do not add:

```text
tokio
reqwest
sqlx
diesel
axum
tonic
MCP SDK
LLM SDK
Debian parsing library
RPM parsing library
```

unless Phase 0A specifically needs them.

It should not.

Phase 0A is a domain model, not an application runtime.

---

# 84. Async Policy

Core domain traits should remain synchronous where they perform only deterministic transformations.

Do not make everything `async` preemptively.

External I/O will later exist behind:

```text
executors
repositories
MCP services
remote providers
```

Those layers can be asynchronous.

`DistributionAdapter` Phase 0A methods should be synchronous because they produce plans and validate data rather than execute remote work.

This keeps the core testable and runtime-agnostic.

---

# 85. Panic Policy

Library code must not panic for expected input errors.

Avoid production:

```text
unwrap()
expect()
panic!()
todo!()
unimplemented!()
```

in implemented paths.

Use typed errors.

Panics are acceptable only for logically unreachable invariant violations where continuing would indicate a programming defect, and even those should be rare.

Clippy configuration should help enforce this where practical.

---

# 86. Unsafe Rust

Phase 0A must contain:

```text
#![forbid(unsafe_code)]
```

in every crate.

There is no need for unsafe Rust in this phase.

---

# 87. Logging

Do not introduce a logging framework into pure domain crates.

If `xtask` needs output, ordinary CLI output is sufficient.

Observability design belongs to a later phase.

---

# 88. Security Properties Established by Phase 0A

At completion, the type and dependency architecture should establish these properties:

```text
agent cannot directly advance state

adapter cannot directly advance state

evidence cannot silently move between candidates

distribution-specific release mechanics cannot leak into core

external mutations are explicit privileged operations

policy exceptions require explicit approval representation

candidate source identity is cryptographically bound

job state can be reconstructed from append-only events
```

These are more important than line count.

---

# 89. Explicit Phase 0A Non-Goals

Codex must not implement:

```text
Debian Policy retrieval
Fedora guideline retrieval
real dpkg version comparison
real RPM EVR comparison
Git command execution
worktree creation
MCP servers
IronClaw tools
SQLite
PostgreSQL
HTTP services
REST APIs
sbuild
Mock
Lintian
rpmlint
Bugzilla
BTS
Koji
Bodhi
Packit
Salsa CI
signing
upload
container execution
LLM integration
```

If implementation begins drifting toward any of these, stop.

---

# 90. Architectural Questions Codex Must Not Decide

Codex is explicitly prohibited from silently redesigning:

```text
candidate identity
crate dependency direction
state ownership
evidence ownership
adapter responsibilities
publication authorization model
distribution-neutral issue model
obligation model
privileged operation model
```

If the specification creates a genuine Rust implementation conflict, Codex should document:

```text
the conflicting requirement
the precise technical reason
the smallest proposed specification change
```

and stop that portion rather than inventing a different architecture.

---

# 91. Definition of Done — Phase 0A

Phase 0A is complete only when all of these statements are true:

1. The Rust workspace compiles cleanly.
2. Core crates contain no Debian or Fedora implementation dependencies.
3. Core domain concepts contain no Debian- or Fedora-specific semantics.
4. Candidates are immutable and fingerprinted.
5. Evidence is bound to candidate fingerprints.
6. The state machine alone owns workflow transitions.
7. Mandatory failed gates block transitions.
8. Mandatory failed obligations block relevant transitions.
9. Adapters describe requirements but cannot advance state.
10. External side effects are represented as privileged operations.
11. Debian and Fedora stub adapters both implement the same adapter contract.
12. Both stubs pass the same conformance suite.
13. Debian and Fedora fixture workflows can be represented without changing core.
14. Public durable structures round-trip through serialization.
15. Public schemas are generated and snapshot-tested.
16. Event replay deterministically recreates job projections.
17. Architecture dependency checks are automated.
18. The entire workspace passes formatting, linting and tests.
19. No external package-maintenance tooling has yet been implemented.
20. The resulting API is stable enough to begin Phase 0B/Phase 1 planning.

---

# 92. Codex Implementation Sequence

Codex should execute Phase 0A in this order:

```text
0A.1
Workspace and dependency boundaries

        ↓

0A.2
Core identities and candidate fingerprinting

        ↓

0A.3
Evidence, policy and state model

        ↓

0A.4
Distribution adapter API

        ↓

0A.5
Debian/Fedora conformance stubs

        ↓

0A.6
Schemas, architecture verification and documentation
```

After each checkpoint:

```text
format
lint
test
architecture verification
commit
```

Do not implement the whole phase in one undifferentiated coding pass.

---

# 93. Recommended Commit Boundaries

Suggested implementation commits:

```text
phase0a: initialize Rust workspace and architecture checks

phase0a: add core identifiers and package identity

phase0a: add immutable candidate and fingerprint model

phase0a: add evidence and artifact domain types

phase0a: add policy authority and obligation model

phase0a: add event-driven job state machine

phase0a: define distribution adapter contracts

phase0a: add Debian adapter conformance stub

phase0a: add Fedora adapter conformance stub

phase0a: add schema and serialization verification

phase0a: document architecture invariants
```

Each commit should compile and pass all tests applicable at that checkpoint.

---

# 94. Codex Completion Report

When Phase 0A is finished, Codex should produce a report containing:

```text
implemented checkpoints

workspace/crate tree

public domain types created

adapter traits created

architectural deviations, if any

test count and result

clippy result

architecture verification result

schema verification result

known limitations

questions requiring design review before the next phase
```

Do not treat a large code diff as evidence of completion.

The acceptance commands and architectural invariants determine completion.

---

# 95. Phase 0A North-Star Test

At the end of this phase, the following statement must be technically accurate:

> A source package for Debian and a source package for Fedora can enter the same IronMaint core as maintenance jobs, produce immutable candidates, accumulate policy obligations and test evidence, fail and retry deterministic gates, correlate external issues, become release candidates, and produce distribution-specific publication plans—without the IronMaint core knowing what a `.deb`, an RPM, `debian/control`, a spec file, BTS, Bugzilla, Koji, or tag2upload is.

If that statement is not true, Phase 0A is not complete.

---

# 96. Handoff to the Next Design Phase

Do not begin real Debian implementation immediately after Phase 0A without review.

The intended sequence is:

```text
Phase 0A implementation
        ↓
architecture/code review
        ↓
adjust core contract if required
        ↓
freeze Phase 0A API baseline
        ↓
design Phase 0B / Phase 1
        ↓
begin real Debian adapter
```

The reason is straightforward:

**Phase 0A is the cheapest point in the entire project to discover a bad abstraction.**

Once real Debian and Fedora integrations depend on these contracts, changes become dramatically more expensive.

This is the spec I would give Codex for the first implementation cycle. I would have it execute **0A.1 only first**, review that workspace/dependency skeleton, and then advance through 0A.2–0A.6 one checkpoint at a time rather than handing it the entire build at once.
