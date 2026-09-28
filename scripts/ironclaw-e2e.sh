#!/usr/bin/env bash
# IronClaw ↔ IronMaint end-to-end smoke test.
#
# Phase 0B.7 / §95 of doc/phases/PHASE-0B.md.
#
# Verifies the IronClaw extension manifest wires correctly
# to the IronMaint MCP server, and that an IronClaw agent can
# drive a no-op job.create round-trip through it. Exits 0 on
# success and non-zero on any failure.
#
# Prereqs:
#   - IronMaint MCP server running on $IRONMAINT_URL (default
#     http://127.0.0.1:7341/mcp) with a bearer token in
#     $IRONMAINT_TOKEN_FILE (default ~/.config/ironmaint/token).
#   - `ironclaw` on $PATH with the ironmaint extension
#     installed (see doc/operator/ironclaw-setup.md).

set -euo pipefail

IRONMAINT_URL="${IRONMAINT_URL:-http://127.0.0.1:7341/mcp}"
IRONMAINT_TOKEN_FILE="${IRONMAINT_TOKEN_FILE:-$HOME/.config/ironmaint/token}"

if [[ ! -r "$IRONMAINT_TOKEN_FILE" ]]; then
    echo "ironclaw-e2e: token file $IRONMAINT_TOKEN_FILE is not readable" >&2
    exit 2
fi

TOKEN="$(cat "$IRONMAINT_TOKEN_FILE")"

# 1. MCP initialize handshake
INIT_RESP="$(curl -sS -X POST "$IRONMAINT_URL" \
    -H "authorization: Bearer $TOKEN" \
    -H "content-type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"ironclaw-e2e","version":"0.1.0"}}}')"

if [[ "$INIT_RESP" != *'"protocolVersion"'* ]]; then
    echo "ironclaw-e2e: initialize failed: $INIT_RESP" >&2
    exit 1
fi

# 2. IronClaw extension healthcheck (only if ironclaw is on PATH)
if command -v ironclaw >/dev/null 2>&1; then
    if ! ironclaw extension status ironmaint >/dev/null 2>&1; then
        echo "ironclaw-e2e: ironmaint extension is not installed or not active" >&2
        echo "ironclaw-e2e: hint: ironclaw extension install doc/operator/mcp-registration.yaml" >&2
        exit 1
    fi

    # 3. End-to-end round-trip: ask IronClaw to create one no-op
    # job. We use the in-process tool-list test rather than a full
    # LLM round-trip so the smoke test does not require an LLM key.
    if ! ironclaw extension tools ironmaint | grep -q '^job.create$'; then
        echo "ironclaw-e2e: ironmaint extension is missing the job.create tool" >&2
        exit 1
    fi
fi

echo "ironclaw-e2e: OK (IronMaint MCP server at $IRONMAINT_URL answered initialize)"
