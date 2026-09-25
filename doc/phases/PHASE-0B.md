# IronMaint Phase 0B

## Runtime, Persistence, Executor, Workspace, and IronClaw Integration

**Specification version:** 0.1
**Parent architecture:** IronMaint Project Plan v0.2
**Depends on:** Phase 0A complete
**Implementation language:** Rust
**Primary runtime orchestrator:** IronClaw
**MCP implementation:** Official Rust MCP SDK (`rmcp`)
**Initial persistence:** SQLite
**Initial artifact backend:** Local content-addressed filesystem
**Initial execution backend:** Controlled local subprocess executor
**Distribution implementation:** Stub adapters only
**Phase objective:** Turn the Phase 0A domain model into a durable, restart-safe, executable system that IronClaw can operate through a constrained MCP interface.

---

# 1. Purpose

Phase 0A established what IronMaint means.

Phase 0B establishes how IronMaint runs.

At the completion of this phase, IronMaint must be capable of:

* persisting maintenance jobs;
* persisting append-only domain events;
* reconstructing job projections after restart;
* maintaining isolated job workspaces;
* creating immutable source candidates;
* storing artifacts by cryptographic digest;
* executing registered deterministic checks;
* capturing logs and execution evidence;
* surviving daemon restart;
* recovering interrupted operations safely;
* exposing package-maintenance capabilities over MCP;
* allowing IronClaw to inspect a job, respond to failures, modify an isolated workspace, create a new candidate, rerun checks, and request workflow progression;
* refusing any attempt by IronClaw to fabricate state, evidence, gate results, or approvals;
* completing a synthetic end-to-end maintenance workflow through an actual IronClaw agent.

No real Debian or Fedora package maintenance is implemented in this phase.

The central invariant is:

> **IronClaw reasons about what to do. IronMaint owns what actually happened.**

---

# 2. Architectural Boundary

IronMaint and IronClaw remain separate systems.

```text
                    ┌──────────────────────────┐
                    │        IronClaw          │
                    │                          │
                    │ LLM reasoning            │
                    │ planning                 │
                    │ iteration                │
                    │ routines                 │
                    │ agent lifecycle          │
                    └────────────┬─────────────┘
                                 │
                                 │ MCP
                                 ▼
                    ┌──────────────────────────┐
                    │       IronMaint MCP      │
                    │      capability API      │
                    └────────────┬─────────────┘
                                 │
                                 ▼
                    ┌──────────────────────────┐
                    │    IronMaint Runtime     │
                    │                          │
                    │ domain commands          │
                    │ state machine            │
                    │ policy/gate evaluation   │
                    │ execution coordination   │
                    └───────┬────────┬─────────┘
                            │        │
                 ┌──────────┘        └──────────┐
                 ▼                              ▼
         ┌───────────────┐              ┌────────────────┐
         │ Persistence   │              │   Executor     │
         │               │              │                │
         │ SQLite        │              │ registered     │
         │ event store   │              │ capabilities   │
         │ projections   │              │ only           │
         └───────┬───────┘              └───────┬────────┘
                 │                              │
                 ▼                              ▼
         ┌───────────────┐              ┌────────────────┐
         │ Artifact      │              │ Job Workspace  │
         │ Store         │              │                │
         │ SHA-256 CAS   │              │ source tree    │
         └───────────────┘              │ scratch        │
                                        └────────────────┘
```

IronClaw must never:

```text
open IronMaint's SQLite database;
append domain events directly;
set JobState directly;
set GateStatus directly;
set ObligationStatus directly;
insert Evidence directly;
write artifact metadata directly;
run arbitrary authoritative package checks;
access release credentials.
```

All authoritative interaction crosses IronMaint's capability boundary.

---

# 3. Why IronMaint Owns Persistence

Do not store IronMaint jobs inside IronClaw's persistence model.

IronClaw currently has its own Reborn persistence architecture and supports multiple backing implementations. That state belongs to IronClaw.

IronMaint owns:

```text
maintenance transactions
source candidates
policy obligations
package evidence
check executions
release evidence
issue correlation
publication plans
```

IronClaw owns:

```text
agent threads
agent context
LLM messages
tool-call coordination
routines
its own scheduler state
its own memory
```

The systems correlate through IDs.

They do not share tables.

This preserves the ability to:

```text
upgrade IronClaw independently;
replace IronClaw;
run multiple orchestrators;
audit package state without LLM transcripts;
reconstruct maintenance history independently.
```

---

# 4. Phase 0B Architectural Invariants

The following are mandatory.

## 4.1 IronMaint is authoritative

MCP calls are commands or queries against IronMaint.

They are not direct database operations.

## 4.2 IronClaw cannot manufacture evidence

A tool call such as:

```text
"mark build passed"
```

must not exist.

The only way build evidence appears is:

```text
registered check
      ↓
IronMaint executor
      ↓
captured execution
      ↓
normalizer
      ↓
Evidence
```

## 4.3 Checks run only against immutable candidates

A dirty workspace may not be tested as authoritative release evidence.

The cycle is:

```text
edit workspace
      ↓
capture candidate
      ↓
run checks against candidate
```

## 4.4 Dirty workspaces are not candidates

A worktree is mutable.

A candidate is immutable.

Do not conflate them.

## 4.5 IronClaw cannot choose arbitrary executables

IronClaw provides:

```text
check_id
```

or another registered capability identifier.

It does not provide:

```text
executable
argv
shell command
environment
working directory
```

## 4.6 No shell interpretation

Executor subprocesses use direct executable + argument invocation.

Never:

```text
sh -c
bash -c
cmd.exe /c
```

for registered check execution.

## 4.7 Persistence precedes acknowledgement

A successful mutating MCP response must not be returned until the corresponding authoritative transaction is durably committed.

## 4.8 Restart must preserve semantic state

Restart may interrupt execution.

Restart must not lose:

```text
jobs
events
candidates
workspaces
completed evidence
gates
obligations
operations
artifacts
```

## 4.9 Interrupted is not failed

If IronMaint crashes while a check is running:

```text
Running
→ Interrupted
```

It must not be silently converted into:

```text
Fail
```

because the package did not necessarily fail.

## 4.10 Privileged external effects remain unavailable

Phase 0B must not perform:

```text
BTS mutation
Bugzilla mutation
canonical Git push
Koji submission
Bodhi update
signing
Debian upload
external email
```

