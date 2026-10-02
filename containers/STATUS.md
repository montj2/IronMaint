# IronMaint image build status

Last verified: 2026-10-02

| Image | Tag | Status | Smoke | Notes |
|---|---|---|---|---|
| `ironmaint/base` | `0.1` | BUILT | PASS | `debian:trixie-slim` + tini + ca-certificates + git + bash + curl + python3 + non-root uid 1000 |
| `ironmaint/workspace` | `0.1` | BUILT | PASS | `rust:1.94.0-bookworm` + build-essential + ca-certificates + python3; prebuilt `target/` DEFERRED to CI PR |
| `ironmaint/debian-tools` | `0.1` | BUILT | PASS | `ironmaint/base:0.1` + sbuild + lintian + autopkgtest + dpkg-dev + debhelper + git-buildpackage + devscripts + diffoscope + reprotest + piuparts + uidmap; schroot built at first run; `--privileged` required |
| `ironmaint/fedora-tools` | `0.1` | BUILT | PASS | `fedora:44` (NOT from base, spec §3) + mock + rpmlint + rpm-build + tmt + fedpkg + koji + bodhi-client + packit; `--privileged` required for buildroot |
| `ironmaint/agent` | `0.1` | BUILT | PASS | `ironmaint/base:0.1`; prebuilt `ironclaw 1.4.1` tarball from `nearai/ironclaw` `ironclaw-v1.4.1` (SHA256-pinned, no compilation); spec §4.5 env-var contract; secrets NOT baked |
| `ironmaint/fake-services` | `0.1` | BUILT | PASS | `ironmaint/base:0.1`; Python 3 stdlib HTTP server; 6 endpoint ops (debian-bts / redhat-bugzilla reads, IssueTrackerMutation, CanonicalRepositoryPush, RemoteBuildSubmission, DistributionUpdateCreation); 401 unauth, 429 rate, occasional 502; spec §4.6 |

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
