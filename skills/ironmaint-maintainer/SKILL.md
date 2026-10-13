---
name: ironmaint-maintainer
version: "0.2.0"
description: Drives a Debian or Fedora package from EventDetected to ReadyForApproval through the IronMaint MCP server. Use when the user asks to advance, repair, push through gates, or otherwise progress a package through IronMaint.
activation:
  keywords:
    - ironmaint
    - "package maintenance"
    - "drive through gates"
    - "release review"
    - "approval gate"
    - "reconcile"
  patterns:
    - "(?i)(drive|advance|push|repair|walk)\\s+.*(ironmaint|package|job)"
    - "(?i)(event_detected|ready_for_approval|final_validation|release_review)\\b"
  tags:
    - packaging
    - ironmaint
  max_context_tokens: 2400
requires:
  skills: []
  bins: []
  env: []
---

# IronMaint Maintainer

Drive an IronMaint job from `EventDetected` to `ReadyForApproval` by
calling the IronMaint MCP server. The runtime is the only writer of
`JobState`, `GateStatus`, `ObligationStatus`, `ApprovalStatus`, and
`PublicationStatus`. You propose; the engine decides.

## Tool surface

Every action goes through one of these fourteen MCP tools:

| Tool | When |
|---|---|
| `job.create` | Once per session, to open the job |
| `job.get` | Before every decision — read the canonical projection |
| `job.next_actions` | To enumerate what the runtime will currently accept |
| `job.reconcile` | When `next_actions` returns no actionable blocker |
| `candidate.capture` | When the active candidate must change |
| `check.run` | To execute the next gate's check (use the `check_id` from `next_actions`) |
| `evidence.list` | To enumerate every evidence row attached to a job (optionally narrowed to one candidate) — the readable substrate of the `check.run` verdict |
| `evidence.get` | To read one evidence row by `evidence_id` — the row carries the `EvidenceKind`, the `EvidenceStatus`, the producer, and the bound artifact ids |
| `evidence.artifact.read` | To read the bytes of a structured report artifact bound to an evidence row — the §18 source analysis, the §26 maintenance context, the §22 policy delta, and the §23 BTS snapshot all live here |
| `workspace.apply_patch` | To apply a candidate patch to the workspace |
| `workspace.stat` | To read the current `WorkspaceRevision` before patching |
| `operation.get` | To inspect a privileged operation's status |
| `job.resume` | Only when `next_actions` offers `resume_job` — returns the job to the state recorded when it was escalated |
| `release.candidate.create` | At `ReadyForApproval`, to assemble the snapshot of what was validated. It publishes nothing |

## Workflow loop

```text
loop:
    projection := job.get(job_id)
    if projection.state == ReadyForApproval:
        return SUCCESS
    if projection.state == HumanReviewRequired or
       projection.state == InfrastructureBlocked:
        stop and narrate — a human is needed (see below)
    actions := job.next_actions(job_id)
    if actions.allowed contains capture_candidate:
        candidate.capture(job_id, package, repository_url)   # see below
        continue
    if actions.allowed contains resume_job:
        do not call it — see "Human review" below
    if actions.blockers is non-empty:
        drive the first blocker (see below)
        continue
    outcome := job.reconcile(job_id)
    match outcome:
        {"advanced": …}                    → continue
        {"no_op": {"current": S}}          → the state machine has no enabled
                                            transition out of S; stop and narrate
        exceptional | needs_actor_decision |
        blocked | concurrent_modification  → stop, narrate to the user
```

### Reading a check's verdict

After `check.run` returns an `evidence_id`, every claim about what the
check established must be sourced from the evidence row, not from
memory of the tool:

```text
check.run returns evidence_id
        ↓
evidence.get(evidence_id)                # the row's EvidenceKind, EvidenceStatus, producer, bound artifacts
        ↓
evidence.artifact.read(evidence_id, artifact_id)   # the structured report (§18, §22, §23, §26)
        ↓
understand the package from the report
        ↓
continue / reconcile
```

The `evidence_status` returned by `check.run` is the runner's first
classification; the evidence row is the durable fact, and the artifact
is the source of every value the runner's verdict summarised. Trust
the row, not the runner's prose.