The Phase 0A types representing such operations remain intact, but execution returns unsupported.

---

# 5. Updated Workspace Layout

Add the following crates and binaries to the completed 0A workspace:

```text
ironmaint/
│
├── crates/
│   ├── ironmaint-core/
│   ├── ironmaint-evidence/
│   ├── ironmaint-policy/
│   ├── ironmaint-state/
│   ├── ironmaint-adapter-api/
│   ├── ironmaint-testkit/
│   │
│   ├── ironmaint-store/
│   ├── ironmaint-store-sqlite/
│   ├── ironmaint-artifacts/
│   ├── ironmaint-workspace/
│   ├── ironmaint-executor/
│   ├── ironmaint-runtime/
│   └── ironmaint-mcp/
│
├── adapters/
│   ├── debian-stub/
│   └── fedora-stub/
│
├── bins/
│   ├── ironmaintd/
│   ├── ironmaintctl/
│   └── ironmaint-fixture-tool/
│
├── integrations/
│   └── ironclaw/
│       ├── README.md
│       └── skills/
│           └── ironmaint-maintainer/
│               └── SKILL.md
│
├── fixtures/
│   └── runtime/
│
├── migrations/
│
├── schemas/
│
├── xtask/
│
└── docs/
```

---

# 6. `ironmaint-store`

This crate defines persistence contracts only.

It does not contain SQLite.

Primary contracts:

```rust
pub trait EventStore;
pub trait ProjectionStore;
pub trait CandidateStore;
pub trait EvidenceStore;
pub trait GateStore;
pub trait ObligationStore;
pub trait OperationStore;
pub trait ArtifactMetadataStore;
pub trait WorkspaceMetadataStore;
```

However, avoid forcing every caller to coordinate ten repositories manually.

Expose a higher-level transaction abstraction:

```rust
pub trait IronMaintStore {
    type Transaction<'a>: StoreTransaction
    where
        Self: 'a;

    async fn begin(&self)
        -> Result<Self::Transaction<'_>, StoreError>;
}
```

`StoreTransaction` provides domain repositories sharing one transaction.

The runtime should be capable of performing:

```text
append event
update projection
persist entity
commit
```

atomically.

---

# 7. Event Store

The Phase 0A event stream remains the authoritative job history.

Persist each `EventEnvelope`.

Required columns conceptually:

```text
event_id
job_id
sequence
schema_version
occurred_at
event_type
payload_json
```

Constraints:

```text
PRIMARY KEY(job_id, sequence)

UNIQUE(event_id)

sequence > 0
```

When appending an event:

```text
expected current job version
        ↓
verify
        ↓
insert event
        ↓
update projection
        ↓
commit
```

The append and projection update occur in the same database transaction.

---

# 8. Projection Persistence

Persist `JobProjection` as a read model.

Conceptual table:

```text
jobs
----
job_id
package_json
initiating_event_id
state
active_candidate_id
version
created_at
updated_at
```

The persisted projection is a cache.

The event stream remains reconstructable authority.

Provide an administrative operation:

```text
ironmaintctl rebuild-projections
```

which:

```text
deletes/reinitializes derived projections
        ↓
replays events
        ↓
reconstructs current state
```

The rebuilt projection must equal the previously persisted projection.

This becomes an important integrity test.

---

# 9. SQLite Backend

Implement:

```text
ironmaint-store-sqlite
```

using SQLite.

Recommended runtime behavior:

```text
WAL mode
foreign_keys = ON
busy timeout configured
synchronous = NORMAL or stronger
```

Do not expose the SQLite connection outside the store implementation.

Use migrations checked into:

```text
/migrations
```

Migrations are forward-only in 0B.

The daemon must refuse to run against:

```text
newer unsupported schema version
corrupt migration history
```

---

# 10. Single-Daemon Rule

Phase 0B supports exactly one `ironmaintd` process per state directory.

Acquire:

```text
<StateDir>/ironmaint.lock
```

at startup.

A second daemon attempting to use the same state directory must fail immediately.

This avoids cross-process workspace and executor races while the distributed-leasing model is not yet implemented.

SQLite may support multiple writers.

IronMaint 0B intentionally does not.

---

# 11. State Directory Layout

Default:

```text
$XDG_STATE_HOME/ironmaint/
```

or an explicitly configured state directory.

Layout:

```text
ironmaint/
│
├── ironmaint.db
├── ironmaint.lock
│
├── artifacts/
│   └── sha256/
│
├── workspaces/
│   └── <job-id>/
│
├── tmp/
│
└── auth/
    └── mcp-token
```

On platforms without XDG semantics, use an appropriate configured application state directory.

Do not scatter state throughout `$HOME`.

---

# 12. Artifact Store

`ironmaint-artifacts` implements a content-addressed artifact repository.

Artifact digest:

```text
SHA-256
```

Storage shape:

```text
artifacts/
└── sha256/
    └── ab/
        └── cd/
            └── abcdef...
```

Artifact write algorithm:

```text
create temp file
      ↓
stream bytes
      ↓
calculate SHA-256
      ↓
fsync file
      ↓
rename atomically into CAS location
      ↓
fsync parent directory where supported
      ↓
persist ArtifactRef metadata
```

Existing identical artifacts are reused.

---

# 13. Artifact Integrity

Every artifact read optionally or mandatorily verifies its digest depending upon configured integrity mode.

Release-related evidence must always use verified reads.

Phase 0B defaults to verified reads.

Corrupt artifacts produce:

```text
ArtifactIntegrityError
```

not ordinary package failure.

---

# 14. Artifact Types

Initial runtime artifact kinds should include:

```text
stdout_log
stderr_log
combined_log
execution_report
workspace_diff
agent_attachment
fixture_output
```

Do not place large logs in SQLite JSON columns.

Persist their `ArtifactRef`.

---

# 15. Artifact Limits

Configurable defaults must exist for:

```text
maximum stdout bytes
maximum stderr bytes
maximum individual artifact bytes
maximum artifacts per execution
maximum retained artifact bytes per job
```

If process output exceeds a configured limit:

```text
capture bounded output
record truncation
continue/terminate according to tool policy
```

The evidence must explicitly record:

```text
truncated = true
```

Never silently truncate logs.

---

# 16. Workspace Model

