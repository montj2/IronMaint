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

Every action goes through one of these nine MCP tools:

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

## Workflow loop

```text
loop:
    projection := job.get(job_id)
    if projection.state == ReadyForApproval:
        return SUCCESS
    actions := job.next_actions(job_id)
    if actions has blockers:
        drive the first blocker (see below)
        continue
    outcome := job.reconcile(job_id)
    match outcome:
        Advanced → continue
        NoOp | Exceptional | NeedsActorDecision | Blocked → stop, narrate to user
```

## Driving blockers

`next_actions` enumerates blockers in priority order. Resolve them in the
order the runtime returns them:

- **`GatePending { gate_id, tool_key }`** → `check.run(check_id)` where
  `check_id` is the `CheckDefinition` the runtime attached to the gate.
  Wait for the result; the runtime will record evidence and update the
  `GateResult` automatically.
- **`GateFailed { gate_id, status }`** → `status == Fail` is the normal
  outcome of a tool that legitimately couldn't build the package. Read the
  `Evidence` rows the runtime attached and decide whether to:
  - capture a new `SourceCandidate` (e.g., upstream fixed the bug), then
    loop from the top;
  - apply a `workspace.apply_patch` and re-run the gate; or
  - stop and narrate the failure to the user.
- **`ObligationPending { obligation_ref }`** → read the obligation via the
  `policy/obligation` MCP lookup (when wired); consult the policy
  reference; either satisfy it via the documented check or stop and
  narrate to the user. The agent never self-grants `ExceptionApproved`.
- **`MissingApproval { requirement }`** → stop. `ReadyForApproval` is the
  agent's exit checkpoint. The user (a human maintainer) is the only
  principal that may record an approval decision.

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
7. **No self-approval.** You may *request* approval; you may not *grant*
   it.
8. **Stop at `ReadyForApproval`.** `Approved` and onward require a
   privileged operation authorised by the privileged service. The agent
   does not have that authority.

## Conflict handling

- **`409 stale_projection` from `job.reconcile`** → re-call `job.get` to
  refresh; the orchestrator advanced the job. Resume the loop from the
  new projection.
- **`409 stale_revision` from `workspace.apply_patch`** → re-call
  `workspace.stat` to refresh the revision; the workspace was modified
  by another actor.
- **`Failed` gate result** → read the evidence, narrate the failure, and
  ask the user how to proceed. Do not silently retry.

## Exit checkpoint

The job is "ready" when `job.get` returns `state == "ready_for_approval"`.
At that point:

1. Stop the loop.
2. Call `job.next_actions` one more time to confirm the only allowed
   action is `RequestApproval`.
3. Narrate to the user: "IronMaint job `<job_id>` is at
   `ReadyForApproval`. The next step is a human approval. The
   `RequiredApproval` is `<approval_id>`."

Do not call `RequestApproval` on the user's behalf. The user must invoke
the approval themselves.
