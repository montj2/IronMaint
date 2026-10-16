# Phase 2.0-C — lock the build execution model

> Pre-implementation readiness PR for Phase 2A (real
> `sbuild` integration). Per `doc/phases/PHASE-2.md`
> §2.0-C, this document answers the five "where / which /
> how" questions about how the IronMaint daemon invokes
> real package builds, *before* any build code lands.
>
> The decisions here are the contract for the 2A
> sub-phases (2A.1–2A.4). The 2A PRs are not
> re-deciding the answers — they are implementing
> them.

## 1. Where `ironmaintd` and `ironmaint-debian-tool` execute

**Decision.** In production, both run inside the
`ironmaint/debian-tools:0.1` container, with `/work`
bind-mounted from the host. The `debian-tools` compose
service (`containers/compose.yml`) runs `ironmaintd`
as its `command`; the executor spawns
`ironmaint-debian-tool` (same binary, on the
container's `PATH`) as a subprocess.

**Where it lives in the tree.**

- `containers/compose.yml` — the new `debian-tools`
  service. Profile-gated `["runtime"]` so the §97
  gate (which only knows the `workspace` profile) is
  not affected.
- `containers/debian-tools/Dockerfile` — the image.
  Already installs `sbuild`, `schroot`, `debootstrap`,
  `uidmap`, `lintian`, `piuparts`, `autopkgtest`,
  `dpkg-dev`, `debhelper`, `git-buildpackage`,
  `devscripts`, `diffoscope`, `reprotest`,
  `python3-debianbts` and runs as uid 1000
  (`ironmaint`).

**What to change to make the decision real.**

- The compose service mounts the host's working tree
  at `/work` and the named volume
  `ironmaint_sbuild-buildroots` at `/var/lib/sbuild`.
- The §97 gate (`ironmaint/workspace:0.1`) is
  unchanged. `ironmaintd` is not part of the gate.

**Regression test.** The `cargo test --workspace`
suite continues to pass without Docker. The
`debian-tools` smoke assertions are opt-in via
`make -f containers/Makefile smoke-debian-tools`.

## 2. Where persistent sbuild buildroots live and how they are provisioned

**Decision.** Named volume
`ironmaint_sbuild-buildroots` mounted at
`/var/lib/sbuild` inside the `debian-tools` service.
First-use provisioning via `debootstrap` (installed
in the image; `sbuild` uses it directly per its
`$sbuild_config` default on trixie) creating
`<suite>-<arch>` chroots on demand. Reused across
invocations.

The buildroot path is operator-overridable via
`--sbuild-buildroot-dir <path>` (default
`/var/lib/sbuild`); the path is also configurable via
the `IRONMAINT_SBUILD_BUILDROOT_DIR` env var. The
flag wins, the env var is the fallback, the default
is the compiled-in `/var/lib/sbuild`.

**Where it lives in the tree.**

- `bins/ironmaintd/src/config.rs` — the
  `sbuild_buildroot_dir` field on `RuntimeConfig`.
- `containers/compose.yml` — the
  `ironmaint_sbuild-buildroots` named volume.

**What to change to make the decision real.**

- The `ironmaint_sbuild-buildroots` named volume is
  declared at the top of `compose.yml` alongside
  the existing four.
- The compose service's `command:` line passes
  `--sbuild-buildroot-dir /var/lib/sbuild`.

**Regression test.** A unit test in
`bins/ironmaintd/tests/config.rs` asserts the
default is `/var/lib/sbuild` and that the flag
overrides the env var. (The host's `sbuild` binary
does not need to exist for this test — only the
config parser does.)

## 3. Which Debian suite and architecture are selected for a candidate

**Decision.** A new `BuildTarget { suite, arch }`
struct in `ironmaint-adapter-api` represents the
per-candidate build target. The default
implementation returns `BuildTarget { suite:
"sid", arch: host_arch() }` — the moving-target
suite and the binary's host architecture. The
default is decided by the adapter via a new
optional `BuildTargetProvider` trait. The first
slice is single-arch (host); per-candidate
overrides and multi-arch are 2A.2's job.

The runtime does NOT have a CLI flag for the build
target — the target is per-candidate, decided by
the adapter, not the operator. (The 2A.2 PR will
add the per-candidate override path, where the
candidate's metadata may carry an explicit target.)

**Where it lives in the tree.**

- `crates/ironmaint-adapter-api/src/build_target.rs` —
  the `BuildTarget` struct, the `BuildTargetProvider`
  trait, the `default_for_candidate` constructor,
  and `host_arch()` mapping.
- `adapters/debian/src/build_target.rs` — the
  production Debian adapter's `BuildTargetProvider`
  impl.
- `adapters/debian-stub/src/build_target.rs` —
  the stub's `BuildTargetProvider` impl.
- `adapters/fedora-stub/src/build_target.rs` —
  the Fedora stub's `BuildTargetProvider` impl
  (returns `rawhide` + host arch; the Fedora
  follow-on will refine).

**What to change to make the decision real.**

- `host_arch()` maps `std::env::consts::ARCH` to
  the Debian name: `aarch64 → arm64`,
  `x86_64 → amd64`, `i686 → i386`, `arm → armhf`,
  `powerpc64 → ppc64el`, `riscv64 → riscv64`,
  `s390x → s390x`. Other arches return
  `unknown-<arch>` (the operator can pass an
  explicit target in 2A.2).

**Regression test.** An integration test in
`crates/ironmaint-adapter-api/tests/`
asserts that `<DebianStubAdapter as
BuildTargetProvider>::build_target(&ctx)` returns
the default. A unit test in the same module
asserts `host_arch()` returns a known Debian
architecture for the compile-time host.

## 4. How the build environment is isolated from agent credentials and unrelated host files

**Decision.** Three layers of isolation, each
established by an existing mechanism:

1. **Subprocess env boundary.** 1.1.I's
   `env_clear() + baseline + per-tool + per-request`
   (in `crates/ironmaint-executor/src/process.rs`)
   seals the env boundary for every tool
   subprocess. The `debian.build.sbuild` tool's
   per-tool env is empty (the tool declares no
   `with_env`), and the per-request env carries
   only the build's working directory and the
   input paths. No `IRONMAINT_*`, `AWS_*`, `LD_*`,
   or proxy variables can leak. (The 1.1.I test
   `tests/env_boundary.rs` proves this.)

2. **Build input is a fresh stage, not a
   bind-mount.** The build input is staged as a
   copy at
   `/var/lib/ironmaint/build-input/<fingerprint>/`
   (or, in the workspace service, under
   `/work/jobs/<job-id>/build-input/<fingerprint>/`).
   The original candidate tree is read-only; the
   build cannot modify the candidate.

3. **The agent image is a separate container.**
   `ironmaint/agent:0.1` is a separate image
   with no build-image access. The agent's API
   keys (e.g. `OPENAI_API_KEY`) are injected into
   the agent container at runtime, never baked.
   The build container has no such keys; the
   agent's MCP client connects to the daemon's
   loopback socket (which is itself unauthenticated
   inside the `debian-tools` container per the
   §50 trust model).

**Where it lives in the tree.**

- `crates/ironmaint-executor/src/process.rs` —
  env composition (1.1.I, unchanged).
- `crates/ironmaint-executor/tests/env_boundary.rs` —
  regression test (1.1.I, unchanged).
- `containers/debian-tools/Dockerfile` — no API
  keys are baked into the image.

**What to change to make the decision real.**
Nothing — 2.0-C is documenting the existing
isolation mechanisms. 2A.3 will add the build
input staging directory and the read-only mount.

**Regression test.** The existing 1.1.I
`env_boundary.rs` tests pass unchanged. The 2A.3
PR will add a test for the staging-directory
invariant.

## 5. Whether user namespaces are sufficient for the required sbuild sandbox, without blanket `--privileged`

**Decision.** Yes, for the first slice. The
`ironmaint/debian-tools:0.1` image ships `uidmap`
and `sbuild`'s `--unshare` backend runs without
`CAP_SYS_ADMIN` in user namespaces.

Default mode: **unprivileged** (`sbuild --unshare`).
Configured via `--sbuild-unprivileged=true` (default
in `RuntimeConfig`). Fallback: `--sbuild-unprivileged=false`
switches to the schroot backend, which requires
`--privileged` on the container. The flag has an
env-var fallback (`IRONMAINT_SBUILD_UNPRIVILEGED`)
following the same precedence as the other
runtime flags.

The smoke test in `containers/debian-tools/smoke.sh`
asserts `sbuild --version` under `unshare -U` exits
0. If the unprivileged mode fails (e.g. on a
runtime that disables user namespaces), the
operator sees a clear error and can opt into
`--no-sbuild-unprivileged` to switch backends.

**Where it lives in the tree.**

- `bins/ironmaintd/src/config.rs` — the
  `sbuild_unprivileged: bool` field on
  `RuntimeConfig` (default `true`).
- `containers/debian-tools/smoke.sh` — the
  unprivileged-mode assertion.
- `containers/debian-tools/Dockerfile` — the
  image label documents the default and fallback.

**What to change to make the decision real.**

- The image label's
  `org.opencontainers.image.description` is
  updated to record the default mode and fallback
  (was: "Requires `--privileged` or `SYS_ADMIN`+user
  namespaces for the buildroot"; now: "Default:
  unprivileged user namespaces (`sbuild --unshare`).
  Fallback: `--privileged` for the schroot backend").

**Regression test.** The new smoke assertion
`unshare -U sbuild --version` exits 0 inside the
`debian-tools:0.1` image. The test is opt-in via
`make -f containers/Makefile smoke-debian-tools`
and is not part of the §97 gate.

---

## Out of scope (what 2.0-C is NOT)

- A real `sbuild` invocation. (PR 2A.3.)
- A `BuildCapability` impl in the real adapter
  with a real `BuildPlan`. (PR 2A.2.)
- A new MCP tool. (None needed — the build is
  invoked by the executor, not the MCP.)
- A `BuildPlan` that carries a `BuildTarget`
  field. (2A.2's call.)
- The Fedora parallel. (Phase 2 follow-on.)
- The `ironmaint-debian-tool` install recipe
  inside the `debian-tools` image (currently
  bind-mounted; 2A.3 will fix it).
- A new `AdapterCapability` or `EvidenceKind`
  or `GateStage`. (All exist; 2.0-C does not
  need new ones.)
- An automatic transition into
  `InfrastructureBlocked` (§17 Q8 / D-09 stays
  open per the spec).
- A change to the §97 gate. The new smoke
  additions are opt-in (`make
  smoke-debian-tools`).

## §106 invariants preserved

- **State machine:** unchanged.
- **Persistence:** unchanged.
- **Executor:** unchanged (the `ToolDefinition`
  trait shape is the same as 1.1.I left it).
- **Candidate identity:** unchanged (BLAKE3
  fingerprint over the versioned byte
  representation).
- **Evidence:** unchanged.
- **State ownership:** unchanged.
- **MCP boundary:** unchanged. The 15-tool count
  is unchanged; 2.0-C is a config / compose / doc
  / trait PR, not a tool PR.
- **IronClaw integration:** unchanged.

## §97 gate result (target)

```text
cargo fmt --check                                                                clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings    clean
cargo run -p xtask --locked -- verify-architecture                               no violations
cargo run -p xtask --locked -- verify-schemas                                    no drift
cargo run -p xtask --locked -- verify-migrations                                 clean
cargo run -p xtask --locked -- verify-mcp-schemas                                no drift
cargo run -p xtask --locked -- verify-seams                                      no violations
cargo test --workspace --locked                                                  all pass
cargo test --workspace --features integration --locked                           582 pass / 0 fail / 1 ignored
```

The `debian-tools` smoke additions run only inside
the `debian-tools:0.1` image (via `make -f
containers/Makefile smoke-debian-tools`); they are
not part of the §97 gate.