`ironmaint-workspace` owns mutable job source areas.

A job workspace conceptually contains:

```text
workspaces/<job-id>/
│
├── source/
└── scratch/
```

`source/` is the mutable maintenance worktree.

`scratch/` is non-authoritative temporary storage.

No evidence-producing check runs against `scratch/`.

---

# 17. Workspace State

Persist:

```rust
pub struct WorkspaceState {
    pub job_id: JobId,
    pub revision: WorkspaceRevision,
    pub base_candidate: Option<CandidateId>,
    pub dirty: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
```

Add a strongly typed:

```rust
pub struct WorkspaceRevision(u64);
```

Every successful source mutation increments it.

---

# 18. Workspace Concurrency

Every mutation requires:

```text
expected_workspace_revision
```

Example:

```text
workspace currently revision 17

agent calls apply_patch(expected=17)
→ succeeds
→ workspace becomes 18
```

A second stale request:

```text
apply_patch(expected=17)
```

returns:

```text
Conflict {
    current_revision: 18
}
```

This protects against duplicate or parallel agent writes.

---

# 19. Workspace Path Security

All workspace file operations must enforce root confinement.

Reject:

```text
absolute paths
..
NUL
paths escaping source/
writes into .git internal control files
```

Symlink policy:

```text
reads:
  may read symlink metadata;
  following symlinks outside workspace prohibited

writes:
  may not traverse a symlink component by default
```

A later package-specific phase may introduce controlled exceptions if real source trees require them.

---

# 20. Workspace MCP Operations

IronClaw may interact only through controlled methods:

```text
list
read_text
read_bytes_metadata
apply_patch
diff
status
```

Do not expose:

```text
rm -rf
arbitrary file descriptor
absolute filesystem access
chown
chmod arbitrary host path
```

Binary file editing is out of scope for 0B.

---

# 21. Patch Application

Primary agent mutation interface:

```text
workspace.apply_patch
```

Input:

```text
job_id
expected_workspace_revision
unified_diff
```

IronMaint validates:

```text
all paths confined to source/
no .git manipulation
patch applies cleanly
workspace revision matches
```

Then applies the patch atomically where practical.

On failure, no workspace revision increments.

---

# 22. Candidate Capture

Checks require immutable candidates.

Therefore provide:

```text
candidate.capture
```

Conceptually:

```text
dirty workspace
       ↓
validate repository state
       ↓
create internal maintenance commit
       ↓
resolve Git commit/tree identity
       ↓
construct SourceCandidate
       ↓
calculate CandidateFingerprint
       ↓
persist CandidateCreated
       ↓
activate candidate
       ↓
mark workspace clean
```

This commit exists in the job-local maintenance history.

It is not a canonical distribution release commit.

---

# 23. Candidate Capture Safety

Candidate creation must not:

```text
push
sign
tag
rewrite canonical branches
```

Internal commit author may be:

```text
IronMaint <ironmaint@localhost>
```

with clearly non-release semantics.

The commit message should contain:

```text
job ID
candidate ID
workspace revision
```

but no secrets.

---

# 24. Minimal Git Support

Phase 0B may use the system `git` executable internally.

It must invoke it through a controlled programmatic interface with fixed operations.

Allowed operations initially:

```text
rev-parse
status
diff
add
commit
worktree
```

No IronClaw tool accepts arbitrary Git arguments.

No generic:

```text
git(args_from_agent)
```

capability exists.

Real `git-buildpackage`, dist-git, branch management, upstream import, etc. remain later-phase concerns.

---

# 25. Executor Architecture

`ironmaint-executor` provides a generic execution contract.

Core abstraction:

```rust
pub trait Executor {
    async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionRecord, ExecutorError>;
}
```

Agent callers do not construct `ExecutionRequest`.

The runtime does.

---

# 26. Tool Registry

Create a runtime `ToolRegistry`.

It maps:

```text
ToolCapabilityKey
```

to trusted execution definitions.

Example:

```text
fixture.build.validate
        ↓
ironmaint-fixture-tool
        args:
          check-build
```

Future:

```text
debian.build.sbuild
        ↓
sbuild ...

fedora.build.mock
        ↓
mock ...
```

The registry belongs to runtime composition/configuration.

It is not agent-editable.

---

# 27. Execution Definition

Conceptually:

```rust
pub struct ToolDefinition {
    pub capability: ToolCapabilityKey,
    pub executable: PathBuf,
    pub fixed_args: Vec<OsString>,
    pub class: ExecutionClass,
    pub retry: RetryClass,
    pub limits: ExecutionLimits,
}
```

Initial execution classes:

```text
Check
WorkspaceInternal
```

Reserve:

```text
PrivilegedExternal
```

but return unsupported in 0B.

---

# 28. Retry Classification

Define:

```rust
pub enum RetryClass {
    Safe,
    Conditional,
    Never,
}
```

Examples:

```text
deterministic local check:
  Safe

future remote read:
  Conditional

future package upload:
  Never
```

IronClaw does not choose this value.

The tool definition does.

---

# 29. Execution Request

A request includes:

```text
operation ID
job ID
candidate fingerprint
registered capability
workspace reference
execution limits
runtime-generated arguments
```

It must not include arbitrary shell input.

---

# 30. Process Environment

Executor should begin with a controlled environment.

Do not inherit the complete daemon environment.

Initial allowlist may include:

```text
PATH
HOME configured to job-safe location
LANG
LC_ALL
TMPDIR
```

Future adapters may add explicitly permitted values.

Do not expose:

```text
MCP token
database credentials
LLM credentials
signing credentials
host cloud credentials
```

to check processes.

---

# 31. Execution Limits

Phase 0B must support:

```rust
pub struct ExecutionLimits {
    pub timeout: Duration,
    pub stdout_max_bytes: u64,
    pub stderr_max_bytes: u64,
}
```

The type should leave room for future:

```text
memory
CPU
PID
disk
network
```

limits.

Do not claim these are enforced until they actually are.

---

# 32. Process Completion Model

Retain the Phase 0A distinction:

```text
process ran and exited 1
```

means:

```text
Completed { exit_code: 1 }
```

It does not mean infrastructure failure.

Executor failure examples:

```text
executable missing
spawn failed
workspace inaccessible
artifact store unavailable
runtime internal error
```

---

# 33. Durable Operation State

Persist each execution.

