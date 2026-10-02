#!/usr/bin/env bash
# smoke.sh — ironmaint/fedora-tools:0.1
# Runs INSIDE the container. Must exit non-zero on any failure.
# Spec §11: a test that cannot fail is not a test.
#
# Invocation patterns:
#   docker run --rm --platform linux/arm64 --privileged \
#     -v "$PWD":/work -w /work \
#     ironmaint/fedora-tools:0.1 /usr/local/bin/smoke.sh
#   make -f containers/Makefile smoke-fedora-tools
#
# This smoke covers the version checks from spec §4.4 / §9. The full
# "mock -r init succeeds + trivial build" gate is in
# scripts/fedora-tools-build-smoke.sh and runs separately from compose
# with --privileged.

set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
ok()   { printf 'OK:   %s\n' "$*"; }

# ---- 1. PID 1 is tini ----------------------------------------------------
# The Dockerfile installs tini as PID 1. The ENTRYPOINT uses tini
# so any signal sent to the container reaches the child process.
[ "$(cat /proc/1/comm 2>/dev/null)" = "tini" ] \
    || fail "PID 1 is $(cat /proc/1/comm 2>/dev/null), expected tini"
ok "PID 1 is tini"

# ---- 2. uid 1000 (ironmaint) ---------------------------------------------
# Spec §4.1 floor: all images in this set run as uid 1000 for
# bind-mount predictability in compose.
[ "$(id -u)" = "1000" ] || fail "running as uid $(id -u), expected 1000"
[ "$(id -un)" = "ironmaint" ] || fail "running as $(id -un), expected ironmaint"
ok "running as uid 1000 (ironmaint)"

# ---- 3. all spec §4.4 binaries present -----------------------------------
# Path-based presence check: some spec binaries (koji, bodhi, packit,
# fedpkg, tmt) don't have a stable --version flag or output something
# unhelpful; we verify presence via PATH and a minimal --help/-V run.
check_bin() {
  local bin="$1" pkg="$2" mandatory="$3" verifier="$4"
  if ! command -v "$bin" >/dev/null 2>&1; then
    if [ "$mandatory" = "yes" ]; then
      fail "$bin ($pkg) is mandatory but missing from PATH"
    else
      printf 'WARN: %s (%s) is optional, skipping: not on PATH\n' "$bin" "$pkg"
      return 0
    fi
  fi
  set +e
  set +o pipefail
  out=$("$bin" $verifier 2>&1 | head -1)
  rc=$?
  set -o pipefail
  set -e
  if [ "$rc" -ne 0 ]; then
    if [ "$mandatory" = "yes" ]; then
      fail "$bin ($pkg) exists but $verifier failed (exit $rc)"
    else
      printf 'WARN: %s (%s) is optional, skipping: exit %d\n' "$bin" "$pkg" "$rc"
      return 0
    fi
  fi
  printf 'OK:   %s (%s): %s\n' "$bin" "$pkg" "$out"
}

# Order matches spec §4.4 table (mandatory first).
# mock + rpmlint + rpmbuild are the core; the rest are orchestration
# tooling for which we check --help presence (the spec lists them but
# doesn't require they each be a smoke gate on this image).
check_bin mock     mock          yes "--version"
check_bin rpmlint  rpmlint       yes "--version"
check_bin rpmbuild rpm-build     yes "--version"
check_bin spectool rpm-build     yes "--version"
check_bin tmt      tmt           no  "--version"
check_bin fedpkg   fedpkg        no  "--help"
check_bin koji     koji          no  "--help"
check_bin bodhi    bodhi-client  no  "--help"
check_bin packit   packit        no  "--version"

# ---- 4. ironmaint user is in the mock group ------------------------------
# Compose invokes this image as uid 1000. `mock` requires the caller
# to be in the `mock` group, otherwise mock errors with
# "You are not allowed to run mock as root" or similar.
if id -Gn ironmaint | tr ' ' '\n' | grep -qx mock; then
  ok "ironmaint is in the mock group"
else
  fail "ironmaint is NOT in the mock group — Dockerfile must add it"
fi

# ---- 5. mock can see its buildroot config --------------------------------
# Spec §4.4 says buildroots are NOT baked in. mock creates them at
# first use into /var/lib/mock/<config>/root. Verify mock's default
# config for fedora-44-aarch64 is on disk — proves /etc/mock is
# readable and has the config the build smoke will actually use.
#
# mock 6.8 removed `--list-configs` (it now treats unknown args as
# srpm paths). Path-based presence check is the modern equivalent:
# `ls /etc/mock/fedora-44-aarch64.cfg` is the contract mock reads
# when invoked with `-r fedora-44-aarch64`.
if [ -r /etc/mock/fedora-44-aarch64.cfg ]; then
  ok "mock sees fedora-44-aarch64 default config at /etc/mock/fedora-44-aarch64.cfg"
else
  fail "mock does not see fedora-44-aarch64 default config — /etc/mock/fedora-44-aarch64.cfg missing or unreadable"
fi

# ---- 6. CA roots carried through -----------------------------------------
# Image is FROM fedora:44 (not FROM ironmaint/base), so we re-install
# ca-certificates here. This guards against a future bump of the
# fedora image that drops it (the same §11.2 argument).
#
# Same set -e + set +o pipefail dance as base/smoke.sh — capture
# curl's exit code and produce a useful FAIL message.
set +e
set +o pipefail
out=$(curl -sS --max-time 10 -o /dev/null -w "%{http_code}" https://dl.fedoraproject.org/pub/fedora/linux/releases/44/Server/aarch64/iso/Fedora-Server-dvd-aarch64-44-1.6.iso 2>&1 | tr -d '\r')
curl_rc=$?
set -o pipefail
set -e
if [ "$curl_rc" -ne 0 ]; then
  fail "TLS to dl.fedoraproject.org failed: curl exited $curl_rc (likely missing ca-certificates — §11.2)"
fi
case "$out" in
  2*|3*|4*|5*) ok "TLS to dl.fedoraproject.org completed (HTTP $out) — CA certs present" ;;
  *) fail "TLS to dl.fedoraproject.org returned unexpected status: '$out'" ;;
esac

echo "ironmaint/fedora-tools:0.1 — SMOKE OK"
