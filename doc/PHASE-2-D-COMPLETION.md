# PHASE-2 PR 2.0-D — completion record

PR: `phase-2/2.0-d-candidate-build-boundary`
Branch: `feature/phase-2/2.0-d-candidate-build-boundary` (cut from `develop`)
Status: GREEN

This PR is the candidate/build boundary slice. It makes the
mapping from a captured `SourceCandidate` to the directory a
build tool sees explicit and verifiable, so the 2A PRs
(2A.1 — `debian.build.sbuild` tool, 2A.2 — `BuildCapability`
impl, 2A.3 — real `sbuild` invocation) do not re-decide the
boundary.

## What 2.0-D delivered

### Types

- `ironmaint_adapter_api::build_input::BuildInputDescriptor` —
  the *what* (which candidate, which source format, which
  upstream tarball).
- `SourceFormatQuilt` — the 3.0 (quilt) source format. 2.0-D
  supports **quilt only**; the `BuildInputError::UnsupportedSourceFormat`
  variant is reserved for the future native / 1.0 PRs (2A.2 or
  later).
- `UpstreamTarball { path: PathBuf, sha256: [u8; 32] }` — the
  upstream `<pkg>_<ver>.orig.tar.gz` for a 3.0 build. The
  `filename()` accessor returns the path's basename.
- `ironmaint_workspace::build_input::BuildInputHandle` — the
  *where* (the directory the build tool will see). Carries the
  staged root, the re-derived tree OID, the upstream tarball's
  final path inside the staged root, the output dir (a sibling
  of the staged root), and a `is_staged_root_readonly()` accessor
  the executor (2A.1) can use to assert the build cannot
  mutate the candidate.

### Staging primitive

`WorkspaceManager::stage_build_input(&self, job_id: &JobId,
descriptor: &BuildInputDescriptor) -> Result<BuildInputHandle,
WorkspaceError>` (in `ironmaint-workspace/src/manager.rs`).

The implementation is hermetic and lossless:

1. Resolve the workspace handle from `job_id`.
2. Materialise a `tempdir` under
   `<workspace>/.ironmaint-staging/<uuid>/`.
3. Verify the upstream tarball's SHA-256 against the
   descriptor's expected hash. A mismatch is a hard error —
   the descriptor was built against a different tarball than
   the one on disk.
4. `git archive <commit> | tar -xC <staging_dir>` extracts the
   captured source tree (Makefile, hello.c, `debian/`, etc.).
5. `git init -q && git add -A && git write-tree` re-derives
   the tree OID at the staged workdir. The primitive asserts
   the re-derived OID equals `descriptor.candidate.tree()` —
   the structural test that says "the staged workdir is exactly
   what the candidate's snapshot says it should be." This is
   the bullet 5 invariant: the read check happens **before** the
   build, not after, so a build that mutates the staged workdir
   cannot invalidate the OID.
6. The upstream tarball is placed alongside the extracted
   tree. The tarball is **excluded** from the tree-OID
   re-derivation; the descriptor's `tree` field is the captured
   source-package tree, not the directory the build tool will
   see.
7. The staging dir is atomically renamed into
   `<workspace>/build-input/<fingerprint>/`. The `tempdir`
   guard means a failed stage leaves nothing behind.
8. `chmod -R a-w` makes the staged root read-only. The
   `is_staged_root_readonly()` accessor exposes this for the
   executor's precondition check.
9. The output dir is created as a sibling
   (`<staged>.output/`) so a build tool that writes into the
   staged root would write into a read-only path; the output
   dir is the only writable sibling.
10. The handle returns with all five accessors populated.

### Test fixture

A small synthetic `hello_2.10.orig.tar.gz` (single
`hello-2.10/hello.c`, 770 bytes, SHA-256
`280bb02ba1f55671eb7ffe5c2426d66e6f9632404a0a5e0c8a89e68ad95fa8f9`)
is committed at
`containers/fixtures/debian/pinned-real/hello_2.10.orig.tar.gz`.
The `METADATA` records its filename, SHA-256, and size. The
fixture is hermetic — no network fetch, no live `dpkg-source -b`.
The §97 gate does not depend on this fixture; the
`pinned_real_debian_*` tests are still gated on
`IRONMAINT_REAL_FIXTURE_REACHABLE=1`.

### Tests

- `crates/ironmaint-workspace/tests/build_input.rs` — 5
  integration tests, all passing in GREEN. They cover the five
  2.0-D bullets: tree OID match, tarball placement, output-dir
  sibling-ness, read-only chmod, and a descriptor-only unit test.