Add runtime state:

```text
Pending
Running
Completed
TimedOut
Cancelled
Interrupted
InfrastructureFailed
```

Do not overload package pass/fail into operation state.

A completed process may produce:

```text
gate PASS
```

or:

```text
gate FAIL
```

depending upon normalization.

---

# 34. Operation Lifecycle

Before process spawn:

```text
persist Pending
```

Immediately before/with spawn:

```text
persist Running
```

After process termination:

```text
persist completion
store logs
create normalized Evidence
evaluate affected gate
```

A process must never produce authoritative evidence before the corresponding durable execution record exists.

---

# 35. Restart Recovery

At daemon startup:

```text
find operations state = Running
```

Because the local 0B executor cannot safely reattach to arbitrary prior child processes:

```text
Running
→ Interrupted
```

Record an audit/domain event.

Do not automatically retry every interrupted operation.

Use its `RetryClass`.

For:

```text
RetryClass::Safe
```

the runtime may expose retry as an allowed next action.

---

# 36. Executor Parallelism

Phase 0B permits:

```text
multiple independent MaintenanceJobs concurrently
```

but defaults to:

```text
one active execution per MaintenanceJob
```

This deliberately simplifies:

```text
workspace consistency
candidate changes
log attribution
gate evaluation
```

Later phases may allow parallel independent checks for the same immutable candidate.

The schema and APIs should not prevent that future extension.

---

# 37. Runtime Application Service

`ironmaint-runtime` is the authoritative application layer.

It coordinates:

```text
domain types
state engine
adapters
store
artifact store
workspace
executor
tool registry
```

MCP handlers call the runtime.

They do not implement domain logic themselves.

---

# 38. Runtime Command Pattern

Mutating operations should resemble:

```text
request
    ↓
load projection
    ↓
verify expected version/revision
    ↓
validate domain invariants
    ↓
perform required deterministic effect
    ↓
produce domain event(s)
    ↓
persist transaction
    ↓
return result
```

MCP is merely one caller of these commands.

`ironmaintctl` should use the same runtime methods.

---

# 39. Runtime Query Pattern

Queries do not mutate domain state.

Examples:

```text
get job
get next actions
list evidence
list gates
get operation
read artifact
read workspace status
```

The query layer may use persisted projections.

---

# 40. Optimistic Job Concurrency

Every mutating job command should require or internally verify:

```text
expected_job_version
```

The response returns:

```text
job_version
```

A stale write returns a conflict, not a silent retry.

The model can then refresh `job.get`.

---

# 41. Reconciliation

Implement a deterministic:

```text
runtime.reconcile(job_id)
```

operation.

Purpose:

Advance any state that requires **no additional agent decision**.

Example:

```text
all mandatory BuildValidation gates now pass
        ↓
reconcile
        ↓
transition to PackageQa
```

Reconciliation may continue through multiple trivially satisfied stages until reaching a state requiring:

```text
tool execution
agent repair
human review
approval
```

This reduces pointless LLM calls.

IronClaw should not need to say:

```text
please move from A to B
please move from B to C
```

when deterministic software already knows those transitions are valid.

---

# 42. `NextAction` Model

This is the principal agent-facing planning abstraction.

Define a runtime type roughly equivalent to:

```rust
pub struct JobNextActions {
    pub job_id: JobId,
    pub job_version: u64,
    pub state: JobState,
    pub candidate: Option<CandidateFingerprint>,
    pub blockers: Vec<ActionBlocker>,
    pub actions: Vec<AllowedAction>,
}
```

Possible allowed actions:

```text
InspectFailure
RunCheck
ReadWorkspace
ApplyPatch
CaptureCandidate
RetryInterruptedCheck
RequestHumanReview
WaitForExternalCondition
NoAction
```

Do not expose arbitrary prose only.

Provide structured fields plus short explanatory text.

---

# 43. Agent Repair Loop

The expected 0B reasoning loop becomes:

```text
IronClaw
   │
   ▼
job.next_actions
   │
   ├── RunCheck
   │       ↓
   │   check.run
   │       ↓
   │      FAIL
   │
   ▼
job.next_actions
   │
   ├── InspectFailure
   │       ↓
   │   evidence/artifact read
   │
   ├── ReadWorkspace
   │
   └── ApplyPatch
           ↓
     workspace dirty
           ↓
    candidate.capture
           ↓
       candidate C2
           ↓
        check.run
           ↓
          PASS
           ↓
       reconcile
```

This is the first real realization of the IronMaint architecture.

---

# 44. Check Model

Add a runtime concept:

```rust
pub struct CheckDefinition {
    pub id: CheckId,
    pub candidate: CandidateFingerprint,
    pub capability: ToolCapabilityKey,
    pub evidence_kind: EvidenceKind,
    pub gate_id: GateId,
    pub mandatory: bool,
}
```

`CheckId` should be a typed UUIDv7 newtype.

This is a generic additive Phase 0B core identifier and does not violate Phase 0A distribution neutrality.

---

# 45. Check Planning

Adapters continue to produce `PlannedCheck`.

Runtime materializes these into durable `CheckDefinition` records tied to:

```text
candidate
gate
tool capability
```

The MCP caller runs:

```text
check_id
```

not:

```text
ToolCapabilityKey chosen arbitrarily by the model
```

This prevents an agent from invoking checks not planned for the candidate.

---

# 46. Check Normalization

A tool definition includes a trusted normalizer.

Conceptually:

```rust
pub trait ResultNormalizer {
    fn normalize(
        &self,
        execution: &ExecutionRecord,
    ) -> Result<NormalizedResult, NormalizationError>;
}
```

The normalizer creates:

```text
EvidenceStatus
diagnostic summary
relevant artifact references
```

The LLM does not interpret exit code into authoritative pass/fail state.

---

# 47. Fixture Tool

Create:

```text
ironmaint-fixture-tool
```

as a tiny deterministic executable used only for testing 0B.

Fixture source tree:

```text
package.conf
```

Example broken state:

```text
name=fixture
build_ready=false
qa_ready=false
```

Fixture checks:

```text
fixture.build.validate
fixture.qa.validate
```

Build succeeds only if:

```text
build_ready=true
```

QA succeeds only if:

```text
qa_ready=true
```

It should print human-readable diagnostics useful to the agent.

---

# 48. Fixture Policy

