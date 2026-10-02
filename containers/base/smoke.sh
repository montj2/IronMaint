#!/usr/bin/env bash
# smoke.sh — ironmaint/base:0.1
# Runs INSIDE the container. Must exit non-zero on any failure.
# Spec §11: a test that cannot fail is not a test.

set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
ok()   { printf 'OK:   %s\n' "$*"; }

# ---- 1. tini is PID 1 and works ------------------------------------------
# Spec §5.3 — every long-running container in this set uses tini as PID 1.
# If tini isn't PID 1, signal forwarding is silently broken and the
# shutdown story for ironmaintd, ironclaw, postgres all degrade.
pid1_comm=$(cat /proc/1/comm 2>/dev/null || true)
[ "$pid1_comm" = "tini" ] || fail "PID 1 is '$pid1_comm', expected 'tini'"
ok "PID 1 is tini"

tini --version | grep -q "tini" || fail "tini --version did not match"
ok "tini --version runs"

# ---- 2. non-root user ----------------------------------------------------
# The image was built USER ironmaint, so by the time smoke runs we ARE
# that user. Verifying uid and username catches a Dockerfile that lost
# the USER directive.
[ "$(id -u)" = "1000" ] || fail "uid is $(id -u), expected 1000"
[ "$(id -un)" = "ironmaint" ] || fail "username is $(id -un), expected ironmaint"
ok "running as uid 1000 (ironmaint)"

# ---- 3. required tools on PATH and reporting versions ---------------------
git --version    | grep -q "git version" || fail "git --version broken"
python3 --version | grep -q "Python 3"   || fail "python3 missing or not 3.x"
curl --version   | head -n1              || fail "curl --version empty"
ok "git, python3, curl all on PATH and reporting versions"

# ---- 4. CA roots + native-roots TLS --------------------------------------
# Spec §11.2 — this is the failure mode most likely to be misdiagnosed as
# "the network is broken". sqlx/reqwest use rustls' native-roots backend
# (verified: Cargo.toml). That backend reads the OS trust store. Without
# ca-certificates, the connect fails with a TLS error that looks like a
# network fault. This is the single most likely thing to be missed.
out=$(curl -sSI --max-time 10 https://github.com 2>&1 | head -n1 | tr -d '\r')
case "$out" in
  HTTP/1.1\ 200*|HTTP/2\ 200*|HTTP/3\ 200*) ok "TLS to github.com returns 200 ($out)" ;;
  *) fail "TLS check failed: expected HTTP 200, got '$out' — ca-certificates likely missing" ;;
esac

# ---- 5. tini forwards signals (the most subtle property) ----------------
# Spawn a child that exits 42 on SIGTERM. Kill its parent (which is tini
# here). If tini forwards SIGTERM, we observe exit 42. If tini swallowed
# the signal, the child would die on a different signal or hang.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
set +e
tini -- bash -c 'trap "exit 42" TERM; while :; do sleep 1; done' >"$work/out" 2>&1 &
child_pid=$!
sleep 1
kill -TERM "$child_pid" 2>/dev/null
wait "$child_pid"
rc=$?
set -e
[ "$rc" = "42" ] || fail "tini did not forward SIGTERM (got exit $rc)"
ok "tini forwards SIGTERM (child exited 42)"

echo "ironmaint/base:0.1 — SMOKE OK"