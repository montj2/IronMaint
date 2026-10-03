#!/usr/bin/env bash
# gate.sh — the §97 acceptance gate, run inside ironmaint/workspace:0.1.
#
# This is the whole workflow. From a Mac, with Docker:
#
#   make -f containers/Makefile gate
#
# Everything below is the command set from doc/phases/PHASE-0B.md §97,
# unchanged, in one container with the source bind-mounted at /work and
# CARGO_TARGET_DIR on a named volume. The first run compiles; every run
# after it is a link. The named volume is what makes that true — a baked
# `target/` layer in the image would be invalidated by every source
# change, which is the opposite of when you want the cache. See
# CONTAINER-IMAGES.md §4.2 and the SUPERSEDED block in the workspace
# Dockerfile.
#
# Why a script and not a Makefile recipe: the gate stops at the first
# failure and says which command failed and how long it took. A Makefile
# recipe with eight commands reports "make: *** [gate] Error 1" and
# leaves you to work out which one. Six of these compile the workspace;
# knowing the third one failed rather than the eighth is worth the file.
#
# GATE_FAST=1 runs everything that is not a test run — fmt, clippy, and
# the five verifiers. That is the right loop while you are editing; a
# full `cargo test --workspace` is the right loop before you commit.
# Clippy is in the fast set on purpose: `--all-targets` compiles the
# test binaries anyway, so gate-fast is never much cheaper than the real
# gate once the build cache is warm.

set -euo pipefail

WORKSPACE_MARKER=rust-toolchain.toml

# ---- preflight ------------------------------------------------------------
# Cheap, and it catches the failure mode this script cannot otherwise
# explain: run outside the container, every cargo command works and the
# gate goes green on the developer's macOS toolchain, which is a
# different toolchain from the one the image pins. The marker file is
# what distinguishes "the gate ran" from "the gate ran somewhere else".
if [ ! -f "$WORKSPACE_MARKER" ]; then
  printf 'FAIL: %s not found in %s\n' "$WORKSPACE_MARKER" "$PWD" >&2
  cat >&2 <<'EOF'

This gate is meant to run inside ironmaint/workspace:0.1, which pins the
same toolchain as rust-toolchain.toml. Run it through the Makefile:

    make -f containers/Makefile gate

Running it directly on the host checks a different toolchain than the
one the image and the spec agree on, which is the property §11.1 exists
to protect.
EOF
  exit 1
fi

started=$(date +%s)
step_no=0

# step <description> <command...>
# Runs the command, prints how long it took on success, and on failure
# prints the step number and the command that failed. The exit code is
# the command's own — not a normalised 1 — so a caller can tell a build
# failure from a verifier failure.
step() {
  local desc=$1
  shift
  step_no=$((step_no + 1))
  printf '\n\033[1m[%d/%d] %s\033[0m\n' "$step_no" "$TOTAL_STEPS" "$desc"
  printf '        $ %s\n' "$*"
  local t0 t1 rc=0
  t0=$(date +%s)
  "$@" || rc=$?
  t1=$(date +%s)
  if [ "$rc" -ne 0 ]; then
    printf '\n\033[1;31mFAIL at step %d: %s\033[0m (exit %d, %ds)\n' \
      "$step_no" "$desc" "$rc" "$((t1 - t0))" >&2
    printf '       command: %s\n' "$*" >&2
    exit "$rc"
  fi
  printf '        ok (%ds)\n' "$((t1 - t0))"
}

# ---- the gate -------------------------------------------------------------
# --locked on every cargo invocation. Cargo.lock is committed; a gate
# that resolves a different dependency graph than the one under review
# is not testing the thing under review.

if [ "${GATE_FAST:-0}" = "1" ]; then
  TOTAL_STEPS=7
else
  TOTAL_STEPS=8
fi

printf '\033[1mIronMaint §97 acceptance gate\033[0m\n'
printf 'worktree  %s\n' "$PWD"
printf 'toolchain %s\n' "$(rustc --version)"
printf 'target    %s\n' "${CARGO_TARGET_DIR:-<unset, using ./target>}"

step "formatting" \
  cargo fmt --check

step "lints (workspace, all targets, all features)" \
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

step "architecture — no core→adapter edges" \
  cargo run -p xtask --locked -- verify-architecture

step "JSON schema snapshots" \
  cargo run -p xtask --locked -- verify-schemas

step "SQLite migration snapshots" \
  cargo run -p xtask --locked -- verify-migrations

step "MCP tool schema snapshots" \
  cargo run -p xtask --locked -- verify-mcp-schemas

step "static seam checks (S1, S2, S4, S5)" \
  cargo run -p xtask --locked -- verify-seams

if [ "${GATE_FAST:-0}" = "1" ]; then
  total=$(( $(date +%s) - started ))
  printf '\n\033[1;32mGATE PASSED\033[0m — %d steps, %ds (GATE_FAST, no tests run)\n' \
    "$TOTAL_STEPS" "$total"
  exit 0
fi

step "tests (workspace, then workspace + integration)" \
  bash -c 'cargo test --workspace --locked &&
           cargo test --workspace --features integration --locked'

total=$(( $(date +%s) - started ))
printf '\n\033[1;32mGATE PASSED\033[0m — %d steps, %ds\n' "$TOTAL_STEPS" "$total"