## Pass, Fail, and Blocked are not the same word

The skill's single most important mental model, called out by the spec
because the Phase 1 evidence surface is rich enough to confuse:

```text
Pass     means the check successfully established its stated fact.
         It does NOT mean the package is "good".

         A BTS snapshot with serious open bugs can Pass — the
         snapshot's job is to enumerate bugs, not to declare the
         package broken.

         A Policy-delta report identifying required review can Pass
         — the report's job is to surface the delta, not to enforce
         it.

         A source-analysis report carrying four patches can Pass —
         the report's job is to inventory them, not to judge them.

Fail     means the candidate itself violated the check contract.

         The changelog's source name does not match the candidate's
         source name. The control file is unparsable. A patch
         referenced in `debian/patches/series` does not exist on
         disk. A check that exists to enforce a structural fact found
         the structure broken.

Blocked  means the check could not establish its fact at all.

         The workspace path is unreadable. A parser binary is
         missing. The BTS host returned malformed JSON. The check
         that exists to report on something could not even read the
         thing it was supposed to report on. A Blocked verdict
         becomes an `InfrastructureError` in the normalizer and an
         `ExecutorError::InfrastructureFailed` in the runtime —
         distinct from a Fail so a future repair loop can react
         correctly.
```

The three are different in what the next move is:

- **Pass** → read the report, decide whether the *fact* the check
  established means a real problem (BTS bugs are real; Standards-Version
  delta is real; four patches is real). The check is not the
  judgement; the report is.
- **Fail** → the check found the candidate broken. Read the report's
  diagnostics, decide whether to patch the workspace, capture a new
  candidate, or narrate the failure.
- **Blocked** → the check could not answer. Re-run it later, or
  surface an operator. Do not patch the workspace in response to a
  Blocked verdict — the package may be fine; the check could not see
  it.

`next_actions` returns **three** fields, and they mean different things.
`allowed` names the moves the runtime will accept *now* — every entry has a
tool behind it, and calling anything else is not your move. `requires_human`
names the one move that belongs to a person, or is absent. `blockers` names
what is standing in the way of the rest. A fresh job returns
`allowed: ["capture_candidate"]` with no blockers — so a loop that only
inspects `blockers` sees nothing to do and stops on a job that has barely
started.

If `requires_human` is set, **stop.** Do not look for a tool that performs
it. Two of its values (`approve_release`, `authorize_publication`) have
no tool surface entry point, and the architecture forbids agents from
performing privileged operations directly.

`job.reconcile` answers with one of these shapes:

The whole `outcome` value is externally tagged, so the first key is the
variant:

| `outcome` | Shape | What to do |
|---|---|---|
| `advanced` | `{from, to, rule_index}` | continue the loop |
| `no_op` | `{current}` | no transition is currently enabled from `current`; stop and narrate |
| `blocked` | `{current, target, blockers: [string]}` | the rule's requirements are unmet; read the strings, stop |
| `needs_actor_decision` | `{current, target, blockers: [string]}` | a human must act; stop and narrate |
| `exceptional` | `{current}` | the job is in `human_review_required` or `infrastructure_blocked`; stop |
| `concurrent_modification` | — | another writer advanced the projection; re-read `job.get` and retry |

`reconcile` walks **every** satisfied rule in a single call. The
walker is bounded by the rule table's length and stops at the first
rule whose gate is unsatisfied, the first rule whose state is
exceptional, or a `ConcurrentModification`. The returned `advanced`
shape carries the *final* `to` state of the walk (i.e., the
"where the state machine stopped advancing" state), not an
intermediate one — the §41 spec and
`ReconcileOutcome::Advanced`'s own doc comment both promise that
single-call multi-step walk, and the runtime's `service.rs:1533`
walker implements it. A `no_op` reply on a fresh job is therefore
not "call again"; it is "no transition is currently enabled from
this state".

## Human review

`HumanReviewRequired` means a person has to look at this job. **Stop and
say so.** `next_actions` reports it as
`requires_human: {review_escalation}`, and you do not clear it.

