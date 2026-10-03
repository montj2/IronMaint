# IronMaint container images — specification

**Status:** spec, not implementation. Written 2026-10-01 for a separate team to build
and wire up. Nothing in this document has been built; every version, digest and
package name below is a **proposal to verify**, not an observed fact, unless it is
marked *(verified)*.

**Scope:** a small series of container images that make IronMaint's execution and
acceptance environment reproducible and locally runnable, and that make
§102 item 30 checkable on a machine that has no `ironclaw` binary.

---

## 1. What this is for — and what it is not

This section is first because the honest framing determines whether the work is
judged correctly when it lands.

### 1.1 What it will not do

**These images would not have prevented any of the fourteen seam defects found
during Phase 0B.10.** That is not a hedge; it is a measured result, and it is
recorded here so nobody later credits the images with work they did not do.

All fourteen had this shape: *a component modelled something correctly, and no
production path produced the input.* Each was a missing seam between two things
that were individually correct and individually tested. Thirteen were found by
driving the real command surface against a real store. The last two were found by
running the CLI against a live daemon's state directory.

None of them needed a packaging toolchain. SQLite is embedded and links into the
test binary. `sbuild`, `mock`, `lintian` and `rpmlint` were **not installed on the
machine where all fourteen were found**, and every one of them was found anyway.

**The blind spot is structural, not environmental.** A container running the
identical suite finds the identical nothing. If the goal were "catch today's
bugs", the correct instrument is a seam verifier (`doc/SEAM-VERIFICATION.md`),
not a container.

### 1.2 What it will do

Three real things, in descending order of value:

1. **Make §102 item 30 checkable.** The one Definition-of-Done item that is
   currently blocked needs the `ironclaw` binary, which needs Rust **1.98.0**
   against this workspace's **1.88** MSRV. A container is the only sane way to
   have both.

2. **Open a defect class that is currently unproven rather than clean.** Every
   packaging tool in this repository is a stub. `lintian`'s real output,
   `sbuild`'s real failure modes, `dpkg-buildpackage`'s stderr, `mock` buildroot
   behaviour, and the `GateResult::Fail` versus `InfrastructureFailed`
   distinction have **never been exercised against anything real**. That is the
   class a Phase 1 real adapter walks straight into, and nothing in the current
   gate would catch it.

3. **Turn the test environment from an ambient input into a pinned one.** Today
   the MSRV is *declared* in `rust-toolchain.toml` and enforced by nothing —
   there is no CI workflow in this repository at all (§97 runs on whatever the
   developer has installed). That is a real hole, though a different hole from
   the one that bit.

### 1.3 The one tension with the existing spec

`CLAUDE.md` lists **"container execution"** among Phase 0A's explicit non-goals,
and PHASE-0B §99 repeats it. That constraint is about the **daemon** — the
product must not spawn containers to run tools. It is a runtime-architecture
constraint, not a prohibition on a developer test environment.

This spec does **not** implement a per-tool container sandbox. §4.5 below is
written so it can be read either way, but the resolution the architect picked is
the second option: the images are tools IronMaint *shells out to* over a
container boundary, and `sbuild`/`mock` continue to provide their own chroot
isolation exactly as the spec already assumes.

**If a reviewer reads §1.3 the other way and decides the daemon should spawn these
images, that is a different and much larger piece of work** — it changes §33's
execution model, and it should be its own phase with its own spec. This document
does not authorise it.

---

## 2. The series

Five images. Four are in scope now; the fifth is Phase 1 and is specified but not
built.

| Image | Role | Size target | Phase |
|---|---|---|---|
| `ironmaint/base:0.1` | common substrate — CA roots, `git`, `bash`, `curl`, `python3`, a non-root user | ~120 MB | 0B.11 |
| `ironmaint/workspace:0.1` | the Rust 1.88 toolchain + prebuilt debug deps; runs the whole §97 gate | ~2.5 GB | 0B.11 |
| `ironmaint/debian-tools:0.1` | `sbuild`, `lintian`, `autopkgtest`, `piuparts`, `dpkg-dev`, `git-buildpackage` | ~900 MB | 0B.11 |
| `ironmaint/fedora-tools:0.1` | `mock`, `rpmlint`, `spectool`, `rpm-build`, `tmt`, `koji`/`bodhi`/`packit` clients | ~1.2 GB | 0B.11 |
| `ironmaint/agent:0.1` | `ironclaw` built at Rust 1.98.0, headless, MiniMax-configurable | ~1.5 GB | 0B.11 |
| `ironmaint/fake-services:0.1` | local stand-ins for BTS / Koji / Bodhi / Bugzilla | ~300 MB | **Phase 1 — specified, not built** |

