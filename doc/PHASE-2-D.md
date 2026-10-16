# Phase 2.0-D — make the candidate/build boundary explicit

> Pre-implementation readiness PR. Per
> `doc/phases/PHASE-2.md` §2.0-D: "Before coding the
> build path, make the candidate/build boundary
> explicit." No real `sbuild` invocation lands in
> 2.0-D; that is PR 2A.3. 2.0-D is a documentation +
> minimal type + test PR — the same shape as 2.0-C.

## Context

`doc/phases/PHASE-2.md` §2.0-D names five concerns
about how a captured `SourceCandidate` becomes the
input to a real build:

1. Stage the exact candidate tree into a disposable
   build input.
2. Define how a captured Git tree becomes valid
   Debian source-package input (3.0 (quilt): orig
   tarball + `.dsc`).
3. Keep generated build outputs outside the immutable
   candidate snapshot.
4. Bind build report to candidate fingerprint,
   commit, and tree.
5. The current `HEAD^{tree}` tree OID read is NOT a
   sufficient write detector for the candidate and
   must not be the write-detection mechanism after
   this PR.

Today's state (verified by exploration):

- `BuildPlan { checks: Vec<PlannedCheck> }` has no
  `input` field. A planned check is a *what to run*,
  not a *where to run it from*. The candidate → build
  input mapping is undefined.
- `WorkspaceManager::frozen_for_check(...)` returns
  a `FrozenWorktree` rooted at
  `<root>/<handle>/.ironmaint-checks/<worktree_id>`.
  It is the read-only view of the captured candidate.
  The build path needs a different sibling: not a
  check view, but a *staged source package* that the
  build tool (`sbuild`) can consume.
- `FrozenWorktree::tree_sha()` uses
  `git rev-parse HEAD^{tree}`. §2.0-D says this is
  NOT a sufficient build write detector — the
  candidate is at a path the build process can write
  to, and reading the tree OID after a possible write
  is racy.
- The artifact store is content-addressed by SHA-256
  only (`crates/ironmaint-artifacts/src/hash.rs`,
  `store.rs`). Build outputs (binary debs,
  `.buildinfo`, `.changes`, `lintian` reports) cross
  the container boundary as artifact-store digests,
  not as paths.
- The `pinned-real` fixture is a hand-crafted 3.0
  (quilt) source tree
  (`containers/fixtures/debian/pinned-real/` has
  `Makefile`, `hello.c`, `debian/`). Its `METADATA`
  records the upstream `hello_2.10.orig.tar.gz` URL
  and SHA-256, but the tarball is NOT committed
  today; the fixture exercises inspection, not build.

2.0-D closes none of the 2A work; it makes the
candidate → build input boundary explicit and
verifiable, so the 2A PRs do not re-decide it.

## Decisions (this PR)

### 1. Where the staged build input lives on disk

**Decision.** A new sibling of
`FrozenWorktree`, `BuildInputHandle`, is rooted at
`<root>/<handle>/build-input/<fingerprint>/`. The
handle is what the executor hands to `sbuild` (in
2A.3); the build tool sees the staged source tree at
`handle.root()`. The handle's `output_dir()` is a
sibling, `<root>/<handle>/build-output/<fingerprint>/`,
not a sub-directory of the staged root — so a build
tool that writes into the output area cannot
inadvertently modify the source tree.

**Where it lives in the tree.**
- `crates/ironmaint-workspace/src/build_input.rs` —
  the `BuildInputHandle` type.
- `crates/ironmaint-workspace/src/manager.rs` — the
  `stage_build_input(...)` method.

**What to change to make the decision real.**
- `WorkspaceManager::stage_build_input(&self,
  job_id, descriptor) -> Result<BuildInputHandle,
  WorkspaceError>` is the API.
- The implementation: `git -C <workspace> archive
  <commit> | tar -xC <staged>`; place the upstream
  tarball at `<staged>/<upstream_filename>`; re-derive
  the tree OID at the staged workdir via `git init
  -q && git add -A && git write-tree` and assert
  equality with `descriptor.candidate.tree`.
