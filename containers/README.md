# IronMaint containers

The reproducible execution environment for IronMaint. Spec at
[`../doc/CONTAINER-IMAGES.md`](../doc/CONTAINER-IMAGES.md); this directory
holds the implementation.

## Layout

```
containers/
├── README.md      ← you are here
├── STATUS.md      ← which images are built vs deferred
├── Makefile       ← build / smoke / multi-arch / clean
├── .dockerignore  ← excludes target/, .ironmaint/, .git/, *.swp, *.tmp
├── compose.yml    ← local dev orchestration
├── base/          ← ironmaint/base:0.1
│   ├── Dockerfile
│   └── smoke.sh
├── workspace/     ← ironmaint/workspace:0.1
│   ├── Dockerfile
│   └── smoke.sh
├── debian-tools/  ← ironmaint/debian-tools:0.1
│   ├── Dockerfile
│   └── smoke.sh
├── fedora-tools/  ← ironmaint/fedora-tools:0.1
│   ├── Dockerfile
│   └── smoke.sh
├── agent/         ← ironmaint/agent:0.1
│   ├── Dockerfile
│   └── smoke.sh
└── fake-services/ ← ironmaint/fake-services:0.1
    ├── Dockerfile
    ├── fake-services.py
    └── smoke.sh
```

## Layer graph

```
debian:trixie-slim ────────▶ ironmaint/base:0.1     (substrate; uid 1000; tini; apt: ca-certificates, git, bash, curl, python3)
                              │
                              ├──▶ ironmaint/debian-tools:0.1   (apt: sbuild, lintian, autopkgtest, dpkg-dev, debhelper, gbp, devscripts, diffoscope, reprotest, piuparts, uidmap)
                              ├──▶ ironmaint/agent:0.1           (prebuilt ironclaw 1.4.1 release tarball, sha256-pinned)
                              └──▶ ironmaint/fake-services:0.1  (Python 3 stdlib HTTP server; 6 endpoint ops; 401/429/202)

rust:1.94.0-bookworm ──────▶ ironmaint/workspace:0.1   (Rust 1.94.0 pinned; + build-essential, ca-certificates, python3; USER rust)

fedora:44 ─────────────────▶ ironmaint/fedora-tools:0.1   (dnf: mock, rpmlint, rpm-build, tmt, fedpkg, koji, bodhi-client, packit; uid 1000)
```

`workspace` derives from `rust:1.94.0-bookworm` directly, **not** from
`ironmaint/base`. Spec §4.2 is authoritative; `base` deliberately ships without
a compiler (§4.1).

## Quickstart for a new contributor

```sh
# Build both images for your native arch.
make -C containers build

# Prove they work end-to-end.
make -C containers smoke

# Run the §97 test gate inside the workspace image.
docker compose -f containers/compose.yml run --rm workspace cargo test --workspace

# Run a one-shot cargo command.
docker compose -f containers/compose.yml run --rm workspace \
  cargo run -p xtask -- verify-architecture
```

## Running `ironmaintd` inside compose

`ironmaintd` is an interactive daemon; it does **not** run as a long-lived
compose service. The dev loop is `compose run --rm`:

```sh
# Generate a token once.
echo "test-token-$(openssl rand -hex 16)" > /tmp/ironmaint-token

# Run the daemon inside the workspace image.
docker compose -f containers/compose.yml run --rm \
  -v /tmp/ironmaint-token:/run/secrets/token:ro \
  workspace ironmaintd \
    --state-dir /work/.ironmaint \
    --migrations-dir /work/migrations \
    --token-file /run/secrets/token
```

## macOS notes

### uid considerations

The `workspace` image runs as uid 1000 (`rust` user, created in the Dockerfile
— recent `rust:*` images no longer ship a non-root user, only root + sync).
On macOS your uid is typically 501.
Files written into named volumes (`target/`, `.ironmaint/`) appear owned by
uid 1000 on the host. This is harmless for development. To reset:

```sh
make -C containers clean-volumes
```

### Bind-mount performance

Docker Desktop on macOS uses osxfs / virtiofs for bind mounts, which is slower
than native ext4. `bind.propagation: cached` is set in `compose.yml` as a
mitigation. The first `cargo build` is slow; subsequent builds hit the
named-volume cache and complete in seconds to a minute. The full §97 gate
typically completes in 60–90 seconds warm.

### Apple Silicon

The host is `aarch64-apple-darwin`. The compose file pins
`platform: linux/arm64`. Override for amd64 hosts via
`COMPOSE_PLATFORM=linux/amd64 docker compose …` or by editing the compose file.

## Multi-arch builds

Single-platform builds (default, fast iteration) use the `docker` driver and
`--load` to push the image to the local daemon:

```sh
make -C containers build       # builds for the native arch only
```

Multi-platform builds (`linux/arm64` + `linux/amd64`) require the
`docker-container` buildx driver and a registry. A local `registry:2` on
`:5000` is the canonical target:

```sh
make -C containers registry    # starts registry:2 on :5000 (background)
make -C containers build-multi # builds for arm64 + amd64, pushes to localhost:5000
```

Multi-arch images go through a registry because `docker buildx` with the
`docker-container` driver cannot `--load` multi-platform images to the local
daemon.

## Smoke tests