You will also see `resume_job` in `allowed`, because the runtime only lists
what a tool can actually perform and there *is* a tool. Having one is not
permission. Resuming returns the job to the state that was recorded when
someone escalated it, so the recorded state is a human's decision — calling
it yourself would silently undo an escalation you have no basis to overrule.
`job.resume` is offered so the path exists and is discoverable to an
operator, not so an agent can take it.

The runtime will tell you when this has happened. A failed mandatory
obligation does **not** cause it: the runtime only escalates when
orchestration explicitly asks it to. A policy obligation that has not
passed yet is reported as a blocker, which is the state you can actually
work on — patch, capture a new candidate, re-run.

## Driving blockers

`next_actions` enumerates blockers in priority order. Resolve them in the
order the runtime returns them:

Blockers are externally tagged too, so you match on the first key:
`terminal`, `gate_pending`, `gate_failed`, `obligation_pending`,
`no_active_candidate`, `state_machine_blocked`.

- **`gate_pending {tool_key}`** → `check.run(check_id)`. Take the `check_id`
  from the `run_check` action that `next_actions` lists in `allowed`; **do not
  pass a tool name**. `tool_key` inside the blocker may be `null` — which
  check is pending is a fact about the job's materialised checks, so the
  runtime reports it in `allowed` rather than guessing a tool for you. Wait
  for the result; the runtime records the evidence and updates the gate
  verdict itself.
- **`GateFailed { gate_id, status }`** → `status == Fail` is the normal
  outcome of a tool that legitimately couldn't build the package. Read the
  `Evidence` rows the runtime attached and decide whether to:
  - capture a new `SourceCandidate` (e.g., upstream fixed the bug), then
    loop from the top;
  - apply a `workspace.apply_patch` and re-run the gate; or
  - stop and narrate the failure to the user.
- **`obligation_pending {reference}`** → consult the policy reference it
  names, then either satisfy it through the check the runtime points at or
  stop and narrate. **There is no obligation tool.** The ten tools are the
  whole surface and reading an obligation's body is not one of them; the
  only thing available to you is the reference string. You never self-grant
  `ExceptionApproved`.
- **`gate_failed {tool_key}`** → read the evidence behind the `Fail`
  verdict and decide, as below.
- **`no_active_candidate`** → `candidate.capture` first.
- **`terminal`** → the job is finished or cancelled. Stop.
- **`state_machine_blocked {message}`** → the engine refused the transition
  and the message says why. Stop and narrate it verbatim.
- **`requires_human` is not a blocker and you cannot clear it.** At
  `ReadyForApproval` the runtime reports `requires_human:
  {approve_release}` with `allowed: []` and *no* blockers. Do not wait for a
  blocker that will never arrive, and do not go looking for a tool that
  performs the approval — that state is your exit checkpoint, and a human
  maintainer is the only principal who can act on it.

## Hard rules

1. **No direct DB access.** Use `job.get` for projection; do not open
   the SQLite store directly.
2. **No host-fs mutation outside `workspace.apply_patch`.** No `cp`, `mv`,
   `rm`, `sed -i`, or shell redirection that touches the workspace.
3. **No arbitrary shell.** `check.run` is the only path that may execute
   an external tool, and only for `CheckDefinition`s the runtime minted.
4. **No state fabrication.** `JobState`, `GateStatus`, `ObligationStatus`,
   `ApprovalStatus`, and `PublicationStatus` are written by the runtime
   after a real tool run completes. You may read them; you do not write
   them.
5. **No evidence invention.** Evidence is recorded by the runtime after
   a real tool run. You may read it; you do not mint it.
6. **No candidate cross-evidence.** Capturing a new `SourceCandidate`
   mints a new fingerprint; evidence does not transfer.
7. **No self-approval.** You may not *grant* approval, and you may not
   *request* it either. `RequestApproval` is a runtime command that is
   refused with a typed `unsupported` error by design: an approval is a
   human principal's decision, and the orchestrator that drives you must
   not be able to manufacture one. It is not exposed as a tool, so
   calling it is not something you can do even by accident.