The stub distribution adapter should generate at least one synthetic mandatory policy obligation.

Example:

```text
POLICY-FIXTURE-001

Mandatory:
package.conf must contain:
policy_version=1
```

A deterministic fixture policy evaluator produces the evidence.

This is not an LLM judgment.

---

# 49. Phase 0B Synthetic Workflow

Seed a repository:

```text
package.conf

name=fixture
build_ready=false
qa_ready=false
policy_version=0
```

Expected agent work:

```text
build check FAIL
       ↓
change build_ready=true
       ↓
capture C2
       ↓
build PASS
       ↓
QA FAIL
       ↓
change qa_ready=true
       ↓
capture C3
       ↓
build reruns for C3
       ↓
build PASS
       ↓
QA PASS
       ↓
policy FAIL
       ↓
change policy_version=1
       ↓
capture C4
       ↓
affected checks rerun
       ↓
all PASS
       ↓
release candidate assembled
       ↓
FinalValidation
       ↓
READY_FOR_APPROVAL
```

This deliberately tests evidence invalidation across multiple candidate generations.

---

# 50. MCP Architecture

Implement:

```text
ironmaint-mcp
```

using the official Rust MCP SDK.

Expose it through `ironmaintd`.

Initial transport:

```text
Streamable HTTP
```

bound by default to:

```text
127.0.0.1
```

No public listener by default.

IronClaw currently supports MCP as part of its tool registry, so this keeps the integration at the intended capability boundary rather than embedding IronClaw libraries directly into IronMaint.

---

# 51. MCP Authentication

Even loopback transport must be authenticated.

At first startup, generate a random token:

```text
auth/mcp-token
```

Permissions:

```text
0600
```

Requests require:

```text
Authorization: Bearer <token>
```

Do not:

```text
print token in normal logs
persist token into SQLite
return token from MCP
place token inside job artifacts
```

An explicit development-only insecure mode may exist but must emit a prominent warning and bind only to loopback.

---

# 52. MCP Scope

The MCP API is a **maintenance capability API**.

It is not:

```text
SQL interface
filesystem RPC
shell service
generic process runner
Git remote
database debugger
```

Expose only operations required by IronMaint maintenance workflows.

---

# 53. Initial MCP Tool Surface

Read-only:

```text
ironmaint_job_get
ironmaint_job_next_actions

ironmaint_workspace_status
ironmaint_workspace_list
ironmaint_workspace_read
ironmaint_workspace_diff

ironmaint_candidate_get

ironmaint_check_list
ironmaint_operation_get

ironmaint_evidence_list
ironmaint_evidence_get

ironmaint_gate_list
ironmaint_obligation_list

ironmaint_artifact_read_text
```

Mutating:

```text
ironmaint_workspace_apply_patch

ironmaint_candidate_capture

ironmaint_check_run
ironmaint_check_retry

ironmaint_job_reconcile
```

Administrative job creation does not need to be exposed to the agent initially.

Use `ironmaintctl` or test setup to create jobs.

---

# 54. Explicitly Forbidden MCP Tools

Do not implement:

```text
ironmaint_event_append

ironmaint_state_set

ironmaint_gate_set_pass

ironmaint_evidence_create

ironmaint_obligation_set_pass

ironmaint_sql

ironmaint_exec

ironmaint_shell

ironmaint_git

ironmaint_artifact_write_arbitrary

ironmaint_approve

ironmaint_publish
```

These would destroy the trust boundary.

---

# 55. MCP Response Envelope

Every IronMaint tool result should return a structured envelope:

```json
{
  "schema_version": 1,
  "request_status": "ok",
  "job_id": "...",
  "job_version": 17,
  "data": {}
}
```

Errors:

```json
{
  "schema_version": 1,
  "request_status": "error",
  "error": {
    "kind": "conflict",
    "message": "...",
    "retryable": true
  }
}
```

Error kinds should include:

```text
validation
not_found
conflict
blocked
unauthorized
unsupported
infrastructure
internal
```

Do not make the model infer error class from prose.

---

# 56. MCP Schema Discipline

Generate JSON Schema for tool inputs and structured outputs.

Use typed Rust request/response objects.

Do not build tool schemas manually as arbitrary JSON.

Schema snapshots belong under:

```text
schemas/mcp/
```

`cargo xtask verify-schemas` should include these.

---

# 57. MCP Long-Running Behavior

Phase 0B deliberately does not depend upon MCP task/subscription behavior.

A `check.run` tool may:

1. execute synchronously if bounded and short enough; or
2. create an `OperationId` and return it for polling.

Prefer option 2 architecturally.

Recommended behavior:

```text
check.run
    ↓
OperationId
    ↓
operation.get
```

This avoids tying package-build duration to one MCP request.

The 0B fixture tool will complete quickly, but the interface must be suitable for future multi-minute or multi-hour package builds.

---

# 58. Operation Polling

`ironmaint_operation_get` returns:

```text
operation ID
state
started_at
completed_at
candidate
check
artifacts
normalized result if available
```

IronClaw may poll.

A future notification/subscription layer may replace polling without changing operation semantics.

---

# 59. IronClaw Integration Strategy

Do not fork or modify IronClaw.

IronMaint integrates as an external MCP server.

Configuration conceptually:

```text
IronClaw Tool Registry
        │
        ▼
IronMaint MCP endpoint
```

The IronClaw agent receives only the IronMaint capabilities required for this workflow.

Generic shell or host filesystem access should not be required for package maintenance through IronMaint.

---

# 60. OS-Level Separation

Preferred deployment:

```text
ironclaw user
    cannot directly access
    IronMaint workspaces

ironmaint user
    owns:
      database
      workspaces
      artifacts
```

Communication occurs through authenticated MCP.

Development mode may run both under one account.

Production architecture must not assume shared filesystem authority.

---

# 61. IronClaw Skill

Create:

```text
integrations/ironclaw/skills/ironmaint-maintainer/SKILL.md
```

Its behavioral contract should tell the agent:

```text
The IronMaint MCP state is authoritative.

Always inspect job_next_actions before deciding what maintenance action is allowed.

Never claim a gate passed based on reasoning.

Run the check.

When a check fails:
  inspect its evidence and logs;
  inspect relevant source;
  modify only through workspace tools;
  capture a new candidate;
  rerun required checks.

Do not attempt to bypass blockers.

Stop when IronMaint reports:
  HumanReviewRequired
  ReadyForApproval
  unsupported privileged action

Do not use generic shell or filesystem tools to manufacture package evidence.
```