Sizes are targets, not commitments. `mock`'s buildroot machinery and the
IronClaw binary dominate; see §8 for the arm64 caveat, which is the largest
schedule risk in this document.

### 2.1 Why `workspace` exists at all, given "lightweight"

The brief asked for lightweight images that spin up quickly. The `workspace`
image is the deliberate exception and it is worth being explicit about why.

Without it, the environment is *"whatever is installed on the developer's
machine"*, which is precisely the condition under which D-14 shipped: a bug
affecting 100% of real jobs passed CI because every test hand-built its log
rather than driving `CreateJob`. A pinned environment is worth ~2.5 GB.

It is also the only image that makes the MSRV claim testable. `rust-toolchain.toml`
*(verified)* pins `channel = "1.88.0"` and the workspace declares
`rust-version = "1.88"`, but nothing checks that the code builds on 1.88 — a
developer on stable 1.94 will never notice a 1.89-only API. The image is where
that becomes a fact.

**Iteration strategy, and the honest cost:** mount the source and `target/`
across, so editing a file does not rebuild the image. Expect the first
`cargo build` after a `Cargo.toml` change to be slow. Do not attempt to
vendor-regenerate on every edit; the image is a build environment, not a
hot-reload system.

---

## 3. Layer graph

```text
debian:bookworm-slim ──┐
fedora:42 ─────────────┼──▶ ironmaint/base ──▶ workspace
rust:1.88.0-bookworm ──┘                        ├──▶ debian-tools
                                                ├──▶ fedora-tools
                                                └──▶ agent   (FROM rust:1.98.0)
```

`debian-tools` and `fedora-tools` **must not** derive from each other. Their base
distributions differ, and the whole point is that a Debian-shaped check runs on a
Debian-shaped system.

`agent` is the exception: it does **not** inherit `base`, because it needs Rust
1.98.0 and `workspace` needs 1.88.0, and the two cannot coexist in one rustup
toolchain directory without `--force` gymnastics that make the MSRV claim
meaningless. It gets its own Rust base and the same CA/git/bash/curl/python3
packages repeated. That duplication is the correct trade.

---

## 4. Per-image specification

### 4.1 `ironmaint/base:0.1`

**Purpose:** one place for the packages every other image needs, so §4.6's
argument about not duplicating is honest.

| Requirement | Why |
|---|---|
| `ca-certificates` | **`sqlx` and `reqwest` are configured `tls-rustls-ring-native-roots`** *(verified, `Cargo.toml`)* — rustls with *native roots* reads the OS trust store. Without this package, TLS fails at runtime in a way that looks like a network fault. This is the single most likely thing to be missed. |
| `git` | `cargo` resolves git-sourced dependencies; and §24 makes `git` a first-class runtime dependency of the workspace manager. |
| `bash`, `curl`, `python3` | `scripts/ironclaw-e2e.sh` requires all three *(verified)* — its embedded JSON helper is a `python3 -c` at lines 59-62, and it has no `jq` dependency. |
| non-root user, uid 1000 | matches the convention the IronClaw submodule's own images already use |
| `tini` or `--init` | see §5.3 |

**Not included, deliberately:** no compiler, no package-manager tooling, no
`sqlite3` CLI. The store is reached through `sqlx`, never through a shell.

### 4.2 `ironmaint/workspace:0.1`

**Contents**

- `rust:1.88.0-bookworm` *(the tag must match `rust-toolchain.toml` exactly — see
  the warning in §11.1)*
- `build-essential` — `rustc` needs a linker. There are **no native build steps
  in this workspace** *(verified: the three `build.rs` hits are Rust modules, not
  build scripts, and no crate pulls `openssl`/`rusqlite`/`ring`/`git2`
  directly)* — so this is the whole of the C toolchain requirement.
