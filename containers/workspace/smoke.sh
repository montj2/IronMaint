#!/usr/bin/env bash
# smoke.sh — ironmaint/workspace:0.1
# Runs INSIDE the container. Must exit non-zero on any failure.
# Spec §11.1: the toolchain pin is a trap. This test guards the pin.
#
# Invocation patterns:
#   docker run --rm -v "$PWD":/work ironmaint/workspace:0.1 \
#     /usr/local/bin/smoke.sh
#   make -C containers smoke-workspace
#
# /work should be the IronMaint workspace source tree (bind-mounted in
# compose, or the repo root in the dev invocation above).

set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
ok()   { printf 'OK:   %s\n' "$*"; }

# CRITICAL: rustup reads rust-toolchain.toml from the current directory and
# auto-installs / switches to the toolchain specified there. The IronMaint
# workspace has rust-toolchain.toml pinned to 1.94.0, so running smoke.sh
# from /work would mask a broken image (rustup would download 1.94.0 on
# demand and the test would falsely pass).
#
# cd to /tmp before any toolchain queries so we test the image's actual
# default toolchain, not the workspace's pinned one.
cd /tmp

# ---- 1. toolchain pin (the whole point of the image) --------------------
# If the image resolves to a newer toolchain, the image is worse than no
# image — it converts an unenforced MSRV claim into an actively false one.
# Spec §4.2 / §11.1.
expected="1.94.0"
got=$(rustc --version | awk '{print $2}')
[ "$got" = "$expected" ] || fail "rustc is $got, expected $expected"
ok "rustc pinned at $expected"

# rustup reports the same thing — guards against rustc being a different
# toolchain than what rustup would invoke. Reads the image's default
# toolchain (no rust-toolchain.toml in /tmp).
active=$(rustup show active-toolchain 2>/dev/null | awk '{print $1}')
case "$active" in
  1.94.0*) ok "rustup default toolchain is $active" ;;
  *) fail "rustup default toolchain is '$active', expected 1.94.0*" ;;
esac

# ---- 2. cargo tooling on PATH --------------------------------------------
cargo --version        | grep -q "1.94" || fail "cargo --version does not match 1.94.x"
cargo clippy --version | grep -q "clippy" || fail "cargo clippy --version broken"
cargo fmt --version    | grep -q "fmt"   || fail "cargo fmt --version broken"
rustfmt --version      | grep -q "rustfmt" || fail "rustfmt missing"
ok "cargo, clippy, rustfmt present"

# ---- 3. minimal compile proves the linker path works ---------------------
# If build-essential wasn't installed, this fails with "linker `cc` not
# found". Without this check, a Dockerfile that lost build-essential would
# pass version checks but fail any cargo build.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cat > "$work/hello.rs" <<'EOF'
fn main() { println!("ok"); }
EOF
rustc -o "$work/hello" "$work/hello.rs" || fail "rustc could not link hello.rs"
[ "$("$work/hello")" = "ok" ] || fail "hello binary did not print 'ok'"
ok "rustc can compile and link a trivial program"

# ---- 4. cargo can resolve a small workspace ------------------------------
# This is the closest we can get to "the image can build the IronMaint
# workspace" without bind-mounting the source. It proves cargo's resolver,
# the index, and the lockfile mechanism all work — the same machinery
# cargo test uses on the real tree.
mkdir -p "$work/mini/hello/src"
cat > "$work/mini/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["hello"]
EOF
cat > "$work/mini/hello/Cargo.toml" <<'EOF'
[package]
name = "hello"
version = "0.0.0"
edition = "2021"
[dependencies]
serde = { version = "1", features = ["derive"] }
EOF
cat > "$work/mini/hello/src/main.rs" <<'EOF'
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
struct Greet { msg: String }
fn main() {
    let g = Greet { msg: "ok".into() };
    println!("{}", g.msg);
}
EOF
(cd "$work/mini" && cargo build --quiet) || fail "cargo build on minimal workspace failed"
"$work/mini/target/debug/hello" | grep -q "ok" || fail "minimal workspace binary did not run"
ok "cargo build on minimal workspace (with a serde dep) succeeded"

# ---- 5. CA roots (TLS) carry over from base -----------------------------
# The workspace image derives from rust:* directly, not from
# ironmaint/base, so we re-install ca-certificates here. This guards
# against a future bump of the rust image that drops it.
#
# crates.io returns 403 to unauthenticated HEAD/GET on the front page
# (the crate download endpoints at static.crates.io return 200). The
# point of this check is that TLS works — i.e., the request reached
# the server and got *some* HTTP status. If CA roots are gone, curl
# exits with code 60 (CURLE_PEER_FAILED_VERIFICATION) and the pipeline
# fails under pipefail before we reach the case statement.
#
# Same set -e dance as base/smoke.sh step 4 — capture curl's exit code
# and produce a useful FAIL message.
set +e
out=$(curl -sS --max-time 10 -o /dev/null -w "%{http_code}" https://crates.io 2>&1 | tr -d '\r')
curl_rc=$?
set -e
if [ "$curl_rc" -ne 0 ]; then
  fail "TLS to crates.io failed: curl exited $curl_rc (likely missing ca-certificates — §11.2)"
fi
case "$out" in
  2*|3*|4*|5*) ok "TLS to crates.io completed (HTTP $out) — CA certs present" ;;
  *) fail "TLS to crates.io returned unexpected status: '$out'" ;;
esac

echo "ironmaint/workspace:0.1 — SMOKE OK"