The skill should remain concise.

The tool API itself must enforce these rules even if the model ignores the skill.

---

# 62. IronClaw Delegation

Phase 0B uses one primary maintainer agent.

Do not implement a swarm.

Do not automatically spawn subagents.

IronClaw currently supports parallel jobs and ongoing work around bounded capability orchestration, but IronMaint should first prove the single-agent state/evidence loop before introducing delegation complexity.

Future specialist workers may handle:

```text
policy
licensing
bug analysis
build diagnosis
```

but not in 0B.

---

# 63. Correlation with IronClaw

Persist an optional orchestration correlation record:

```rust
pub struct OrchestratorRef {
    pub kind: String,
    pub external_thread: Option<String>,
    pub external_job: Option<String>,
}
```

For IronClaw:

```text
kind = "ironclaw"
```

These identifiers are informational correlation metadata.

They do not grant authority.

---

# 64. MCP Session Audit

Record:

```text
MCP client name/version
server-generated session identifier
tool name
request time
job ID
result classification
duration
```

Do not persist full prompts or private LLM context unless explicitly required.

IronMaint's audit trail records actions, not the agent's hidden reasoning.

---

# 65. Runtime Audit Events

In addition to Phase 0A domain events, produce operational audit entries for:

```text
daemon start
daemon stop
MCP authentication failure
MCP tool invocation
executor start
executor completion
executor interruption
artifact stored
workspace mutation
candidate capture
recovery action
```

Operational audit records are not necessarily domain events.

Keep the distinction explicit.

---

# 66. Configuration

Add an `IronMaintConfig`.

Conceptual values:

```toml
[state]
dir = "..."

[database]
path = "ironmaint.db"

[executor]
max_parallel_jobs = 4
default_timeout_seconds = 900
stdout_max_bytes = 10485760
stderr_max_bytes = 10485760

[mcp]
listen = "127.0.0.1:9876"
token_file = ".../auth/mcp-token"

[workspace]
root = ".../workspaces"

[artifacts]
root = ".../artifacts"
```

Support environment overrides for deployment automation.

Do not use environment variables directly inside domain crates.

---

# 67. Startup Sequence

`ironmaintd` startup:

```text
load configuration
      ↓
acquire state-directory lock
      ↓
validate/create directories
      ↓
open SQLite
      ↓
run migrations
      ↓
verify artifact store
      ↓
recover interrupted operations
      ↓
load adapter registry
      ↓
load tool registry
      ↓
bind MCP
      ↓
READY
```

If any authoritative subsystem fails initialization, daemon startup fails.

Do not run partially.

---

# 68. Shutdown

On graceful termination:

```text
stop accepting new commands
      ↓
mark/cancel active local executions
      ↓
persist final operation states
      ↓
flush artifact writes
      ↓
close database
      ↓
release daemon lock
```

A forced crash is handled by restart recovery.

---

# 69. Health

Expose non-sensitive operational health information.

At minimum:

```text
process alive
database accessible
artifact store writable
state directory locked correctly
migration version supported
```

MCP health should not reveal:

```text
auth token
credentials
package source
agent prompts
```

---

# 70. `ironmaintctl`

Provide a human/operator CLI using the same runtime/store contracts.

Initial commands:

```text
ironmaintctl init

ironmaintctl fixture create

ironmaintctl job show <job-id>
ironmaintctl job events <job-id>
ironmaintctl job next <job-id>

ironmaintctl operation show <id>

ironmaintctl workspace status <job-id>

ironmaintctl rebuild-projections

ironmaintctl doctor
```

Avoid implementing duplicate business logic in the CLI.

---

# 71. `doctor`

`ironmaintctl doctor` should verify:

```text
state directory
database
migration level
artifact integrity sample
workspace root
configured Git executable
fixture tool
MCP configuration
```

Later phases can add:

```text
sbuild
mock
etc.
```

---

# 72. Persistence Recovery Test

Required scenario:

```text
create job
advance state
create candidate
record evidence
stop daemon
start daemon
```

Expected:

```text
same job version
same state
same active candidate
same evidence
same events
same artifact digests
```

---

# 73. Projection Reconstruction Test

For each fixture job:

```text
persist events + projection
        ↓
save expected projection
        ↓
delete/rebuild projections
        ↓
replay events
```

Result must exactly match expected projection.

---

# 74. Crash-During-Execution Test

Scenario:

```text
start long-running fixture check
        ↓
confirm Running persisted
        ↓
kill daemon
        ↓
restart
```

Expected:

```text
operation = Interrupted
no PASS evidence created
no FAIL evidence fabricated
job remains blocked
safe retry offered
```

---

# 75. Candidate Staleness Test

Scenario:

```text
candidate C1
build PASS

workspace edit
capture C2
```

Expected:

```text
C1 evidence remains historically valid
C1 evidence does not satisfy C2 gate
C2 build gate not satisfied
```

The system must force rerun.

---

# 76. MCP Authorization Test

Without valid token:

```text
initialize/tool call
→ rejected
```

With valid token:

```text
allowed
```

Auth failures must not reveal the expected token.

---

# 77. MCP Mutation Concurrency Test

Scenario:

```text
workspace revision 5

request A:
expected revision 5

request B:
expected revision 5
```

Only one succeeds.

The other receives:

```text
conflict
current_revision = 6
```

---

# 78. Job Version Concurrency Test

Equivalent test for authoritative job state.

Stale expected version cannot append a mutation.

---

# 79. Path Traversal Test

All must fail:

```text
../../etc/passwd

/etc/passwd

source/../../secret

symlink-outside/file
```

No external file may be read or written.

---

# 80. Arbitrary Execution Test

There must be no MCP mechanism by which an agent can cause:

```text
/bin/bash
curl arbitrary URL
python arbitrary code
git arbitrary args
```

to execute.

Only registered capability definitions are executable.

---

# 81. Fixture End-to-End Without LLM

Before IronClaw integration, implement a deterministic driver test.

Driver follows exactly the actions an agent would take:

```text
create fixture job
next_actions
run failing check
read evidence
patch
capture
rerun
...
```

It must reach:

```text
ReadyForApproval
```

