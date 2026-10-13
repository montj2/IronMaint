# Tool permissions — what an IronClaw agent may and may not do while driving IronMaint

Phase 0B.7 / §95 of `doc/phases/PHASE-0B.md`.

IronMaint's safety story is built on a small number of explicit invariants
(PHASE-0A.md §4 and PHASE-0B.md §4). The agent's authority flows entirely
through the IronMaint MCP server; every other action is forbidden. The table
below is the canonical permission list. The IronMaint maintainer skill
(`skills/ironmaint-maintainer/SKILL.md`) repeats the same list verbatim so
the agent's prompt cannot drift from this document.

## Allowed MCP calls

| Tool | Authoritative purpose | Agent may call? |
|---|---|---|
| `job.create` | Open a new `MaintenanceJob` from a `MaintenanceEvent` | Yes — only entry to create a job |
| `job.get` | Read the current `JobProjection` (state, version, fingerprint) | Yes |
| `job.next_actions` | Enumerate `AllowedAction`s the runtime is willing to accept | Yes |
| `job.reconcile` | Advance the job through the static `TRANSITION_RULES` | Yes — but only when `next_actions` shows no actionable blocker |
| `candidate.capture` | Mint a new `SourceCandidate` for the job | Yes |
| `check.run` | Trigger a tool execution by `CheckId` | Yes — but only for the `check_id` returned by `job.next_actions` |
| `workspace.apply_patch` | Apply a candidate patch and bump `WorkspaceRevision` | Yes — but the revision must come from `workspace.stat` |
| `workspace.stat` | Read the workspace's current revision | Yes |
| `operation.get` | Read a `PrivilegedOperation` by id | Yes (read-only) |

## Forbidden actions

These actions would break the invariants the rest of IronMaint relies on.
If the agent's prompt ever asks the agent to do any of them, the right
response is **refuse and explain**.

| Action | Why it is forbidden |
|---|---|
| Open `ironmaint.sqlite` (or any future backing store) directly | The runtime is the only writer; the agent reads through `job.get`. Direct DB access bypasses the audit ledger and the engine's optimistic-concurrency guarantees (§17). |
| `cp`, `mv`, `rm`, or any other host-fs mutation outside the workspace manager | All host-fs writes are funnelled through `workspace.apply_patch`, which is gated by the artifact guard (§32) and bumps `WorkspaceRevision`. Anything else leaves the workspace in a state the runtime cannot reason about. |
| `bash`/`sh` invocations of `sbuild`, `mock`, `lintian`, `rpmlint`, `koji`, `bodhi`, `packit`, `git`, etc. | These tools have capability keys (`debian.build.sbuild`, `fedora.qa.rpmlint`, …) and may only run through `check.run` after the adapter's `BuildPlan`/`QaPlan` has materialised them as `CheckDefinition`s. The agent never names the binary itself. |
| Mutate the agent's host shell history, `~/.bashrc`, the IronClaw config dir, or the IronMaint config dir | Out of scope; IronMaint has no reason to touch these. |
| Mint `JobState` directly | Only `ironmaint_state::TransitionEngine` may transition state. The agent reads `next_actions` and chooses from the typed set the runtime enumerates. |
| Fabricate evidence | `Evidence` rows are minted by the runtime after a real tool run completes. The agent never writes evidence directly. |
| Mark an obligation satisfied without evidence | `MarkObligationSatisfied` is gated by the obligation's policy reference; the agent cannot bypass the gate by minting a fake `GateResult`. |
| Self-approve `ExceptionApproved` | An approval is a first-class record with an `ApprovalId`. The agent can *request* approval; only a human approver may *grant* it. |
| Skip `ReadyForApproval` | The agent must reach `ReadyForApproval` and stop. `Approved → PublicationPending → Published` requires a privileged operation authorised by the privileged service (§63); the agent does not have that authority. |

## Permission escalation rules

- The agent may call `job.reconcile` more than once in a row, but it must
  stop after a `Blocked`, `NeedsActorDecision`, or `Exceptional` outcome.
  Driving the job from those outcomes is a human activity.
- The agent may capture additional candidates after the initial one. Each
  new candidate mints a new `SourceCandidate`; the previous candidate is
  not deleted. Evidence does not transfer between candidates (§6).
- The agent may call `workspace.apply_patch` only with the current
  `WorkspaceRevision`. A mismatch is a `409 stale_revision` and the agent
  must call `workspace.stat` to refresh.

## Audit trail

Every MCP call is logged by the IronMaint server with a `ToolRunFinished`
audit envelope (`PHASE-0B.md §65`). The agent is not expected to write
audit records of its own — the runtime does so automatically.