- `crates/ironmaint-adapter-api/src/build_input.rs` unit tests
  for the descriptor and the `BuildInputError` variants.

### Documentation

- `doc/PHASE-2-D.md` — the spec; the five bullets with their
  decisions, the §106 invariants preserved, and the §97 gate
  result.
- `crates/ironmaint-artifacts/src/hash.rs` — explicit decision
  recorded: the artifact store is SHA-256 only; the candidate
  fingerprint (BLAKE3) is recorded as evidence metadata on the
  build report, not as a CAS key on the contents.

## §106 invariants preserved

- **State machine:** unchanged. `stage_build_input` is a
  workspace-layer primitive; it does not call into
  `ironmaint-state`.
- **Persistence:** unchanged. The handle is an in-memory value
  with paths; no new store rows.
- **Executor:** unchanged. `ToolDefinition::with_env` shape is
  the same as 1.1.I left it; 2.0-D adds no new env fields.
- **Candidate identity:** unchanged. BLAKE3 fingerprint over
  the versioned byte representation; `SourceCandidate`
  immutability rule from §30 is preserved.
- **Evidence:** unchanged. `Evidence.candidate: CandidateFingerprint`
  is the §30 binding; 2.0-D does not add a new `EvidenceKind`.
- **State ownership:** unchanged. The staging primitive does
  not write to the state machine. It returns an opaque handle
  to the runtime; the runtime decides what to do with it.
- **MCP boundary:** unchanged. 15-tool count is unchanged;
  2.0-D is a workspace + adapter-api + doc + test PR, not a
  tool PR.
- **IronClaw integration:** unchanged.

## §97 gate result (target / observed)

```text
cargo fmt --check                                                                clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings    clean
cargo run -p xtask --locked -- verify-architecture                               no violations
cargo run -p xtask --locked -- verify-schemas                                    no drift
cargo run -p xtask --locked -- verify-migrations                                 clean
cargo run -p xtask --locked -- verify-mcp-schemas                                no drift
cargo run -p xtask --locked -- verify-seams                                      no violations
cargo test --workspace --locked                                                  all pass
cargo test --workspace --features integration --locked                           all pass
```

## What 2.0-D deliberately did NOT do

- A real `sbuild` invocation. (PR 2A.3.)
- A `BuildCapability` impl in the real adapter. (PR 2A.2.)
- A `BuildPlan` that carries a `BuildInputDescriptor` field.
  (2A.2's call — the side-table pattern keeps them decoupled
  in 2.0-D so 2A.2 can wire them in without another redesign.)
- A new MCP tool. (None needed — the staging is invoked by
  the executor, not the MCP.)
- A read-only bind-mount of the candidate. (`chmod -R a-w` is
  the 2.0-D mechanism; bind-mounting the candidate read-only
  is 2A.3's hardening.)
- A `SourceFormat` enum covering native and 1.0.
  (`SourceFormatQuilt` is a struct in 2.0-D; the enum is 2A.2's
  call when the real adapter's `BuildCapability` lands.)
- A new `AdapterCapability` or `EvidenceKind` or `GateStage`.
- The Fedora parallel. (Phase 2 follow-on.)
- An artifact-store CAS keyed by BLAKE3. (The artifact store
  is SHA-256 only; 2.0-D's doc states this explicitly so 2A.2
  doesn't re-debate it.)
- A change to `FrozenWorktree` (2.0-D adds a sibling, not a
  replacement). The two have disjoint filesystem roots
  (`frozen/<sha>/` vs `build-input/<fingerprint>/`) and
  disjoint purpose statements.
- A change to `FrozenWorktree::tree_sha()`. It is the
  *check path's* tree OID reader, used at capture time when
  the workspace is necessarily read-only. 2.0-D's
  `BuildInputHandle::tree_sha()` re-derives at stage time,
  before any build runs; that is the structural test 2.0-D
  adds.

## Next steps (per `doc/phases/PHASE-2.md`)

- **2.0-E** — final readiness PR, about the reproducibility
  harness (the `reprotest` / `diffoscope` claim) and the
  `plan_to_materialised` capability-vs-runtime reconciliation
  check.
- **2A.1** — first real-build code, the `debian.build.sbuild`
  `ToolDefinition`. Consumes `BuildInputHandle` from 2.0-D.
- **2A.2** — `BuildCapability` impl with a real `BuildPlan`;
  carries the `BuildTarget` (2.0-C) and the `BuildInputDescriptor`
  (2.0-D).
- **2A.3** — real `sbuild` invocation against the pinned-real
  fixture. Hardens the read-only mount and the actual
  `dpkg-source -b` step.
- **2A.4** — second tool (`debian.qa.lintian`).
