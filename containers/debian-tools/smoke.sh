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

# ---- 7. BTS read-only transport helper (Phase 1 1E.1) -------------------
# Spec §23 + §27: `python3-debianbts` is the BTS read-only transport
# (the SOAP client), wrapped in a fixed-binary shim at /usr/local/bin/bts
# so the executor has a stable path. The shim's `--self-test` mode
# emits a hard-coded canonical-JSON fixture (no network), which the
# smoke test exercises here.
#
# This assertion is the broken-then-fixed sentinel for 1E.1. The RED
# commit's `bts` shim is a stub that exits 1 with "not yet implemented"
# (no JSON on stdout). The GREEN commit replaces the stub body with
# a real `python3-debianbts` shim that emits canonical JSON.
#
# The check is the same shape as the §4.3 / §4.4 binary checks above:
# a real exit code, a JSON parse, and a useful FAIL message. We use
# `python3` for the parse (not `jq`) so the test doesn't introduce a
# new dependency — `python3` is in the base image (per §4.2).
if ! command -v bts >/dev/null 2>&1; then
  fail "bts: not on PATH — Dockerfile must COPY bts-helper.py to /usr/local/bin/bts"
fi
# The self-test mode is hermetic (no network). If the live mode is
# what we want to smoke, that comes in 1E.2 with a fake-BTS hermetic
# test; the §97 gate never touches the network.
set +e
bts_out=$(bts --self-test example 2>/dev/null)
bts_rc=$?
set -e
if [ "$bts_rc" -ne 0 ]; then
  fail "bts --self-test example exited $bts_rc (RED-state stub expected to fail; GREEN commit lands the real python3-debianbts shim)"
fi
# Validate that the output is JSON. The exact shape (field set,
# report kind discriminator) is 1E.2's concern; the smoke just
# asserts the wire format is parseable.
if ! printf '%s' "$bts_out" | python3 -c 'import json,sys; json.load(sys.stdin)' >/dev/null 2>&1; then
  fail "bts --self-test example emitted non-JSON output: $bts_out"
fi
# Surface the report kind for grep-ability in CI logs. The shape
# can grow in 1E.2 without breaking this assertion; the
# discriminator value is the stable contract.
bts_kind=$(printf '%s' "$bts_out" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("report_kind",""))')
[ -n "$bts_kind" ] || fail "bts --self-test example JSON has no report_kind field"
ok "bts --self-test example emitted canonical JSON (report_kind=$bts_kind)"

echo "ironmaint/debian-tools:0.1 — SMOKE OK"
