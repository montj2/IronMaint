#!/usr/bin/env bash
# smoke.sh — ironmaint/agent:0.1
# Runs INSIDE the container. Must exit non-zero on any failure.
# Spec §11: a test that cannot fail is not a test.
#
# Invocation patterns:
#   docker run --rm --platform linux/arm64 ironmaint/agent:0.1 \
#     /usr/local/bin/smoke.sh
#   make -f containers/Makefile smoke-agent
#
# This smoke verifies the contract from spec §4.5 / §9:
#   - pid 1 = tini (inherited from base; SPEC §4.5 last paragraph)
#   - uid 1000 ironmaint (spec §4.1 floor)
#   - ironclaw binary present, --version reports 1.4.1 exactly (pin guard)
#   - the public-side env vars are wired to the spec defaults
#   - the secret-side env vars are NOT baked (they come in via -e)
#   - /var/lib/ironclaw is writable
#   - TLS to api.minimax.io completes (CA roots carried through;
#     spec §11.2 — same backstop as base)
#
# Not in this smoke: a real `ironclaw serve` against postgres. That
# is a Phase 1 acceptance gate (spec §4.5) and needs a sidecar;
# the build smoke is intentionally just here.

set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
ok()   { printf 'OK:   %s\n' "$*"; }

# ---- 1. PID 1 is tini ---------------------------------------------------
# Same property as base. If tini isn't PID 1, `docker stop` becomes
# a kill -9 against ironclaw, killing it mid-shutdown.
[ "$(cat /proc/1/comm 2>/dev/null)" = "tini" ] \
    || fail "PID 1 is $(cat /proc/1/comm 2>/dev/null), expected tini"
ok "PID 1 is tini"

# ---- 2. uid 1000 ironmaint ---------------------------------------------
[ "$(id -u)" = "1000" ] || fail "running as uid $(id -u), expected 1000"
[ "$(id -un)" = "ironmaint" ] || fail "running as $(id -un), expected ironmaint"
ok "running as uid 1000 (ironmaint)"

# ---- 3. ironclaw binary + version pin ----------------------------------
# The release tarball is pinned by SHA256 in the Dockerfile; the
# binary is installed to /usr/local/bin/ironclaw. `ironclaw --version`
# is the simplest possible evidence the binary is real and not a stub.
if ! command -v ironclaw >/dev/null 2>&1; then
  fail "ironclaw is not on PATH (Dockerfile must install the release tarball)"
fi
set +e
set +o pipefail
out=$(ironclaw --version 2>&1 | head -1)
rc=$?
set -o pipefail
set -e
if [ "$rc" -ne 0 ]; then
  fail "ironclaw --version failed (exit $rc) — binary is broken or missing shared libraries"
fi
case "$out" in
  *"1.4.1"*) ok "ironclaw version is pinned to 1.4.1 ($out)" ;;
  *) fail "ironclaw version is '$out', expected 1.4.1 (Dockerfile pin should be re-checked)" ;;
esac

# ---- 4. non-secret env vars wired to spec defaults --------------------
# Spec §4.5 table. A developer can `docker run -e MINIMAX_API_KEY=…`
# without re-supplying the defaults below.
check_env() {
  local var="$1" expected="$2"
  actual=$(getenv "$var" || true)
  if [ "$actual" = "$expected" ]; then
    ok "$var=$actual (matches spec default)"
  else
    fail "$var is '$actual', expected '$expected' (spec §4.5 default drifted)"
  fi
}
getenv() { printf '%s' "${!1-}"; }
check_env LLM_BACKEND                 minimax
check_env MINIMAX_MODEL                MiniMax-M3.1-Flash-Preview
check_env MINIMAX_BASE_URL            https://api.minimax.io/v1
check_env IRONCLAW_REBORN_SERVE_HOST   0.0.0.0
check_env IRONCLAW_REBORN_HOME        /var/lib/ironclaw

# ---- 5. secrets NOT baked ---------------------------------------------
# Spec §4.5: "secret, injected at run time — never baked". If these
# are present in the environment, that's a supply-chain defect
# (anything that pulls this image inherits those creds).
if [ -n "${MINIMAX_API_KEY-}" ]; then
  fail "MINIMAX_API_KEY is set in the image — secret must not be baked (spec §4.5)"
fi
if [ -n "${IRONCLAW_REBORN_SECRET_MASTER_KEY-}" ]; then
  fail "IRONCLAW_REBORN_SECRET_MASTER_KEY is set in the image — secret must not be baked (spec §4.5)"
fi
ok "secret side: MINIMAX_API_KEY and IRONCLAW_REBORN_SECRET_MASTER_KEY are NOT baked"

# ---- 6. /var/lib/ironclaw writable -------------------------------------
# The volume must be writable by uid 1000 or the agent cannot store
# state. touch a sentinel file and confirm it lands.
sentinel=/var/lib/ironclaw/.smoke
if ! touch "$sentinel" 2>/dev/null; then
  fail "/var/lib/ironclaw is not writable by uid $(id -u)"
fi
rm -f "$sentinel"
ok "/var/lib/ironclaw is writable"

# ---- 7. TLS to api.minimax.io -----------------------------------------
# Spec §11.2: same backstop as base. If this fails, the agent
# cannot reach the model API and the symptom is "the daemon is up but
# nothing happens". The HTTP status code from the model API is not
# predictable (auth required → 401; rate limit → 429; etc.) so we
# accept any 2xx/3xx/4xx/5xx — the TLS handshake is what we are
# testing.
set +e
set +o pipefail
out=$(curl -sS --max-time 10 -o /dev/null -w "%{http_code}" https://api.minimax.io/v1/models 2>&1 | tr -d '\r')
curl_rc=$?
set -o pipefail
set -e
if [ "$curl_rc" -ne 0 ]; then
  fail "TLS to api.minimax.io failed: curl exited $curl_rc (likely missing ca-certificates — §11.2)"
fi
case "$out" in
  2*|3*|4*|5*) ok "TLS to api.minimax.io completed (HTTP $out) — CA certs present" ;;
  *) fail "TLS to api.minimax.io returned unexpected status: '$out'" ;;
esac

echo "ironmaint/agent:0.1 — SMOKE OK"