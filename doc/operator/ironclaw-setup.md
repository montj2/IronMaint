# IronClaw setup — pointing an IronClaw agent at an IronMaint MCP server

Phase 0B.9 / §95 of `doc/phases/PHASE-0B.md`.

IronMaint exposes its workflow driver as a Streamable-HTTP MCP server, served
by the `ironmaintd` daemon. IronClaw connects to it the same way it connects to
any other MCP extension: via a `reborn.extension_manifest.v3` that names the
server URL, declares the IronMaint tool set, and lists the credentials the
server expects. This document walks through that wiring end-to-end.

Every command below is runnable as written.

## 1. Run the IronMaint server

Build the workspace and start the daemon:

```sh
cargo build -p ironmaintd -p ironmaint-fixture
```

Create a token file. **The daemon reads the token; it does not mint one.**
There is no default token and no unauthenticated mode, so this step is not
optional:

```sh
mkdir -p ~/.config/ironmaint
umask 077
openssl rand -hex 32 > ~/.config/ironmaint/token
```

Start the server:

```sh
./target/debug/ironmaintd \
  --bind 127.0.0.1:7341 \
  --token-file ~/.config/ironmaint/token \
  --state-dir ~/.local/state/ironmaint
```

On a successful start the daemon prints exactly one line to **stdout**:

```text
ironmaintd listening on http://127.0.0.1:7341/mcp
```

Everything else — configuration, the resolved roots, the tool registry — goes to
**stderr** as `tracing` output, so that line is safe to parse. That is how
`scripts/ironclaw-e2e.sh` and the daemon's own tests find the port when it is
started with `--bind 127.0.0.1:0`.

### Options

Every option has an environment-variable equivalent. **A flag beats a variable
beats a default.**

| Flag | Variable | Default |
|---|---|---|
| `--state-dir PATH` | `IRONMAINT_STATE_DIR` | `./.ironmaint` |
| `--migrations-dir PATH` | `IRONMAINT_MIGRATIONS_DIR` | the workspace `migrations/` |
| `--bind HOST:PORT` | `IRONMAINT_BIND` | `127.0.0.1:7341` |
| `--token-file PATH` | `IRONMAINT_TOKEN_FILE` | **none — required** |
| `--workspace-root PATH` | `IRONMAINT_WORKSPACE_ROOT` | `<state-dir>/workspaces` |
| `--artifacts-root PATH` | `IRONMAINT_ARTIFACTS_ROOT` | `<state-dir>/artifacts` |
| `--fixture-bin PATH` | `IRONMAINT_FIXTURE_BIN` | `<dir of ironmaintd>/ironmaint-fixture` |
| `--log FILTER` | `IRONMAINT_LOG` | `info` |

`--migrations-dir` and `--token-file` are checked for existence at startup, and
the daemon exits `1` naming any that is missing. The migrations default resolves
relative to the *build* machine's workspace, so a packaged install must pass
`--migrations-dir` explicitly; failing at startup with a clear message is the
point of the check.

The daemon drains on `SIGINT` or `SIGTERM` and exits `0`.

### Liveness probe

The standard MCP `initialize` handshake. Note the `accept` header: the MCP
Streamable-HTTP transport requires the client to offer **both**
`application/json` and `text/event-stream`, and answers `406` if it does not.

```sh
curl -sS -X POST http://127.0.0.1:7341/mcp \
  -H "authorization: Bearer $(cat ~/.config/ironmaint/token)" \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
```

The response is a `result` object with `protocolVersion`, server capabilities
(`tools`), and the server info. **Without the token this is `401`**, with a
`WWW-Authenticate: Bearer` header. An `Authorization` header using any scheme
other than `Bearer` is `400`, not `401` — a malformed request is not a failed
authentication, and conflating the two would tell the client to retry a bad
header forever.

Authentication is checked before the transport validates anything else, so an
unauthenticated request that also violates `Accept`, `Host`, or `Content-Type`
is still a `401` — an anonymous caller cannot map the protocol surface for free.