Each image has a `smoke.sh` that runs **inside** the container and exits
non-zero on any failure. Per spec §11: *"a test that cannot fail is not a
test."* Run them individually or all together:

```sh
make -C containers smoke-base
make -C containers smoke-workspace
make -C containers smoke-debian-tools
make -C containers smoke-fedora-tools
make -C containers smoke-agent
make -C containers smoke-fake-services
make -C containers smoke
```

## `debian-tools` — sandbox requirement

`sbuild` creates its schroot via `sbuild-debootstrap`, which needs to mount
filesystems and use namespaces. Docker's default seccomp profile blocks both,
so the container must run with one of:

```sh
# Preferred: unprivileged user namespaces (no root, no host risk)
docker run --rm --security-opt seccomp=unconfined \
  --security-opt apparmor=unconfined \
  -v "$(pwd)":/work -w /work \
  ironmaint/debian-tools:0.1 sbuild --version

# Fallback: privileged mode (full root in the container)
docker run --rm --privileged \
  -v "$(pwd)":/work -w /work \
  ironmaint/debian-tools:0.1 sbuild --version
```

Spec §4.3: *"`--privileged` on a machine that also runs an agent with API
keys is a real risk and not a theoretical one."* Prefer user namespaces;
fall back to `--privileged` only when the alternative doesn't work on your
Docker runtime. The compose service is not configured for `debian-tools` —
the sbuild-gated jobs are invoked ad-hoc from a developer machine, not from
the §97 build loop.

## `fedora-tools` — sandbox requirement

`mock` builds its chroot via `newuidmap` / user namespaces, and also needs to
mount `/proc` and `/sys` from the host. Mock must therefore run with
`--privileged` (mock deliberately refuses to start without it; there is no
unprivileged-user-namespaces mode comparable to sbuild's unshare backend).
The `smoke-fedora-tools` target runs the version-and-TLS smoke, which doesn't
need privileges; the `mock -r fedora-44-aarch64 --init` and trivial-build gate
lives in `scripts/fedora-tools-build-smoke.sh` and is invoked separately with
`--privileged`. Spec §4.4.

## `agent` — prebuilt binary, no toolchain

The spec (§4.5) says *"`agent` is exception: does not inherit `base`,
needs Rust 1.98.0 and `workspace` needs 1.88.0, two cannot coexist in one
rustup toolchain directory without `--force` gymnastics."* But the
submodule's own release pipeline
(`https://github.com/nearai/ironclaw/releases/tag/ironclaw-v1.4.1`) ships
glibc tarballs for both `aarch64-unknown-linux-gnu` and
`x86_64-unknown-linux-gnu`, SHA256-checked. The `agent` image pulls the
right tarball, verifies it, installs to `/usr/local/bin/ironclaw`, and
ships a thin runtime — no Rust toolchain, no Node, no WebUI. End image
is ~80MB instead of the submodule's ~1.5GB Railway deployment image.

`MINIMAX_API_KEY` and `IRONCLAW_REBORN_SECRET_MASTER_KEY` are NOT baked.
Pass them at run time (`docker run -e MINIMAX_API_KEY=…` or via compose
`env_file`). The smoke explicitly refuses to load if either is set in
the image — that would be a supply-chain defect.

## `fake-services` — six privileged endpoints, four services

Spec §4.6 says this image is "specified, not built" because no
`PrivilegedOperation` producer exists in the tree yet (D-09). Built
anyway so the integration-test surface exists when the producer does
land. Python 3 stdlib HTTP server — single file, ~20MB final image.

Six endpoint operations (spec §4.6 paragraph 1):

| HTTP | Path | Operation |
|---|---|---|
| GET    | /issues/debian-bts/{id}        | provider read (`IssueCapability::provider_id = debian-bts`) |
| GET    | /issues/redhat-bugzilla/{id}   | provider read (`IssueCapability::provider_id = redhat-bugzilla`) |
| POST   | /issues/{provider}/{id}/mutate | `IssueTrackerMutation` (covers both providers) |
| POST   | /release/debian/canonical-push | `CanonicalRepositoryPush` (Debian) |
| POST   | /release/fedora/koji-build     | `RemoteBuildSubmission` (Fedora) |
| POST   | /release/fedora/bodhi-update   | `DistributionUpdateCreation` (Fedora) |

Default: `401` without `Bearer` auth, `429` once an IP blows
`FAKE_SERVICES_RATE_LIMIT` (default 60/min),
`FAKE_SERVICES_PARTIAL_FAILURE_RATE` (default 5%) chance of `502`. A
fake that always returned 200 would be worse than no fake — the spec
is explicit that failure shape must match reality.

Signing is intentionally not exposed — spec §4.6 constraint #3.
There is no `/sign` endpoint; the smoke asserts its absence.

## Cleanup

```sh
make -C containers clean         # remove locally-loaded images and the multi-builder
make -C containers clean-volumes # also drop compose-managed named volumes
```

## Where to find what

- Spec (the canonical design): [`../doc/CONTAINER-IMAGES.md`](../doc/CONTAINER-IMAGES.md)
- Build status (what's done, what's deferred): [`STATUS.md`](STATUS.md)
- D-17 (no CI yet): [`../doc/DEBT.md`](../doc/DEBT.md)
