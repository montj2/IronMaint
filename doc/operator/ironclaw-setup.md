# IronClaw setup — pointing an IronClaw agent at an IronMaint MCP server

Phase 0B.7 / §95 of `doc/phases/PHASE-0B.md`.

IronMaint exposes its workflow driver as a Streamable-HTTP MCP server
(`ironmaint-mcp`). IronClaw connects to it the same way it connects to any
other MCP extension: via a `reborn.extension_manifest.v3` that names the
server URL, declares the IronMaint tool set, and lists the credentials the
server expects. This document walks through that wiring end-to-end.

## 1. Run the IronMaint server

> **⚠ Status: the `serve` binary does not exist yet.** The sections below
> describe the *intended* operator workflow, written so the design is
> reviewable. Nothing in this document is runnable today — see
> `doc/PHASE-0B-COMPLETION.md` §16.2. Until the binary lands, the MCP
> dispatcher is exercised in-process by `ironmaint-mcp/tests/dispatcher.rs`.
> Treat steps 2–5 as a specification for the work, not as instructions.

The server is intended to speak Streamable HTTP on `127.0.0.1` and to
authenticate every request with a bearer token from a config file. The
intended invocation is:

```sh
cargo run -p ironmaint-mcp --release -- serve \
  --bind 127.0.0.1:7341 \
  --token-file ~/.config/ironmaint/token
```

The `--token-file` flag is intended to write a freshly-minted bearer token
to the file and print it to stderr. The server keeps no copy of the token —
losing the file means re-running with `--rotate-token`.

On a successful start you should see a log line similar to:

```text
ironmaint-mcp listening on http://127.0.0.1:7341/mcp
```

The intended liveness probe is the standard MCP `initialize` handshake:

```sh
curl -sS -X POST http://127.0.0.1:7341/mcp \
  -H "authorization: Bearer $(cat ~/.config/ironmaint/token)" \
  -H "content-type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
```

The response is a `result` object with `protocolVersion`, server
capabilities, and the IronMaint server info.

## 2. Register the server with IronClaw

IronClaw has no separate MCP server registry. The IronMaint server reaches
an agent by being packaged as an extension whose manifest declares its
tools and endpoint. A complete manifest is shipped at
`doc/operator/mcp-registration.yaml`; the relevant fields are:

```yaml
api_version: reborn.extension_manifest.v3
metadata:
  name: ironmaint
  vendor: ironmaint
extensions:
  - name: ironmaint
    runtime: mcp
    endpoint: http://127.0.0.1:7341/mcp
    auth:
      bearer_token_file: ~/.config/ironmaint/token
    capabilities:
      tools:
        - job.create
        - job.get
        - job.next_actions
        - job.reconcile
        - candidate.capture
        - check.run
        - workspace.apply_patch
        - workspace.stat
        - operation.get
```

Install it:

```sh
ironclaw extension install doc/operator/mcp-registration.yaml
ironclaw extension activate ironmaint
```

Once activated, every IronClaw agent — TUI, WebUI, headless — has the
IronMaint tool set available in its capability budget.

## 3. Load the IronMaint maintainer skill

IronMaint ships a runtime skill at `skills/ironmaint-maintainer/SKILL.md`.
It encodes the workflow IronClaw should follow when driving a package from
`EventDetected` to `ReadyForApproval`, the invariants it must respect
(immutable candidates, candidate-scoped gates, no shell, no state
fabrication), and the typed outcomes it should expect from `reconcile`.

Activate it for the active workspace:

```sh
cp -r skills/ironmaint-maintainer ~/.ironclaw/installed_skills/
ironclaw skill refresh
```

The skill activates when the agent encounters IronMaint-specific triggers
("drive ironmaint job", "advance package through gates", "release-review
the candidate"). It is read-only and gain no additional tool authority
beyond what the agent already has — the rules in §4 govern that.

## 4. Tool permissions

The skill authorises the agent to call any tool on the IronMaint MCP server
*provided the call flows through the MCP transport*. The agent is **not**
authorised to:

- open the IronMaint SQLite database directly (the runtime is the only
  writer; the agent reads through `job.get`);
- write to any workspace on the host filesystem (the workspace manager
  brokers `workspace.apply_patch` and is the only path that bumps
  `WorkspaceRevision`);
- execute arbitrary host binaries (the executor owns the tool registry and
  the artifact guard; agent-issued `check.run` calls go through the
  executor's per-job capability key);
- invent `JobState` transitions (`reconcile` and the engine are the only
  writers; the agent reads `next_actions` and chooses the next call from
  what the runtime enumerates).

See `doc/operator/tool-permissions.md` for the full table.

## 5. End-to-end check

A scripted smoke test that verifies the wiring without driving a full job
lives at `scripts/ironclaw-e2e.sh`:

```sh
scripts/ironclaw-e2e.sh
```

It expects the IronMaint server on `127.0.0.1:7341` and an IronClaw
binary on `$PATH`. It exits non-zero if either MCP `initialize` or the
`job.create` round-trip fails.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `403 invalid_token` on every call | Token file rotated under IronClaw | `ironclaw extension reload ironmaint` |
| `404 unknown tool` | IronMaint server older than the manifest | Upgrade IronMaint; the manifest and the server share the tool names from `crates/ironmaint-mcp/src/schema.rs` |
| `409 stale_projection` on `reconcile` | Parallel orchestrator advanced the job | Re-run `job.get`; the new version is the canonical one |
| Skill never activates | `max_context_tokens` too low | Bump in `.claude/config.toml` or `~/.ironclaw/config.toml` |
