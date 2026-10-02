# IronMaint image build status

Last verified: 2026-10-01

| Image | Tag | Status | Smoke | Notes |
|---|---|---|---|---|
| `ironmaint/base` | `0.1` | BUILT | PASS | `debian:trixie-slim` + tini + ca-certificates + git + bash + curl + python3 + non-root uid 1000 |
| `ironmaint/workspace` | `0.1` | BUILT | PASS | `rust:1.88.0-bookworm` + build-essential + ca-certificates + python3; prebuilt `target/` DEFERRED to CI PR |
| `ironmaint/debian-tools` | `0.1` | NOT BUILT | — | Spec §4.3; follow-up PR |
| `ironmaint/fedora-tools` | `0.1` | NOT BUILT | — | Spec §4.4; arm64/amd64 question (§8) needs day-one test before Dockerfile |
| `ironmaint/agent` | `0.1` | NOT BUILT | — | Spec §4.5; `rust:1.98.0`; ironclaw submodule required |
| `ironmaint/fake-services` | `0.1` | NOT BUILT | — | Spec §4.6; Phase 1 |

## Prebuilt debug deps — DEFERRED

Spec §4.2 calls for `cargo build --workspace --all-targets` +
`cargo build --workspace --tests` to be baked into the workspace image so a
cold CI run is a link step rather than a compile. There is no CI yet (spec §10,
D-17 in `../doc/DEBT.md`). Deferred to the PR that adds CI. The named-volume
approach in `compose.yml` handles dev iteration in the meantime.

## Day-one risks before the next image lands

Per spec §8, these should be tested on day one of any follow-up PR:

1. **`mock` on `linux/arm64` (Fedora 44).** Run
   `docker run --rm fedora:44 dnf install -y mock && mock -r fedora-44-x86_64 --repos …`.
   If it works under emulation, fine; if not, document and fall back to amd64-only.
2. **`sbuild` on `linux/arm64` (Debian trixie).** Run
   `docker run --rm debian:trixie-slim bash -c 'apt-get update && apt-get install -y sbuild && sbuild --version'`.