This proves the runtime independently of LLM behavior.

---

# 82. MCP End-to-End Without IronClaw

Use an MCP client in integration tests.

Run the same fixture workflow through:

```text
MCP tool calls only
```

Expected result:

```text
ReadyForApproval
```

This proves the MCP boundary separately.

---

# 83. Actual IronClaw End-to-End

Then run the full scenario:

```text
IronClaw
   ↓
IronMaint MCP
   ↓
fixture job
```

Prompt concept:

```text
Maintain IronMaint job <ID>.
Use the IronMaint tools and continue until IronMaint reports
ReadyForApproval or HumanReviewRequired.
```

The agent must autonomously:

```text
inspect next actions
run checks
read failures
modify package.conf
capture candidates
rerun invalidated checks
reconcile workflow
```

Final authoritative state:

```text
ReadyForApproval
```

Do not make CI dependent upon a paid/external LLM.

This test can be an opt-in:

```text
IRONMAINT_E2E_IRONCLAW=1
```

test or script.

The non-LLM runtime and MCP tests remain mandatory CI.

---

# 84. Restart During Agent Workflow

Manual/optional E2E:

```text
IronClaw starts repair job
        ↓
stop ironmaintd
        ↓
restart ironmaintd
        ↓
agent refreshes job
        ↓
continues
```

No package state may be lost.

IronClaw may lose conversational convenience.

IronMaint must not lose maintenance truth.

---

# 85. Logging

Phase 0B may introduce:

```text
tracing
tracing-subscriber
```

Logs should include structured fields:

```text
job_id
candidate_id
operation_id
tool capability
duration
result
```

Never log:

```text
MCP bearer token
future credentials
full secret-bearing environment
```

---

# 86. Dependencies

New dependencies may reasonably include:

```text
tokio
sqlx with SQLite
rmcp
tracing
tracing-subscriber
fs2 or equivalent file locking
tempfile
```

Retain:

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

Do not introduce a web framework merely for an unrelated REST API.

Use the HTTP facilities required by the MCP server.

---

# 87. Async Boundary

Unlike Phase 0A, Phase 0B is an async runtime.

Async belongs in:

```text
store implementations
executor
artifact I/O where useful
MCP server
runtime application service
daemon
```

Keep pure domain logic synchronous.

Do not retrofit async into:

```text
CandidateFingerprint
state transition evaluation
obligation evaluation
version/value validation
```

---

# 88. Unsafe Rust

Continue:

```text
#![forbid(unsafe_code)]
```

wherever practical.

Phase 0B does not justify unsafe Rust.

---

# 89. Phase 0B.1 — Persistence Foundation

Implement:

```text
ironmaint-store
ironmaint-store-sqlite
migrations
event persistence
projection persistence
replay/rebuild
daemon lock
```

### Exit checkpoint

A Phase 0A fixture job survives:

```text
create
persist
shutdown
restart
reconstruct
```

with byte-equivalent serialized domain state where applicable.

---

# 90. Phase 0B.2 — Artifact Store

Implement:

```text
content-addressed SHA-256 store
atomic writes
digest verification
artifact metadata persistence
size limits
```

### Exit checkpoint

Executor-independent artifact tests pass, including corruption detection.

---

# 91. Phase 0B.3 — Workspace and Candidate Runtime

Implement:

```text
job workspaces
revision concurrency
path confinement
read/list/diff
apply_patch
minimal controlled Git operations
candidate capture
candidate activation
```

### Exit checkpoint

A fixture repository can produce:

```text
C1
edit
C2
edit
C3
```

with immutable fingerprints and preserved history.

---

# 92. Phase 0B.4 — Executor

Implement:

```text
ToolRegistry
ToolDefinition
ExecutionRequest
ExecutionRecord
ProcessExecutor
timeouts
log capture
operation persistence
recovery states
fixture tool
result normalization
```

### Exit checkpoint

Fixture checks can:

```text
PASS
FAIL
TIMEOUT
INTERRUPT
INFRASTRUCTURE_FAIL
```

and each condition is represented correctly.

---

# 93. Phase 0B.5 — Runtime Application Service

Implement:

```text
command/query boundary
check planning
check materialization
gate evaluation
evidence recording
candidate switching
job reconciliation
next-actions calculation
optimistic concurrency
```

### Exit checkpoint

The no-LLM fixture driver reaches `ReadyForApproval`.

---

# 94. Phase 0B.6 — MCP Server

Implement:

```text
authenticated MCP transport
typed tool schemas
read tools
workspace mutation tools
candidate capture
check execution
operation query
reconcile
next-actions
```

### Exit checkpoint

An automated MCP client completes the fixture scenario without direct store access.

---

# 95. Phase 0B.7 — IronClaw Integration

Implement:

```text
IronClaw setup documentation
MCP registration example
IronMaint maintainer skill
tool permission guidance
optional actual-agent E2E script
```

### Exit checkpoint

An actual IronClaw agent can successfully repair the fixture package and drive it to:

```text
ReadyForApproval
```

without:

```text
direct DB access
direct workspace host access
arbitrary authoritative shell execution
state fabrication
```

---

# 96. Phase 0B.8 — Recovery and Hardening

Implement and test:

```text
daemon restart
interrupted operations
projection reconstruction
stale candidate evidence
workspace conflicts
job version conflicts
MCP auth failure
artifact corruption
path traversal
duplicate requests
```

### Exit checkpoint

All runtime invariants remain true under restart and common failure modes.

---

# 97. Required CI Commands

Phase 0A commands remain mandatory:

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

Add:

```text
cargo xtask verify-migrations

cargo xtask verify-mcp-schemas

cargo test --workspace --features integration
```

where practical.

---

# 98. Architecture Verification Extensions

`verify-architecture` must now also reject:

```text
ironmaint-core
  → store implementation

ironmaint-core
  → executor

ironmaint-core
  → MCP

ironmaint-state
  → SQLite

adapter crate
  → SQLite directly

ironmaint-mcp
  → SQLite directly
```

Correct flow:

```text
MCP
 ↓
Runtime
 ↓
Store contract
 ↓
SQLite implementation
```

MCP must never bypass runtime.

---

# 99. Codex Must Not Implement Yet

Phase 0B explicitly excludes:

