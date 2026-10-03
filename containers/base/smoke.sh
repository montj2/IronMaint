#!/usr/bin/env bash
# smoke.sh — ironmaint/base:0.1
# Runs INSIDE the container. Must exit non-zero on any failure.
# Spec §11: a test that cannot fail is not a test.

set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
ok()   { printf 'OK:   %s\n' "$*"; }

# ---- architecture: the image is what it claims to be -----------------------
# Register entry D-23. `docker inspect` reports the platform an image was
# REQUESTED for, not the architecture of the binaries inside it, so an image can
# be labelled arm64, contain only x86_64, and still inspect as correct.
#
# Two things made that invisible when it happened here. A base pinned by digest
# where the digest names a single amd64 manifest rather than a multi-arch index
# resolves amd64 even under `--platform linux/arm64`, and BuildKit warns once and
# carries on. And Docker Desktop emulates x86_64 transparently, so every version
# check in these scripts answers correctly from a wrong binary.
#
# So the assertion compares the arch we ASKED for against the arch we GOT.
# Anything else is circular: a README, an image label, or `dpkg --print-architecture`
# on an emulated image all report what was requested, not what runs.
#
# IRONMAINT_EXPECT_ARCH is set by `make -C containers smoke-*` and by CI. A bare
# `docker run` cannot answer the question, so it says so rather than passing a
# check it did not make.
got_arch=$(uname -m)
if [ -z "${IRONMAINT_EXPECT_ARCH:-}" ]; then
  printf 'SKIP: architecture not checked - IRONMAINT_EXPECT_ARCH is unset.\n'
  printf '      actual: %s. Run `make -C containers smoke`, or pass\n' "$got_arch"
  printf '      -e IRONMAINT_EXPECT_ARCH=arm64 to assert it.\n'
else
  # Normalise Go/docker arch names (arm64) to uname's (aarch64).
  case "$got_arch" in
    aarch64) got_norm=arm64 ;;
    x86_64)  got_norm=amd64 ;;
    *)       got_norm=$got_arch ;;
  esac
  [ "$got_norm" = "$IRONMAINT_EXPECT_ARCH" ] || \
    fail "image is $got_arch ($got_norm) but was built for $IRONMAINT_EXPECT_ARCH - D-23. A base pinned to a single-arch manifest is the usual cause; pin the multi-arch index digest."
  ok "image architecture is $got_arch, as requested"
fi


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
#
# Temporarily disable set -e AND pipefail so we can capture curl's
# exit code AND have the FAIL message printed with a useful diagnostic.
# `head -n1` exits non-zero on SIGPIPE when curl keeps writing past the
# first line (GitHub's response is multi-line for HTTP/2 — the head
# closes the pipe after the first line, curl's next write gets EPIPE,
# and curl exits 23 "Failed writing body"). With pipefail on, that
# propagates through the command substitution and we report curl's
# exit code as the failure cause.
set +e
set +o pipefail
out=$(curl -sSI --max-time 10 https://github.com 2>&1 | head -n1 | tr -d '\r')
curl_rc=$?
set -o pipefail
set -e
if [ "$curl_rc" -ne 0 ]; then
  fail "TLS to github.com failed: curl exited $curl_rc (likely missing ca-certificates — §11.2)"
fi
case "$out" in
  HTTP/1.1\ 200*|HTTP/2\ 200*|HTTP/3\ 200*) ok "TLS to github.com returns 200 ($out)" ;;
  *) fail "TLS check failed: expected HTTP 200, got '$out'" ;;
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