- `chmod -R a-w` the staged root after staging so the
  build tool cannot modify the source.

**Regression test.**
- `crates/ironmaint-workspace/tests/build_input.rs`
  drives the staging end-to-end against the
  `pinned-real` fixture and asserts all of: the
  staged tree OID equals the candidate's tree OID;
  the orig tarball is present at the expected path;
  the staged root is not writable; a `Drop` of the
  handle leaves no garbage in the workspace dir.

### 2. How a captured Git tree becomes valid Debian source-package input

**Decision.** For 2.0-D, the only supported source
format is **3.0 (quilt)**. A
`BuildInputDescriptor::quilt(candidate, upstream_tarball)`
constructor is the entry point. The `upstream_tarball`
field carries a path and a SHA-256; the staging
primitive validates the SHA-256 by hashing the file
before placing it. The staged root then contains
exactly the files `dpkg-source -b` expects for
3.0 (quilt): the unpacked source tree
(`Makefile`, `hello.c`, `debian/`, etc.) and the
upstream orig tarball (`hello_2.10.orig.tar.gz`).

Native and 1.0 formats are deferred. The
`BuildInputError::UnsupportedSourceFormat` variant is
reserved for a future PR that adds them.

**Where it lives in the tree.**
- `crates/ironmaint-adapter-api/src/build_input.rs` —
  `BuildInputDescriptor`, `SourceFormatQuilt`,
  `UpstreamTarball`, `BuildInputError`.

**What to change to make the decision real.**
- The descriptor's SHA-256 is validated at
  `stage_build_input` time, not at
  `BuildInputDescriptor::quilt(...)` time. The
  constructor only stores the expected value; the
  stage primitive compares against the on-disk
  hash. This keeps the constructor cheap and
  side-effect-free (so it can be used in a
  conformance test in isolation).
- The `containers/fixtures/debian/pinned-real/`
  fixture's `METADATA` records the upstream
  tarball's filename and SHA-256. The committed
  `hello_2.10.orig.tar.gz` is the substrate the
  test exercises.

**Regression test.**
- `crates/ironmaint-workspace/tests/build_input.rs`
  asserts: the orig tarball is present at
  `<staged>/hello_2.10.orig.tar.gz` after staging;
  its SHA-256 matches the value the fixture's
  `METADATA` records; if the descriptor's
  `upstream_tarball.sha256` is wrong, the stage
  returns `WorkspaceError::Other("upstream tarball
  SHA-256 mismatch")`.
- `crates/ironmaint-adapter-api/tests/build_input_descriptor.rs`
  pins the descriptor shape (struct fields,
  constructor).

### 3. How generated build outputs stay outside the candidate snapshot

