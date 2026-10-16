# Phase 2.0-C — completion

> PHASE-2 PR 2.0-C (lock the build execution model,
> `doc/PHASE-2-C.md`). This document records what
> 2.0-C delivered and what it deliberately did not.
> The 2A sub-phases (2A.1–2A.4) build on these
> decisions without re-deciding them.

## What 2.0-C delivered

1. **`BuildTarget` struct + `BuildTargetProvider` trait**
   in `crates/ironmaint-adapter-api/src/build_target.rs`.
   The `default_for_candidate(ctx)` constructor
   returns `(suite="sid", arch=host_arch())` for
   the first slice; the `parse(suite, arch)`
   constructor validates operator input. The
   `host_arch()` helper maps `std::env::consts::ARCH`
   to the Debian name (`aarch64 → arm64`,
   `x86_64 → amd64`, etc.).

2. **Three adapter impls** of `BuildTargetProvider`:
   the production `DebianAdapter`, the
   `DebianStubAdapter` (returns the same default
   as the real adapter), and the `FedoraStubAdapter`
   (returns `rawhide` + host arch — the Fedora
   moving-target suite). The conformance integration
   test
   `crates/ironmaint-adapter-api/tests/build_target_provider_required.rs`
   pins both.

3. **`RuntimeConfig` gains two build-execution
   knobs**: `sbuild_buildroot_dir: PathBuf`
   (default `/var/lib/sbuild`, override flag
   `--sbuild-buildroot-dir <path>`, env-var
   fallback `IRONMAINT_SBUILD_BUILDROOT_DIR`)
   and `sbuild_unprivileged: bool` (default `true`,
   override flags `--sbuild-unprivileged` /
   `--no-sbuild-unprivileged`, env-var fallback
   `IRONMAINT_SBUILD_UNPRIVILEGED`). The
   precedence — flag beats env var beats
   default — matches the existing runtime flags.
   Five new tests in
   `bins/ironmaintd/tests/config.rs` pin the
   defaults and the overrides; the
   `usage_names_every_flag_the_parser_accepts`
   test was updated to include the three new
   flag spellings.

4. **Startup log line** in
   `bins/ironmaintd/src/main.rs` records the
   resolved build-execution mode: the buildroot
   dir, the sandbox mode (`unprivileged` /
   `schroot`), and the default build target
   (`sid/<host-arch>`). The line lets the
   operator confirm the compose service's
   mounts and the daemon's expectations agree.

5. **Compose service** `debian-tools` added to
   `containers/compose.yml`, gated by the
   `runtime` profile so the §97 gate (which
   only knows the `workspace` profile) is
   unaffected. The service runs `ironmaintd`
   with the new flags; the named volume
   `ironmaint_sbuild-buildroots` is mounted at
   `/var/lib/sbuild` inside the service.

6. **Image label** for `ironmaint/debian-tools:0.1`
   updated to record the unprivileged default
   and the schroot fallback. (Was: "Requires
   `--privileged` or `SYS_ADMIN`+user namespaces
   for the buildroot." Now: "Default sandbox mode
   is unprivileged user namespaces
   (`sbuild --unshare`); pass
   `--no-sbuild-unprivileged` on ironmaintd to
   fall back to the schroot backend, which
   requires the container to run with
   `--privileged`.")

7. **Smoke tests** for the unprivileged mode,
   the buildroot dir writability, and
   `debootstrap --version` added to
   `containers/debian-tools/smoke.sh`. The
   new assertions run only inside the image
   via `make -f containers/Makefile
   smoke-debian-tools`; they are not part of
   the §97 gate.

8. **Spec doc** `doc/PHASE-2-C.md` answers the
   five §2.0-C questions (where ironmaintd
   runs, where sbuild buildroots live, which
   (suite, arch) is selected, how the build is
   isolated from agent credentials, and whether
   unprivileged user namespaces are sufficient)
   with the concrete decisions documented.

## What 2.0-C deliberately did NOT do

- A real `sbuild` invocation. (PR 2A.3.)
- A `BuildCapability` impl in the real adapter
  with a real `BuildPlan`. (PR 2A.2.)
- A new MCP tool. (None needed — the build is
  invoked by the executor, not the MCP.)
- A `BuildPlan` that carries a `BuildTarget`
  field. (2A.2's call.)
- The Fedora parallel. (Phase 2 follow-on;
  the Fedora stub's `rawhide` + host arch is
  the 2.0-C contract; the real Fedora
  adapter is a follow-on.)
- The `ironmaint-debian-tool` install recipe
  inside the `debian-tools` image (currently
  bind-mounted; 2A.3 will fix it).
- A new `AdapterCapability` or `EvidenceKind`
  or `GateStage`. (All exist; 2.0-C did not
  need new ones.)
- An automatic transition into
  `InfrastructureBlocked` (§17 Q8 / D-09 stays
  open per the spec).
- A change to the §97 gate. The new smoke
  additions are opt-in (`make
  smoke-debian-tools`).
- A change to `Cargo.lock` beyond what the
  workspace already has — no new transitive
  deps were needed (`std::env::consts::ARCH`
  is in std).

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
- **MCP boundary:** unchanged. The 15-tool
  count is unchanged; 2.0-C is a config /
  compose / doc / trait PR, not a tool PR.
- **IronClaw integration:** unchanged.

## §97 gate result

```text
cargo fmt --check                                                                clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings    clean
cargo run -p xtask --locked -- verify-architecture                               no violations
cargo run -p xtask --locked -- verify-schemas                                    no drift
cargo run -p xtask --locked -- verify-migrations                                 clean
cargo run -p xtask --locked -- verify-mcp-schemas                                no drift
cargo run -p xtask --locked -- verify-seams                                      no violations
cargo test --workspace --locked                                                  1107 pass / 0 fail / 1 ignored
cargo test --workspace --features integration --locked                           1116 pass / 0 fail / 1 ignored
```

The 1 ignored test is the same
`IRONMAINT_REAL_FIXTURE_REACHABLE`-gated 1.1.E
test that was ignored at 1.1.1 release. No new
ignored tests were added by 2.0-C.

## Next steps (per `doc/phases/PHASE-2.md` §§31–37)

- **2.0-D** — another readiness PR, this one
  about the build input staging directory
  (§4 of the doc) and the artifact-store
  integration.
- **2.0-E** — final readiness PR, about the
  reproducibility harness.
- **2A.1** — first real-build code, the
  `debian.build.sbuild` `ToolDefinition`.
- **2A.2** — `BuildCapability` impl with a real
  `BuildPlan`; carries the `BuildTarget`.
- **2A.3** — real `sbuild` invocation against
  a Debian source package fixture.
- **2A.4** — second tool (`debian.qa.lintian`).

The 2A PRs are documented in
`doc/phases/PHASE-2.md` §42 (16-PR sequence).
