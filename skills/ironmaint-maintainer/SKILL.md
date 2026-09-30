---
name: ironmaint-maintainer
version: "0.1.0"
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

Every action goes through one of these ten MCP tools:

| Tool | When |
|---|---|
| `job.create` | Once per session, to open the job |
| `job.get` | Before every decision — read the canonical projection |
| `job.next_actions` | To enumerate what the runtime will currently accept |
| `job.reconcile` | When `next_actions` returns no actionable blocker |
| `candidate.capture` | When the active candidate must change |
| `check.run` | To execute the next gate's check (use the `check_id` from `next_actions`) |
| `workspace.apply_patch` | To apply a candidate patch to the workspace |
| `workspace.stat` | To read the current `WorkspaceRevision` before patching |
| `operation.get` | To inspect a privileged operation's status |
| `job.resume` | Only when `next_actions` offers `resume_job` — returns the job to the state recorded when it was escalated |

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

`next_actions` returns **three** fields, and they mean different things.
`allowed` names the moves the runtime will accept *now* — every entry has a
tool behind it, and calling anything else is not your move. `requires_human`
names the one move that belongs to a person, or is absent. `blockers` names
what is standing in the way of the rest. A fresh job returns
`allowed: ["capture_candidate"]` with no blockers — so a loop that only
inspects `blockers` sees nothing to do and stops on a job that has barely
started.

If `requires_human` is set, **stop.** Do not look for a tool that performs
it. Two of its values (`approve_release`, `authorize_publication`) have no
tool in 0B at all, and the phase forbids adding one.

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

`reconcile` advances at most **one** transition per call. A job walks the
chain only if you call it again.

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

In Phase 0B, `reconcile` returns `no_op` for a job that has no active
candidate, and no tool sets one: the runtime commands that do
(`SetActiveCandidate`, `MaterializeChecks`) are driven by a distribution
adapter, and there is no adapter yet. A real Debian or Fedora adapter is
what seeds the checks the state machine walks, and it is Phase 1 work.

So today a correct session is: create the job, capture a candidate, read
the projection, and report honestly that the job cannot advance further
until an adapter exists. **Do not improvise past this** — there is no
supported way to set an active candidate from the tool surface, and
fabricating state is the one thing the rules above exist to prevent.

`doc/PHASE-0B-COMPLETION.md` §16.8 records this precisely.