## 2. Register the server with IronClaw

IronClaw has no separate MCP server registry. The IronMaint server reaches an
agent by being packaged as an extension whose manifest declares its tools and
endpoint. A complete manifest is shipped at
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
      kind: bearer
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
It encodes the workflow IronClaw should follow when driving a package through
the state machine, the invariants it must respect (immutable candidates,
candidate-scoped gates, no shell, no state fabrication), and the typed outcomes
it should expect from `reconcile`.

Activate it for the active workspace:

```sh
cp -r skills/ironmaint-maintainer ~/.ironclaw/installed_skills/
ironclaw skill refresh
```

The skill activates when the agent encounters IronMaint-specific triggers
("drive ironmaint job", "advance package through gates", "release-review
the candidate"). It is read-only and gains no additional tool authority
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

A scripted smoke test that verifies the wiring lives at
`scripts/ironclaw-e2e.sh`:

```sh
scripts/ironclaw-e2e.sh
```

It expects the IronMaint server on `127.0.0.1:7341` and a readable token file,
and it exercises the handshake, the tool list, and a `job.create` →
`job.get` round-trip. If an `ironclaw` binary is on `$PATH` it also checks the
extension is installed and advertises `job.create`.

### What the smoke test does *not* prove

It does not drive a job to `ReadyForApproval`. `candidate.capture` activates
the candidate and derives its gates and obligations (0B.10 C1), so a job no
longer sits at `EventDetected` forever — but the gates it derives name
`debian.*` / `fedora.*` tool keys, and the only tools registered in 0B are the
`synthetic.*` fixtures. `check.run` on a derived gate therefore reports the
tool as unknown. That is the honest state of 0B, and it is visible rather than
silent: `job.next_actions` names the pending gate and the tool key it is
waiting on.

| Tool | State in 0B.10 |
|---|---|
| `job.create`, `job.get`, `job.next_actions`, `job.reconcile` | live |
| `candidate.capture` | live — reads the real HEAD commit and tree OID, then activates the candidate and derives its gates and obligations from the registered adapter |
| `check.run` | live — runs the tool, records evidence, updates the gate; reports `unknown tool` for a `debian.*`/`fedora.*` key until Phase 1 registers real ones |
| `workspace.stat`, `workspace.apply_patch` | live — real revision, real `git apply` |
| `operation.get` | live — reads the operation store; always empty in 0B, since §4.10/§26/§99 forbid creating a privileged operation |
| `job.resume` | live, but a no-op in practice for 0B: nothing in 0B escalates a job to `HumanReviewRequired` on its own, so the runtime never offers `resume_job` and an agent has no reason to call it. It exists so the exit path is real when an operator escalates a job |

The daemon logs which adapter families it loaded at startup:

```text
adapter registry loaded families=["debian", "fedora"]
```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `401` on every call | Token file rotated, or IronClaw has a stale copy | Restart IronClaw, or `ironclaw extension reload ironmaint` |
| `400` on every call | `Authorization` is not using the `Bearer` scheme | Check the manifest's `auth.kind` |
| `406` on every call | The client is not sending `accept: application/json, text/event-stream` | Add both media types to the request |
| Daemon exits 1 naming a migrations directory | Packaged install is using the build machine's default | Pass `--migrations-dir` |
| `no such table: events` on the first call | The store was opened without migrations | Pass `--migrations-dir` pointing at a real `NNNN_*.sql` directory |
| `404 unknown tool` | IronMaint server older than the manifest | Upgrade IronMaint; the manifest and the server share the tool names from `crates/ironmaint-mcp/src/schema.rs` |
| `409 stale_projection` on `reconcile` | A parallel orchestrator advanced the job | Re-run `job.get`; the new version is the canonical one |
| Second daemon exits 1 immediately | §10 allows one daemon per state directory | Stop the first, or use a different `--state-dir` |
| Skill never activates | `max_context_tokens` too low | Bump in `~/.ironclaw/config.toml` |