- A prebuilt `cargo build --workspace --all-targets` and
  `cargo build --workspace --tests` of `target/debug`, so a cold `cargo test` is
  a link step rather than a compile.
- `cargo install cargo-nextest` — optional; if adopted, §97's `cargo test` line
  must be updated in the same change, or the gate silently diverges from the one
  people run on a laptop.

**Entry point:** none. This image is `docker run --rm` + a command.

**Health check:** none needed.

**The one thing it must prove**

```sh
docker run --rm -v "$PWD":/w -w /w ironmaint/workspace:0.1 \
  sh -c 'rustc --version && cargo test --workspace --features integration'
```

`rustc --version` must report **1.88.0**. If the image resolves to a newer
toolchain, the image is worse than no image, because it converts an unenforced
claim into an actively false one.

### 4.3 `ironmaint/debian-tools:0.1`

**Base:** `debian:bookworm-slim` — *proposed; verify against current Debian
stable and against the release IronMaint's `DistributionRelease` values
represent.* `adapters/debian-stub` fixtures use `sid`; the image being `bookworm`
is a mismatch worth resolving deliberately rather than by accident.

**Packages, mapped to the capability keys the stub already plans**
*(verified: `adapters/debian-stub/src/build.rs:36-55`)*:

| Tool key | Binary | Package | Mandatory |
|---|---|---|---|
| `debian.build.sbuild` | `sbuild` | `sbuild` | yes |
| `debian.qa.lintian` | `lintian` | `lintian` | yes |
| `debian.qa.piuparts` | `piuparts` | `piuparts` | no |
| `debian.test.autopkgtest` | `autopkgtest` | `autopkgtest` | yes |
| *(not yet a key)* | `dpkg-buildpackage`, `dpkg-dev`, `debhelper` | `dpkg-dev`, `debhelper` | yes |
| *(not yet a key)* | `gbp` | `git-buildpackage` | yes |
| *(not yet a key)* | `uscan` | `devscripts` | yes |
| *(not yet a key)* | `diffoscope`, `reprotest`, `debdiff` | `diffoscope`, `reprotest`, `debian-diff` | no |

**Buildroots are NOT baked in.** `sbuild` needs a schroot, which
`sbuild-debootstrap` creates on first use from the network. Baking one would
triple the image and freeze a snapshot that goes stale. The image ships the
*binaries and configuration templates*; buildroots are created into a mounted
volume at run time.

**This is the single most important design decision in the image set** and it is
the direct answer to "lightweight". A `mock` buildroot for Fedora is measured in
gigabytes; an `sbuild` schroot is hundreds of megabytes. Neither belongs in a
layer.

**Sandbox requirement:** `sbuild` needs either unprivileged user namespaces or
root. On Docker Desktop this means either `--privileged` or
`--security-opt seccomp=unconfined` plus a documented `SYS_ADMIN` grant. The
image's README must state which, and the spec should prefer namespaces, because
`--privileged` on a machine that also runs an agent with API keys is a real
risk and not a theoretical one.

### 4.4 `ironmaint/fedora-tools:0.1`

**Base:** `fedora:42` — *proposed; verify against the current Fedora release.*
Fedora moves fast and the image digest will need periodic bumps. Pin by digest
and automate the bump, or the image is a snapshot of a distribution that was
current in October 2026.

**Packages, mapped to the capability keys the stub already plans**
*(verified: `adapters/fedora-stub/src/build.rs`)*:

| Tool key | Binary | Package | Mandatory |
|---|---|---|---|
| `fedora.build.mock` | `mock` | `mock` | yes |
| `fedora.qa.rpmlint` | `rpmlint` | `rpmlint` | yes |
| `fedora.qa.spectool` | `spectool` | `spectool` | no |
| `fedora.test.functional` | `tmt` | `tmt` | yes |
| *(not yet a key)* | `rpmbuild` | `rpm-build` | yes |
| *(not yet a key)* | `fedpkg` | `fedpkg` | yes |
| *(not yet a key)* | `koji`, `bodhi`, `packit` | `koji`, `bodhi`, `packit` | Phase 1 only |

**The `mock` problem, stated plainly.** `mock` creates buildroots with `mock -r`,
which is a multi-gigabyte download and a multi-minute operation. A `fedora-tools`
image that has not run `mock -r` has not been tested. The image should therefore
be accompanied by a **smoke check that creates one buildroot and runs one
trivial build**, and that check is a definition-of-done item, not a nice-to-have.
Without it the image ships untested in the only way that matters.

