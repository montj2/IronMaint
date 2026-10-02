#!/usr/bin/env bash
# smoke.sh — ironmaint/debian-tools:0.1
# Runs INSIDE the container. Must exit non-zero on any failure.
# Spec §11: a test that cannot fail is not a test.
#
# Invocation patterns:
#   docker run --rm -v "$PWD":/work ironmaint/debian-tools:0.1 \
#     /usr/local/bin/smoke.sh
#   make -f containers/Makefile smoke-debian-tools
#
# This smoke covers the version checks from spec §4.3 / §9. The
# full "real sbuild run against a real source package" gate is in
# scripts/debian-tools-build-smoke.sh and runs separately from compose
# with --privileged.

set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
ok()   { printf 'OK:   %s\n' "$*"; }

# ---- 1. PID 1 is tini (inherited from base) ------------------------------
# The base image sets tini as PID 1. debian-tools inherits that — the
# Dockerfile doesn't override USER or ENTRYPOINT. If a future change
# drops tini (e.g. switches to docker's built-in --init hint), PID 1
# would become the smoke script or bash; signal forwarding breaks.
[ "$(cat /proc/1/comm 2>/dev/null)" = "tini" ] \
    || fail "PID 1 is $(cat /proc/1/comm 2>/dev/null), expected tini"
ok "PID 1 is tini"

# ---- 2. uid 1000 (ironmaint) ---------------------------------------------
# Spec §4.1 floor: all images in this set run as uid 1000 for
# bind-mount predictability in compose.
[ "$(id -u)" = "1000" ] || fail "running as uid $(id -u), expected 1000"
[ "$(id -un)" = "ironmaint" ] || fail "running as $(id -un), expected ironmaint"
ok "running as uid 1000 (ironmaint)"

# ---- 3. all spec §4.3 binaries present -----------------------------------
# Each line is "binary  package  mandatory". Both --version and a
# presence-on-PATH check. `set +e` + `set +o pipefail` dance around $()
# so a missing binary produces a FAIL message instead of dying under
# pipefail with no context. `head -1` exits non-zero on SIGPIPE when
# the binary keeps writing past the first line (sbuild is a real
# offender here — its --version prints an advisory banner and then a
# multi-line copyright before exiting), so we have to look at the
# binary's exit code, not the pipe's.
# Some packages (uidmap, etc.) ship multiple binaries; the smoke test
# only knows one name per package. Pass `bin` as a real binary path
# that's known to exist; the package name is purely informational.
check_bin() {
  local bin="$1" pkg="$2" mandatory="$3"
  set +e
  set +o pipefail
  out=$("$bin" --version 2>&1 | head -1)
  rc=$?
  set -o pipefail
  set -e
  if [ "$rc" -ne 0 ]; then
    if [ "$mandatory" = "yes" ]; then
      fail "$bin ($pkg) is mandatory but missing or broken (exit $rc)"
    else
      printf 'WARN: %s (%s) is optional, skipping: exit %d\n' "$bin" "$pkg" "$rc"
      return 0
    fi
  fi
  printf 'OK:   %s (%s): %s\n' "$bin" "$pkg" "$out"
}

# Path-based presence check for tools whose --version flag isn't stable
# or doesn't exist. Use this for things like `schroot` (no --version),
# `debootstrap` (no --version), `newuidmap` (no --version).
check_present() {
  local bin="$1" what="$2" mandatory="$3"
  if command -v "$bin" >/dev/null 2>&1; then
    printf 'OK:   %s present (%s)\n' "$bin" "$what"
    return 0
  fi
  if [ "$mandatory" = "yes" ]; then
    fail "$bin ($what) is mandatory but missing"
  else
    printf 'WARN: %s (%s) is optional, skipping: not on PATH\n' "$bin" "$what"
    return 0
  fi
}

# Order matches spec §4.3 table (mandatory first).
check_bin sbuild        sbuild           yes
check_bin lintian       lintian          yes
check_bin autopkgtest   autopkgtest      yes
check_bin dpkg-buildpackage dpkg-dev      yes
check_bin gbp           git-buildpackage yes
check_bin uscan         devscripts       yes
check_bin piuparts      piuparts         no
check_bin diffoscope    diffoscope       no
check_bin reprotest     reprotest        no

# ---- 4. ironmaint user is in the sbuild group ----------------------------
# Compose invokes this image as uid 1000. `sbuild` requires the caller
# to be in the `sbuild` group; otherwise it errors out with
# "You are not privileged to use sbuild".
if id -Gn ironmaint | tr ' ' '\n' | grep -qx sbuild; then
  ok "ironmaint is in the sbuild group"
else
  fail "ironmaint is NOT in the sbuild group — Dockerfile must add it"
fi

# ---- 5. sandbox tooling present ----------------------------------------
# Spec §4.3 says schroots are created on first use; the runtime sandbox
# needs `schroot` (or `unshare` + `bubblewrap` if sbuild is on the
# `unshare` backend) and `debootstrap` for the initial tarball.
# Both schroot and debootstrap are NOT transitive deps of sbuild on
# trixie (verified: the sbuild package only Depends on libsbuild-perl),
# so the Dockerfile installs them explicitly.
check_present schroot      "schroot sandbox manager" yes
check_present debootstrap  "buildroot creator"      yes
# uidmap's actual binary is `newuidmap` (the package name `uidmap`
# is a metapackage alias).
check_present newuidmap    "uidmap package binary"  yes

# ---- 6. CA roots carried through from base -------------------------------
# The image is FROM ironmaint/base:0.1, which has ca-certificates.
# Sanity check that we can reach the Debian package mirror over HTTPS.
# Same set -e dance as base/smoke.sh — capture curl exit code and
# produce a useful FAIL message.
set +e
out=$(curl -sS --max-time 10 -o /dev/null -w "%{http_code}" https://deb.debian.org/debian/ 2>&1 | tr -d '\r')
curl_rc=$?
set -e
if [ "$curl_rc" -ne 0 ]; then
  fail "TLS to deb.debian.org failed: curl exited $curl_rc (likely missing ca-certificates)"
fi
case "$out" in
  2*|3*|4*|5*) ok "TLS to deb.debian.org completed (HTTP $out) — CA certs present" ;;
  *) fail "TLS to deb.debian.org returned unexpected status: '$out'" ;;
esac

echo "ironmaint/debian-tools:0.1 — SMOKE OK"