8. **Stop at `ReadyForApproval`.** `Approved` and onward require a
   privileged operation authorised by the privileged service. You do not
   have that authority.
9. **`candidate.capture` reports real source state.** Its fingerprint is
   derived from the working tree's actual HEAD commit and tree OID. Two
   captures of the same repository at *different* commits produce
   *different* fingerprints. A capture that returns the fingerprint you
   already have means the tree did not change — that is a real answer, not
   a cached one, and it is your signal to make an edit before capturing
   again.

## Reading a tool error

A failed call comes back with `isError: true` and a body shaped like

```json
{"error": {"kind": "invalid_input", "message": "…"}}
```

at HTTP 200. The same text appears in `structuredContent.error.message` and
in the first content block, so read whichever your client exposes. **The
`kind` is the thing to branch on** — `invalid_input` means fix your
arguments and retry, `conflict` means re-read state before retrying,
`runtime` means the call was well-formed and the runtime refused it on the
merits, `unsupported` means the runtime will never implement it. Retrying
an `invalid_input` unchanged will fail identically.

## Conflict handling

- **A conflict from `job.reconcile`** → re-call `job.get` to refresh; a
  parallel orchestrator advanced the job. Resume the loop from the new
  projection.
- **A conflict from `workspace.apply_patch`** → re-call `workspace.stat`
  to refresh the revision; the workspace was modified by another actor.
- **A `Fail` gate result** → read the evidence, narrate the failure, and
  ask the user how to proceed. Do not silently retry. A `Fail` verdict is
  the *normal* outcome of a package that genuinely does not build; it is
  not a transport error and not a reason to distrust the tool.

## Exit checkpoint

The job is "ready" when `job.get` returns `state == "ready_for_approval"`.
At that point:

1. Stop the loop.
2. Call `job.next_actions` once more. It will return
   `requires_human: {approve_release}`, an empty `allowed`, and no blockers.
3. Narrate to the user: "IronMaint job `<job_id>` is at
   `ReadyForApproval`. The next step is a human approval; I cannot request
   or grant one."

Do not try to act on `requires_human`. There is no tool for it, and the
runtime command behind it is refused by design.

## Where the loop currently stops

Phase 1 closed the read-only intake path through the state machine's
first six transitions: a real Debian job, against the hermetic
fixture, walks `EventDetected → Intake → SourceReview →
CandidateAssembly → SourceRevision → SourceIntegrity` in a single
`reconcile` call, and stops at `SourceIntegrity` because the next
rule (`SourceIntegrity → BuildValidation`) has no Phase 1 gate
(`doc/phases/PHASE-1.md` §14, §36; the §97 integration test in
`bins/ironmaintd/tests/startup_shutdown.rs` is the proof). The
reconcile return is `Blocked{current: source_integrity, target:
build_validation}` — the gate is named in the blocker string, not
guessed.

The five pre-build checks the intake path now exercises are:

| Routing key | EvidenceKind | Report kind |
|---|---|---|
| `debian.inspect.source_preparation` | `SourcePreparation` | `DebianSourcePreparationV1` |
| `debian.inspect.source_analysis` | `SourceIntegrity` | `DebianSourceReportV1` (§18) |
| `debian.inspect.maintenance_context` | `Maintenance` | `DebianMaintenanceContextV1` (§26) |
| `debian.policy.delta_review` | `PolicyEvaluation` | `DebianPolicyDeltaV1` (§20, §22) |
| `debian.issue.bts_snapshot` | `IssueCorrelation` | `DebianIssueReportV1` (§23, §24) |

After all five have produced `Pass` verdicts, every claim in the
maintainer intake summary is sourced from the bound artifacts; no
summary line is sourced from the LLM's memory (the §43 invariant).
The next Phase 2 stage — `BuildValidation` and the build / QA /
IssueWrite / publication capabilities that follow it — has no
implementation in this tree, and S6's seam check structurally
prevents the production Debian adapter from advertising any of
those capabilities. The seam check is the reason a future
patch cannot silently re-introduce a Phase 2 capability against
the intent of §11 and §106.