**arm64 is the risk here.** See §8.

### 4.5 `ironmaint/agent:0.1`

**Purpose:** run the real IronClaw agent against a real `ironmaintd` over real
HTTP, and close §102 item 30.

**Build**

| Requirement | Value | Source |
|---|---|---|
| Rust toolchain | **1.98.0, stable** | `doc/vendor/ironclaw/rust-toolchain.toml:23` *(verified)* |
| Minimum viable | 1.96.0 | `doc/vendor/ironclaw/Cargo.toml:20` *(verified)* |
| Edition | 2024 | *(verified)* |
| Frontend | **skipped** — `SKIP_FRONTEND_BUILD=1` | `crates/product/ironclaw_webui/build.rs` honours this *(verified)* |

Skipping the WebUI is the difference between a 15-minute and a 60-minute build,
and it costs nothing here: the acceptance run drives the MCP tool surface, not a
browser. It removes the Node 22 / pnpm 11 dependency entirely.

**Do not copy the submodule's `Dockerfile` verbatim.** It is a Railway deployment
image: cargo-chef multi-stage, WebUI built inline, `openssh-server` installed,
`0.0.0.0` serve host. Most of that is irrelevant here. Copy its *conventions* —
pinned base digests, `SKIP_FRONTEND_BUILD`, the entrypoint shape — and build a
much smaller image.

**Runtime requirements, verified against `crates/domains/ironclaw_llm/src/config.rs`
and `.env.example`:**

| Variable | Required | Notes |
|---|---|---|
| `LLM_BACKEND` | yes | see §7 |
| `MINIMAX_API_KEY` | yes | secret, injected at run time — **never baked** |
| `MINIMAX_MODEL` | yes | `MiniMax-M3.1-Flash-Preview` |
| `MINIMAX_BASE_URL` | yes | `https://api.minimax.io/v1` (global); `https://api.minimaxi.com/v1` for China |
| `DATABASE_URL` | yes | `postgres://ironclaw:ironclaw@db:5432/ironclaw` |
| `IRONCLAW_REBORN_SECRET_MASTER_KEY` | yes | no OS keychain in a container |
| `IRONCLAW_REBORN_SERVE_HOST` | yes | `0.0.0.0` |
| `IRONCLAW_REBORN_HOME` | yes | `/var/lib/ironclaw`, on a volume |
| `SKIP_FRONTEND_BUILD` | build-time | `1` |

A `postgres` service is required — the submodule's own `docker-compose.yml` uses
`pgvector/pgvector:pg16` and says in its own header *"Local development only —
do NOT use these credentials in production."* Treat that as authoritative.

**PID 1 and signals.** `ironclaw serve` is long-running and the acceptance run
will need to stop it. Use `tini` and an exec-form `ENTRYPOINT`. The same applies
to `ironmaintd`, which installs `SIGTERM` and `SIGINT` handlers and drains on
both *(verified, `bins/ironmaintd/src/main.rs:369-401`)* — a shell wrapper that
does not forward signals loses graceful shutdown, and the second daemon on the
same state dir will then fail to acquire the lock.

### 4.6 `ironmaint/fake-services:0.1` — Phase 1, specified not built

Local stand-ins for the privileged services: Debian BTS, Koji, Bodhi, Red Hat
Bugzilla.

**Why deferred even though it is specified.** §4.10 keeps privileged external
effects unavailable in 0B, and **nothing in the tree produces a
`PrivilegedOperation` yet**. The producer is the open half of **D-09** — "who may
mark a job `InfrastructureBlocked`, and on what evidence" — which is a Phase 1
*decision* that has not been made. Building fakes now means building a test
surface for a path no code reaches, against an API shape nobody has agreed on.

**When it is built, three constraints:**

1. **The endpoints come from `IssueCapability::provider_id` and
   `ReleaseCapability::publication_plan`**, which the stubs already emit
   *(verified)*: `debian-bts` / `redhat-bugzilla`, and
   `CanonicalRepositoryPush` + `IssueTrackerMutation` (Debian) /
   `RemoteBuildSubmission` + `DistributionUpdateCreation` (Fedora). The fakes
   cover exactly those, and nothing else.