```text
real Debian Policy retrieval
real Fedora guidelines retrieval

sbuild
lintian
autopkgtest
piuparts
reprotest

rpmlint
Mock
Koji
Bodhi
Packit

Debian BTS
Bugzilla

git-buildpackage
uscan

real release assembly
signing
package publication

multi-agent swarm
automated delegation
remote build farm
Kubernetes
distributed workers
PostgreSQL
container sandboxing
network policy enforcement
```

The architecture may reserve interfaces for them.

Do not implement them.

---

# 100. Additions to Phase 0A Core

Only minimal additive domain changes are permitted:

```text
CheckId

WorkspaceRevision if placed in core rather than workspace crate

operation/correlation identifiers if required
```

Do not redesign completed 0A types merely for runtime convenience.

If runtime implementation exposes a genuine flaw in 0A, document it as:

```text
PHASE-0A-CONTRACT-CHANGE
```

with:

```text
old contract
problem
minimal proposed change
Debian fit
Fedora fit
migration impact
```

before altering the architecture.

---

# 101. Required Synthetic Acceptance Scenario

Phase 0B is not complete until the following exact conceptual scenario passes:

```text
1. Create fixture repository.

2. Create MaintenanceJob.

3. Capture candidate C1.

4. Restart daemon.

5. Confirm C1 and job state survived.

6. Run build check against C1.
   Result: FAIL.

7. Persist logs and FAIL evidence.

8. IronClaw reads failure.

9. IronClaw patches workspace.

10. Capture C2.

11. Verify C1 evidence does not satisfy C2 gates.

12. Run build against C2.
    Result: PASS.

13. Run QA against C2.
    Result: FAIL.

14. IronClaw patches workspace.

15. Capture C3.

16. Required C3 checks rerun.

17. QA passes.

18. Mandatory synthetic policy obligation fails.

19. IronClaw patches workspace.

20. Capture C4.

21. Relevant evidence is invalidated.

22. Required checks rerun.

23. All mandatory gates pass.

24. Policy obligation passes.

25. Runtime reconciles workflow.

26. Synthetic release candidate created.

27. Final validation bound to C4.

28. Job reaches ReadyForApproval.

29. No signing or publication operation executes.

30. Entire history is reconstructable after restart.
```

---

# 102. Definition of Done

Phase 0B is complete only when all statements are true:

1. IronMaint has durable SQLite persistence.
2. Domain events are append-only.
3. Job projections can be rebuilt from events.
4. Jobs survive daemon restart.
5. Candidates survive daemon restart.
6. Evidence survives daemon restart.
7. Artifacts are content-addressed and integrity checked.
8. Job workspaces are isolated.
9. Workspace mutations use optimistic revision checks.
10. Path traversal is blocked.
11. Dirty workspaces cannot directly generate authoritative evidence.
12. Candidates can be captured from workspaces.
13. Checks can execute only through registered capability definitions.
14. The agent cannot provide arbitrary executables or shell commands.
15. Process failures are distinct from infrastructure failures.
16. Check operations are persisted.
17. Interrupted executions recover as `Interrupted`, not fabricated `Fail`.
18. Evidence is generated by trusted normalizers.
19. Evidence remains candidate-bound.
20. Old evidence cannot authorize new candidates.
21. Runtime can deterministically calculate next actions.
22. Runtime can deterministically reconcile state.
23. MCP is authenticated.
24. MCP tools expose domain capabilities rather than storage primitives.
25. MCP cannot directly set state, gates, obligations, approvals, or evidence.
26. IronClaw can call the MCP server.
27. An IronClaw skill describes the maintenance interaction model.
28. A deterministic no-LLM driver completes the synthetic workflow.
29. An MCP-client integration test completes the synthetic workflow.
30. An actual IronClaw agent can complete the synthetic repair workflow.
31. The final state is `ReadyForApproval`.
32. No real Debian or Fedora maintenance tool is required.
33. No release credential exists in the agent environment.
34. All 0A architecture invariants continue to pass.

---

# 103. Recommended Codex Commit Boundaries

```text
phase0b: add persistence contracts and SQLite event store

phase0b: add projection persistence and replay

phase0b: add content-addressed artifact store

phase0b: add isolated workspace manager

phase0b: add controlled Git candidate capture

phase0b: add executor and registered tool model

phase0b: add durable operation lifecycle and recovery

phase0b: add fixture check executable and normalizers

phase0b: add runtime command and query services

phase0b: add deterministic next-actions and reconciliation

phase0b: add authenticated MCP server

phase0b: expose IronMaint maintenance capabilities over MCP

phase0b: add IronClaw skill and integration documentation

phase0b: add synthetic maintenance end-to-end tests

phase0b: harden recovery, concurrency, and path confinement
```

Every commit should pass all applicable workspace tests.

---

# 104. Codex Completion Report

When complete, Codex should report:

```text
workspace changes

new crates/binaries

database schema and migrations

domain changes from 0A, if any

MCP tools exposed

tool registry capabilities

executor behavior

workspace security behavior

artifact behavior

restart/recovery behavior

test inventory

test results

actual IronClaw E2E result

architecture verification result

schema verification result

known limitations

questions requiring review before real Debian integration
```

---

# 105. North-Star Test

At the end of 0B, this statement must be accurate:

> **IronClaw can autonomously diagnose and repair a synthetic package-maintenance failure, but it cannot directly alter IronMaint's truth. Every source change becomes an immutable candidate, every authoritative result comes from a registered deterministic capability, every state transition is decided by IronMaint, and the entire maintenance transaction survives process restart.**

And this must also be accurate:

> **Replacing IronClaw with another MCP-capable orchestrator would not require changing IronMaint's package state, evidence model, persistence model, executor, or release semantics.**

---

# 106. Handoff to Phase 1

After Phase 0B passes, review the implementation before connecting real Debian infrastructure.

Then Phase 1 can replace synthetic capabilities incrementally:

```text
fixture package inspection
        ↓
real Debian package inspection

fixture policy authority
        ↓
Debian Policy / Developer's Reference

fixture build check
        ↓
Debian native checks

fixture issue provider
        ↓
Debian BTS read-only
```

The important change is that these become **adapter and tool implementations**.

They should not require another redesign of:

```text
runtime
persistence
executor
candidate identity
evidence
state ownership
MCP boundary
IronClaw integration
```

If Phase 1 requires such a redesign, Phase 0B has not fully succeeded.