**Decision.** The `BuildInputHandle::output_dir()`
returns a path **sibling to the staged root**, not a
sub-directory. The runtime mounts the staged root
read-only (via `chmod -R a-w` for the 2.0-D slice; a
bind-mount is 2A.3's hardening). The build tool's
working directory at the OS level is
`<output_dir>/<job-id>/<check-id>/` — the
executor sets the CWD via `ProcessExecutor`'s
`with_env` env override, and the build tool is
expected to write into the CWD.

**Where it lives in the tree.**
- `BuildInputHandle::output_dir()` returns
  `<root>/<handle>/build-output/<fingerprint>/`.
- The `ProcessExecutor` env override for the
  `debian.build.sbuild` tool (in 2A.1) is
  `BUILD_OUTPUT_DIR=<output_dir>`. 2.0-D only
  ships the path; 2A.1 wires the env.

**What to change to make the decision real.**
- The staged root is `chmod -R a-w`'d at stage
  time. A test asserts `metadata().permissions().readonly()`
  on a sample file under the staged root.
- The output dir is created at stage time (empty
  subdir) so the executor does not race with
  creation.

**Regression test.**
- The integration test asserts: `output_dir()` is
  a sibling of `root()`, not a sub-directory;
  writing a sentinel file into the staged root
  returns `EACCES`; writing a sentinel file into
  the output dir succeeds.

### 4. How the build report binds to the candidate fingerprint, commit, and tree

**Decision.** `BuildInputHandle` exposes
`tree_sha()`, `commit()`, and the descriptor's
candidate fingerprint. The future `sbuild` tool
(in 2A.1) carries these into the `BuildReport` it
emits. The fingerprint is the BLAKE3 hex stored in
`SourceCandidate::fingerprint`; the commit and
tree are the SHA-1s.

The artifact store is **not** keyed by candidate —
it is SHA-256 only. The candidate's BLAKE3
fingerprint lives in evidence metadata, not in the
CAS. The binding candidate → artifact is the
`Evidence → ArtifactRef → Digest` path that
already exists; 2.0-D does not add a new binding
mechanism, only ensures the build tool can carry
all three OIDs into the report.

**Where it lives in the tree.**
- `BuildInputHandle` carries the three OIDs
  (re-derived `tree_sha`, `commit`, candidate
  fingerprint).
- `crates/ironmaint-artifacts/src/hash.rs` — a
  doc comment notes: "Build outputs are addressed
  by SHA-256 only; the candidate fingerprint
  (BLAKE3) lives in evidence metadata, not as a CAS
  key."

**What to change to make the decision real.**
- `BuildInputHandle` exposes a `candidate()`
  accessor returning `&SourceCandidate` so the
  caller has the full identifier set.
- A test asserts the three OIDs are exposed and
  non-empty.

**Regression test.**
- The integration test asserts:
  `handle.candidate().fingerprint() ==
  descriptor.candidate.fingerprint()`;
  `handle.commit() == descriptor.candidate.commit()`;
  `handle.tree_sha() == descriptor.candidate.tree()`.

### 5. Why `HEAD^{tree}` is not a sufficient build write detector

**Decision.** 2.0-D does NOT remove
`FrozenWorktree::tree_sha()` — it is the *check
path's* tree OID reader, used at capture time when
the workspace is necessarily read-only. The
replacement for the build write detector is
**directory separation**: the candidate is on a
read-only path (`chmod -R a-w`), the build input is
on a separate writable path, and the build input's
tree OID is verified by re-deriving it via `git
write-tree` *before* the build starts, not by
reading `HEAD^{tree}` *after* the build runs.

The race is structural: a `HEAD^{tree}` read after a
build is racy because the build may have written to
the path and the read returns the new tree, not
the candidate's tree. The re-derivation at stage
time, before any build, is the structural
guarantee. The test `chmod -R a-w` is the
write-prevention mechanism.

**Where it lives in the tree.**
- The staging primitive chmods the staged root at
  the end of `stage_build_input`, *after* the
  `git write-tree` re-derivation succeeds.
- `BuildInputHandle` exposes a
  `is_staged_root_readonly()` accessor that returns
  `true` so the executor can assert it before
  running the build tool (defence in depth — the
  2A.1 PR uses this as a precondition check).

**What to change to make the decision real.**
- The chmod is part of the GREEN implementation;
  the RED commit leaves the staging primitive as a
  stub that returns
  `WorkspaceError::Other("stage_build_input: not
  yet implemented in 2.0-D red commit")`. The
  stub's return type is the eventual
  `Result<BuildInputHandle, WorkspaceError>` so
  the GREEN commit can replace the body without
  touching the type signature.

**Regression test.**
- The integration test asserts:
  `handle.is_staged_root_readonly()` is `true`;
  attempting to `create_file` inside the staged
  root returns `EACCES`.

## Out of scope (what 2.0-D is NOT)

- A real `sbuild` invocation. (PR 2A.3.)
- A `BuildCapability` impl in the real adapter.
  (PR 2A.2.)
- A `BuildPlan` that carries a `BuildInputDescriptor`
  field. (2A.2's call — the side-table pattern
  keeps them decoupled in 2.0-D so 2A.2 can wire
  them in.)
- A new MCP tool. (None needed — the staging is
  invoked by the executor, not the MCP.)
- A read-only bind-mount of the candidate.
  (`chmod -R a-w` is the 2.0-D mechanism;
  bind-mounting the candidate read-only is 2A.3's
  hardening.)
- A `SourceFormat` enum covering native and 1.0.
  (`SourceFormatQuilt` is a struct in 2.0-D; the
  enum is 2A.2's call when the real adapter's
  `BuildCapability` lands.)
- A new `AdapterCapability` or `EvidenceKind` or
  `GateStage`. (All exist; 2.0-D does not need new
  ones.)
- The Fedora parallel. (Phase 2 follow-on.)
- An artifact-store CAS keyed by BLAKE3. (The
  artifact store is SHA-256 only; 2.0-D's doc
  states this explicitly so 2A.2 doesn't re-debate
  it.)
- A change to the `pinned-real` fixture's hermetic
  semantics beyond adding the committed orig
  tarball. (The committed tarball is the only
  change; the rest of the fixture is unchanged
  from 1.1.E.)
- A change to `FrozenWorktree`. (2.0-D adds a
  sibling, not a replacement; `FrozenWorktree`
  remains the read-only check view.)
- A change to `FrozenWorktree::tree_sha()`. (Still
  the read-only check path's tree OID reader; the
  doc explains why it stays.)
- A change to the §97 gate. (The new
  `build_input.rs` tests run in the existing
  `cargo test --workspace` set; the `pinned-real`
  test is still opt-in via
  `IRONMAINT_REAL_FIXTURE_REACHABLE=1`.)

## §106 invariants preserved

- **State machine:** unchanged.
- **Persistence:** unchanged.
- **Executor:** unchanged (`ToolDefinition::with_env`
  shape is the same as 1.1.I left it; 2.0-D adds
  no new env fields).
- **Candidate identity:** unchanged (BLAKE3
  fingerprint over the versioned byte
  representation; `SourceCandidate` immutability
  rule from §30 is preserved).
- **Evidence:** unchanged
  (`Evidence.candidate: CandidateFingerprint` is
  the §30 binding; 2.0-D does not add a new
  `EvidenceKind`).
- **State ownership:** unchanged (adapters still
  produce plans; runtime still decides; no state
  machine mutation from the build path).
- **MCP boundary:** unchanged. The 15-tool count
  is unchanged; 2.0-D is a workspace + adapter-api
  + doc + test PR, not a tool PR.
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
cargo test --workspace --features integration --locked                           1107+ pass / 0 fail / 1 ignored
```

The new `build_input.rs` tests and the
conformance test in
`ironmaint-adapter-api/tests/build_input_descriptor.rs`
are part of the workspace test set. The `pinned-real`
fixture remains opt-in (gated on
`IRONMAINT_REAL_FIXTURE_REACHABLE=1`); 2.0-D does
NOT change that.

## Next steps (per `doc/phases/PHASE-2.md`)

- **2.0-E** — final readiness PR, about the
  reproducibility harness (the `reprotest` /
  `diffoscope` claim) and the `plan_to_materialised`
  capability-vs-runtime reconciliation check.
- **2A.1** — first real-build code, the
  `debian.build.sbuild` `ToolDefinition`. Consumes
  `BuildInputHandle` from 2.0-D.
- **2A.2** — `BuildCapability` impl with a real
  `BuildPlan`; carries the `BuildTarget` (2.0-C)
  and the `BuildInputDescriptor` (2.0-D).
- **2A.3** — real `sbuild` invocation against the
  pinned-real fixture. Hardens the read-only
  mount and the actual `dpkg-source -b` step.
- **2A.4** — second tool (`debian.qa.lintian`).