2. **The fakes must fail in the same shape as the real services**, including
   authentication failure, rate limiting and partial failure. A fake that always
   returns 200 makes the privileged path look far more reliable than it is, which
   is worse than not having it.

3. **Signing stays out of scope entirely.** §30 forbids exposing signing
   credentials to the process environment, and no concrete `gpg`/`debsign`
   binary is named anywhere in the three spec documents. Do not invent one.

---

## 5. Composition — how these are actually run

### 5.1 §97, the reproducible way

```sh
docker run --rm -v "$PWD":/w -w /w -v ironmaint-target:/w/target \
  ironmaint/workspace:0.1 sh -c '
    cargo fmt --check &&
    cargo clippy --workspace --all-targets --all-features -- -D warnings &&
    cargo test --workspace &&
    cargo test --workspace --features integration &&
    cargo run -p xtask -- verify-architecture &&
    cargo run -p xtask -- verify-schemas &&
    cargo run -p xtask -- verify-migrations &&
    cargo run -p xtask -- verify-mcp-schemas'
```

A named volume for `target/` is what makes iteration bearable. A bind mount would
recompile from scratch on every run and defeat the point.

### 5.2 The item-30 run

```text
┌──────────────────┐      ┌──────────────────┐      ┌────────────┐
│ agent (ironclaw) │─────▶│ ironmaintd       │─────▶│ postgres   │
│ + SKILL.md       │ MCP  │ :7341 /mcp       │ libsql│            │
└──────────────────┘      └──────────────────┘      └────────────┘
                                   │
                                   ▼
                     debian-tools / fedora-tools
```

Three containers, one network, shared volumes for state and the workspace root.

**`mcp-registration.yaml` cannot be used as-is.** See §6. This is not a small
problem and it is not optional.

### 5.3 Signals and PID 1

Every long-running container in this set — `ironmaintd`, `ironclaw serve`,
`postgres` — installs signal handlers and expects to be PID 1. `tini` everywhere,
exec-form `ENTRYPOINT`, and a test that actually sends `SIGTERM` and asserts a
clean exit. `bins/ironmaintd/tests/startup_shutdown.rs` already does exactly this
*(verified)*; the image should run that test rather than assert the property a
second time in a shell script.

---

## 6. The manifest is the wrong format — D-06's real answer

**`doc/operator/mcp-registration.yaml` is not a valid IronClaw extension
manifest.** This was checked directly against the submodule's own reference
manifests, and it is a finding, not an observation.

Our file is YAML, declares `api_version:`, and nests under `metadata:` and
`extensions:`. The real schema is **TOML**, declares **`schema_version =`**, and is
flat at the top (`id`, `name`, `version`, `description`, `trust`) with `[runtime]`,
`[mcp]`, `[[tools]]`, `[auth.<vendor>]` and `[admin_configuration]` sections
*(verified — `doc/vendor/ironclaw/crates/extensions/ironclaw_extension_registry/src/v3.rs:55,106-136,223-236`,
against `crates/extensions/packages/slack/manifest.toml`)*.

**It will not degrade gracefully.** `RawManifestV3` is `#[serde(deny_unknown_fields)]`,
so `ironclaw extension install` fails on the first unexpected key. It is a parse
error, not a warning.

Specific divergences the implementing team must resolve:

| Our YAML | IronClaw v3 | Consequence |
|---|---|---|
| `api_version:` | `schema_version =` | rejected |
| `metadata.name` | `id` (plus `name` for display) | rejected |
| `extensions: [{...}]` array | flat + `[mcp]` | rejected |
| `extensions[0].endpoint` | `[mcp] server` (HTTPS) | rejected |
| `transport: streamable_http` | *not in IronClaw's vocabulary* — MCP is JSON-RPC 2.0 over HTTPS | must be dropped |
| `auth.kind: bearer`, `bearer_token_file` | per-tool `[[tools.credentials]]` with `handle` + `vendor` + `audience` | no direct equivalent |
| `egress.allowlist: [...]` | per-tool `network_targets` | no direct equivalent |
| `healthcheck: {method, timeout_ms}` | not a manifest field — the host owns probes | must be dropped |

