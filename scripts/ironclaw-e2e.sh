#!/usr/bin/env bash
# IronClaw ↔ IronMaint end-to-end smoke test.
#
# Phase 0B.9 / §95 of doc/phases/PHASE-0B.md.
#
# Verifies that an MCP client can reach the IronMaint server, authenticate,
# list the tool surface, and complete a job.create -> job.get round-trip.
# Exits 0 on success, 1 on a check failure, 2 on a missing prerequisite.
#
# Prereqs:
#   - `ironmaintd` running (see doc/operator/ironclaw-setup.md §1), or
#     IRONMAINT_URL pointing at one.
#   - a readable bearer token in $IRONMAINT_TOKEN_FILE.
#   - optionally, an `ironclaw` binary on $PATH with the ironmaint
#     extension installed (doc/operator/mcp-registration.yaml).

set -euo pipefail

IRONMAINT_URL="${IRONMAINT_URL:-http://127.0.0.1:7341/mcp}"
IRONMAINT_TOKEN_FILE="${IRONMAINT_TOKEN_FILE:-$HOME/.config/ironmaint/token}"

die() { echo "ironclaw-e2e: $*" >&2; exit "${2:-1}"; }

if [[ ! -r "$IRONMAINT_TOKEN_FILE" ]]; then
    die "token file $IRONMAINT_TOKEN_FILE is not readable" 2
fi

TOKEN="$(cat "$IRONMAINT_TOKEN_FILE")"

# The MCP Streamable-HTTP transport requires the client to offer BOTH
# media types; a request offering only application/json is rejected 406
# before it is ever dispatched. Omitting this header makes every
# assertion below fail with a status code that says nothing about the
# real problem.
ACCEPT="application/json, text/event-stream"

# Post a JSON-RPC body. Echoes "<status>\n<body>".
rpc() {
    curl -sS -X POST "$IRONMAINT_URL" \
        -H "authorization: Bearer $TOKEN" \
        -H "content-type: application/json" \
        -H "accept: $ACCEPT" \
        -d "$1" \
        -w '\n%{http_code}'
}

# Extract a field from a JSON document on stdin. Avoids a jq dependency:
# the smoke test should run on a machine with nothing but curl.
# A parse failure prints nothing rather than raising: the caller has
# the raw response body and its own `die` message, and a Python
# traceback on stderr tells an operator nothing they can act on.
json() {
    python3 -c '
import json, sys
try:
    doc = json.load(sys.stdin)
except ValueError:
    sys.exit(0)
for key in sys.argv[1].split("."):
    if doc is None:
        break
    doc = doc.get(key) if isinstance(doc, dict) else None
print("" if doc is None else (json.dumps(doc) if isinstance(doc, (dict, list)) else doc))
' "$1"
}

# 1. The token is actually enforced: an unauthenticated call must be 401.
#
# Checking this first matters. If the server were not authenticating,
# every later step would still pass, and the smoke test would report a
# working server for one that is wide open.
ANON_STATUS="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$IRONMAINT_URL" \
    -H "content-type: application/json" \
    -H "accept: $ACCEPT" \
    -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}')"
if [[ "$ANON_STATUS" != "401" ]]; then
    die "an unauthenticated tools/list returned $ANON_STATUS, expected 401" 1
fi

# 2. MCP initialize handshake.
INIT="$(rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ironclaw-e2e","version":"0.1.0"}}}')"
INIT_BODY="${INIT%$'\n'*}"
INIT_STATUS="${INIT##*$'\n'}"
[[ "$INIT_STATUS" == "200" ]] || die "initialize returned HTTP $INIT_STATUS: $INIT_BODY" 1
[[ "$INIT_BODY" == *'"protocolVersion"'* ]] || die "initialize returned no protocolVersion: $INIT_BODY" 1

# 3. The eleven-tool surface (§94).
TOOLS="$(rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}')"
TOOLS_BODY="${TOOLS%$'\n'*}"
TOOLS_STATUS="${TOOLS##*$'\n'}"
[[ "$TOOLS_STATUS" == "200" ]] || die "tools/list returned HTTP $TOOLS_STATUS: $TOOLS_BODY" 1

for tool in job.create job.get job.next_actions job.reconcile job.resume \
            candidate.capture check.run workspace.apply_patch \
            workspace.stat operation.get release.candidate.create; do
    [[ "$TOOLS_BODY" == *"\"$tool\""* ]] \
        || die "tools/list does not advertise $tool: $TOOLS_BODY" 1
done
# §94 enumerates tool capabilities, not a count, so the tenth tool
# (`job.resume`, 0B.10 C2) and the eleventh (`release.candidate.create`,
# 0B.10 C5) are spec-compatible. The count is still pinned so a tool
# disappearing is caught here rather than at first use.
TOOL_COUNT="$(printf '%s' "$TOOLS_BODY" | json result.tools | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
[[ "$TOOL_COUNT" == "11" ]] || die "expected 11 tools, got $TOOL_COUNT" 1

# 4. job.create -> job.get round-trip, over the wire.
CREATED="$(rpc '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"job.create","arguments":{"orchestrator":{"kind":"ironclaw"},"package":{"distribution":{"family":"debian","release":"sid"},"source_name":"ironclaw-e2e-probe","binary_names":[]}}}}' | sed '$d')"
JOB_ID="$(printf '%s' "$CREATED" | json result.structuredContent.job_id)"
[[ -n "$JOB_ID" ]] || die "job.create returned no job_id: $CREATED" 1

GOT="$(rpc "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"job.get\",\"arguments\":{\"job_id\":\"$JOB_ID\"}}}" | sed '$d')"
GOT_ID="$(printf '%s' "$GOT" | json result.structuredContent.projection.job.id)"
[[ "$GOT_ID" == "$JOB_ID" ]] || die "job.get returned $GOT_ID, expected $JOB_ID: $GOT" 1

# 5. A failing call must come back readable, not as a protocol error. An
#    agent that cannot read why a call failed cannot decide what to do.
BAD="$(rpc '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"job.get","arguments":{"job_id":"00000000-0000-7000-8000-000000000000"}}}' | sed '$d')"
[[ "$BAD" == *'"error"'* ]] || die "an unknown job id did not produce a tool error: $BAD" 1
[[ "$BAD" != *'"code"'* ]] || die "an unknown job id produced a JSON-RPC error, not a tool error: $BAD" 1

# 6. IronClaw-side wiring, when an ironclaw binary is available.
if command -v ironclaw >/dev/null 2>&1; then
    if ! ironclaw extension status ironmaint >/dev/null 2>&1; then
        die "ironmaint extension is not installed or not active" 1
        echo "ironclaw-e2e: hint: ironclaw extension install doc/operator/mcp-registration.yaml" >&2
    fi
    if ! ironclaw extension tools ironmaint | grep -q 'job\.create'; then
        die "ironmaint extension is missing the job.create tool" 1
    fi
    echo "ironclaw-e2e: ironclaw extension ironmaint is installed and active"
else
    echo "ironclaw-e2e: no ironclaw binary on \$PATH; skipping the extension checks" >&2
fi

echo "ironclaw-e2e: OK"
echo "  server:  $IRONMAINT_URL (auth enforced: 401 without a token)"
echo "  surface: $TOOL_COUNT tools advertised"
echo "  job:     $JOB_ID created and read back"