**The spec should require a translator**, not a hand-edit: our manifest and the
IronClaw one are generated from the same source of truth (the daemon's own
`tools/list`), and the moment they are hand-maintained separately they will
diverge. The tool count has already been wrong four times across 0B.10 — the
lesson recorded in C3's notes is *grep for the number, not the symbol*.

**Definition of done for this item:** `ironclaw extension install` succeeds
against the real binary, and the extension advertises the same tool set the
daemon does. Until then D-06 stays open and item 30 stays blocked regardless of
how good the images are.

---

## 7. LLM configuration

Per the architect's direction, the agent image targets **MiniMax** with model
**`MiniMax-M3.1-Flash-Preview`**, chosen because it is available behind both an
OpenAI-compatible and an Anthropic-compatible interface.

### 7.1 Two wirings, one of which is a fallback

**Primary — the native `minimax` backend:**

```sh
LLM_BACKEND=minimax
MINIMAX_API_KEY=<secret, injected>
MINIMAX_MODEL=MiniMax-M3.1-Flash-Preview
MINIMAX_BASE_URL=https://api.minimax.io/v1
```

Caveat: `minimax` is **not** a variant of `LlmBackendKind`; it falls through
`from_backend_id` to `Registry(String)` *(verified,
`crates/domains/ironclaw_llm/src/config.rs:408-415`)*. It is therefore a
**registry provider**, configured through `providers.json` written by
`ironclaw onboard`, not purely by environment variable. Budget time for
producing that file correctly.

**Fallback — the OpenAI-compatible path**, which is what the other providers in
`.env.example` use and which needs no registry configuration:

```sh
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.minimax.io/v1
LLM_API_KEY=<same secret>
LLM_MODEL=MiniMax-M3.1-Flash-Preview
```

**Try the fallback first.** It is strictly less configuration, and if the native
path is preferred it should be because the fallback was measured to fail, not
because the fallback was never tried.

### 7.2 Two hazards the implementing team must not hit

**The model is not in IronClaw's reasoning registry.**
`NATIVE_THINKING_PATTERNS` contains `"minimax-m2"` and nothing else
*(verified, `crates/domains/ironclaw_llm/src/reasoning_models.rs:54-68`)*.
`MiniMax-M3.1-Flash-Preview` lowercased does not contain that substring, so it
will not be detected as a native-thinking model. Whether M3.1 needs the
`<think>` handling is **unknown and must be measured** — a wrong answer here
produces silent garbage, not an error.

`doc/vendor/ironclaw/` is a **read-only reference submodule** (`CLAUDE.md`: "do
not edit it from this repo"). So the fix is *not* to patch the pattern list here.
It is either an upstream contribution to `nearai/ironclaw`, or a workaround at
the configuration level. Decide which, and write down which, before the
acceptance run — do not discover it mid-run.

**Access is limited.** MiniMax's own documentation states
`MiniMax-M3.1-Flash-Preview` is *"available only through M Plan and MiniMax Code
for now."* Whoever runs the acceptance run needs the right account, and the
image's README should say so. An image that only works on one subscription tier
is a finding, not a bug.

### 7.3 Cost and determinism

A live agent run is non-deterministic and costs tokens. The spec should require
that the acceptance run record its transcript, so a failure is diagnosable after
the fact rather than requiring a re-run. The submodule has
`tests/fixtures/llm_traces`, which is the natural place for a recorded mode — but
**a recorded-trace mode does not satisfy §102 item 30**, because item 30 asks
whether *an actual IronClaw agent* can complete the workflow. A replay proves the
harness works. Keep both, label them honestly.

---

## 8. Platform reality: arm64

The development host is **aarch64 macOS with Docker's `aarch64` engine** *(verified)*.
`buildx` lists `linux/amd64`, `linux/arm64`, `linux/ppc64le`, `linux/s390x` — the
non-native ones run under emulation, which is slow enough to make an `amd64`
image unusable for iteration.

**Everything here is arm64-native except possibly two things, and neither was
verified:**

1. **Fedora `mock` buildroots on `aarch64`.** Mock's configuration packages are
   `noarch`, which is a good sign, but whether a Fedora 42 aarch64 buildroot is
   fully supported is a real open question. If it is not, the fallback is
   building the `fedora-tools` image for `linux/amd64` and running it under
   emulation — slow, but workable for a build-once-and-cache case.

2. **`sbuild` on `arm64`.** Debian's arm64 is a release architecture, so this is
   likely fine, but schroot creation and `sbuild-debootstrap` are the part to
   smoke-test first.

**This is the largest schedule risk in the document and it should be resolved in
the first day of the work, before any Dockerfile is written.** A five-minute
`docker run --rm fedora:42 dnf install -y mock && mock -r fedora-42 --repos ...`
settles it, and finding out on day nine would be expensive.

**Whatever the answer, the images must declare their platform explicitly.**
A multi-arch manifest built by emulation is not the same artifact as a native
one, and "it built" hides which it was.

---

## 9. Definition of done

Each item is checkable by a person who did not do the work.

**Per image**

- [ ] Builds reproducibly from a clean checkout, with a pinned base digest
- [ ] Builds natively on `linux/arm64`; the platform is declared in the manifest
- [ ] Has a README stating what it is for, what it is not for, and its known limits
- [ ] Runs as a non-root user
- [ ] `docker inspect` shows no baked secret. `MINIMAX_API_KEY` is injected at
      run time and must never appear in a layer.

**`workspace`**

- [ ] `rustc --version` reports exactly `1.88.0`
- [ ] The full §97 set passes inside the image
- [ ] A cold run with a warm `target/` volume completes in a stated, measured time

**`debian-tools`**

- [ ] `sbuild`, `lintian`, `autopkgtest`, `dpkg-buildpackage`, `gbp` all present
      and on `PATH`
- [ ] **A real `sbuild` run completes against a real source package**, creating
      its schroot into a mounted volume. Not a `--version` check.
- [ ] The sandbox requirement is documented (`--privileged` vs namespaces) with
      the security tradeoff stated

**`fedora-tools`**

- [ ] `mock`, `rpmlint`, `tmt`, `rpmbuild` present
- [ ] **`mock -r` creates a buildroot and one trivial build succeeds.** This is
      the single most important check in the image set.
- [ ] The §8 arm64 question is answered in writing, with the evidence

**`agent`**

- [ ] Builds with `SKIP_FRONTEND_BUILD=1` and no Node.js in the final image
- [ ] `ironclaw --version` runs
- [ ] `ironclaw extension install` accepts a **correctly formatted** manifest
      (§6) and `ironclaw extension activate` succeeds
- [ ] The agent reaches `ironmaintd` over HTTP and completes a `job.create` →
      `job.get` round trip
- [ ] The §7.2 reasoning-registry question is answered in writing
- [ ] `SIGTERM` produces a clean exit

**The set**

- [ ] `scripts/ironclaw-e2e.sh` runs **inside** the images, unmodified or with
      changes justified in the commit
- [ ] A run of the full §101 repair loop by a live agent is recorded, and
      `doc/DEBT.md` **D-15** is updated with the result
- [ ] `doc/PHASE-0B-COMPLETION.md` §20 row 30 is updated to reflect reality

---

## 10. Explicit non-goals

Stated so the work is not judged against goals it was never meant to hit:

- **A per-tool container sandbox.** §1.3. This is a phase-sized change to §33,
  not a Dockerfile.
- **A CI pipeline.** The absence of CI is a real hole and this does not close it
  — but "add CI" is a different decision with a different owner. Say so; do not
  smuggle it in.
  - **Superseded 2026-10-03.** The separate decision was taken, and the owner
    asked for it explicitly. `.github/workflows/ci.yml` runs the §97 gate and
    builds and smokes the six images on `ubuntu-24.04-arm`. The line above is
    kept as written because it was right when written: CI was a decision, not a
    Dockerfile, and nothing in this image set should have implied otherwise.
    What the decision does **not** cover is listed under §12, because a CI
    pipeline that exists is not the same as a CI pipeline that runs everything
    §9 asks for.
- **Real privileged side effects.** No signing keys, no upload credentials, no
  real BTS/Koji/Bodhi writes. §4.10 and §30 both forbid it.
- **Making the synthetic tools obsolete.** They are §92's exit checkpoint and the
  daemon registers them verbatim. Real toolchains are *additional* capability, not
  a replacement, and `ScenarioAdapter` stays in the testkit.
- **Supporting both amd64 and arm64 as first-class targets.** Pick native arm64
  and, if `mock` requires it, add amd64 as a documented slow path.

---

## 11. Risks to verify before building

### 11.1 The toolchain pin is a trap

`rust-toolchain.toml` says 1.88.0; the image must say 1.88.0. But the submodule's
`AGENTS.md` notes that **its** Dockerfile deliberately excludes `rust-toolchain.toml`
from the build context so the image uses the base image's toolchain *(verified)*.
That is a reasonable pattern for IronClaw and the **wrong** one for
`ironmaint/workspace`, where the pin is the entire point. Do not copy that
convention across without deciding which rule applies.

### 11.2 `ca-certificates` and `native-roots`

Already covered in §4.1, and repeated here because it is the failure mode most
likely to be misdiagnosed as "the network is broken".

### 11.3 Distribution release mismatch

The Debian fixtures use `sid`; the proposed base is `bookworm`. The Fedora
fixtures use whatever `DistributionRelease` the stub declares. Whether an image
should track the fixture's release or a stable release is a real decision, not a
detail — an adapter that plans `debian/sid` and builds on `bookworm` will produce
package-version comparisons that mean nothing.

### 11.4 Image staleness

Fedora moves every six months. A pinned digest is reproducible and rots. The
spec should require a scheduled bump with a smoke test, or the images become a
snapshot of October 2026 that nobody trusts in March.

### 11.5 The manifest is not optional

§6. Without it, the image set produces a running agent that cannot register
IronMaint, and item 30 stays blocked for a reason that has nothing to do with the
images.

---

## 12. Where this lives in the repository

A proposal, for the implementing team to accept or change:

```text
images/
  base/          Dockerfile
  workspace/     Dockerfile
  debian-tools/  Dockerfile
  fedora-tools/  Dockerfile
  agent/         Dockerfile
  fake-services/ Dockerfile          # Phase 1
  smoke/         *.sh                 # the §9 checks, one per image
  README.md                         # the set, and how to run §5
```

The `smoke/` scripts should be runnable on a developer machine and in CI
alike, and each should be a *real* check — a `sbuild` build, a `mock` buildroot,
an `ironclaw --version` — because that is the lesson of D-14 written down as
infrastructure: **a test that cannot fail is not a test.**

### 12.1 What CI does, and what it does not

Added 2026-10-03 alongside the §10 supersession. A CI pipeline existing is not
the same as a CI pipeline running everything §9 asks for, and the difference is
worth writing down rather than discovering from an unticked box.

**What runs:** the §97 gate, and `make -C containers verify` — all six images
built and all six smoke tests executed, on `ubuntu-24.04-arm`.

**What does not, and why:**

- **The `mock` buildroot and the real `sbuild` build.** §9 asks for both and
  means it — "not a `--version` check". Both need `--privileged`, both are
  minutes-to-tens-of-minutes, and `STATUS.md`'s two day-one arm64 risks for
  `mock` and `sbuild` have still not been run. A runner that builds a mock
  buildroot is a different and much slower job, and putting it in this one
  would make the gate unreliable rather than fast. The §9 boxes stay unticked.
- **Multi-arch.** Native arm64 only. §8 treats emulated builds as unusable for
  iteration and §10 rules out amd64 as a first-class target, so `build-multi` and
  its local-registry path stay a developer-machine concern.
- **Publishing.** The image job builds and smokes. Nothing is pushed and
  nothing is tagged; a registry and a release policy are a separate decision on
  the same grounds §10 gave for CI itself.
- **Branch protection.** The gate is not yet required on `develop`. Making it
  required changes what every other contributor can do, which is the same class
  of change this section's parent non-goal was protecting.

**One deviation from §4.2, on purpose.** §4.2 wants
`cargo build --workspace --all-targets` baked into the workspace image so a cold
CI run is a link step. That is not what this CI does, and it is worth saying why
rather than leaving the Dockerfile's DEFERRED block to imply it is still coming:
a baked `target/` is a multi-GB layer that is invalidated by every source
change, which is to say it is stale exactly when CI matters most. A
`target/`-keyed CI cache gives the same "link, not compile" behaviour on the
second run, invalidates correctly, and costs no image size. The block stays in
the Dockerfile with its reasoning updated, because the spec's instinct was
right and its mechanism was sized for a world where CI is one cold run.
