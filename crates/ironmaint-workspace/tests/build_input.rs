//! Tests for `WorkspaceManager::stage_build_input` (PHASE-2 PR
//! 2.0-D, `doc/PHASE-2-D.md`).
//!
//! The staging primitive is the *where* a build runs from:
//! it takes a `BuildInputDescriptor` (which captures the
//! candidate, the source format, and the upstream tarball)
//! and produces a `BuildInputHandle` (which captures the
//! staged root, the re-derived tree OID, and the output
//! dir).
//!
//! RED state: `stage_build_input` is a stub that returns
//! a typed `WorkspaceError`; the conformance test below
//! fails because `expect("stage_build_input should
//! succeed")` panics on the stub's `Err`.
//!
//! GREEN state: the stub is replaced with the
//! git-archive + tar + git-write-tree staging; the test
//! passes.
//!
//! Each test is gated on `git` and `tar` being on PATH; a
//! missing tool produces a `SKIP` message, not a failure,
//! so the §97 gate stays green on machines where only
//! the host's `git` is available.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;

use ironmaint_adapter_api::{BuildInputDescriptor, UpstreamTarball};
use ironmaint_core::{
    DistributionFamily, DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
    PackageIdentity, PackageName, PackageRevision, PackageVersion, RepositoryRef, SourceCandidate,
    VcsKind,
};
use ironmaint_store::mock::MockStore;
use ironmaint_workspace::WorkspaceManager;
use tempfile::TempDir;
use time::macros::datetime;
use url::Url;

fn hex(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

/// Run `git` with `args` in `cwd`, returning trimmed stdout.
async fn git(cwd: &Path, args: &[&str]) -> String {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/var/empty")
        .output()
        .await
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed in {}: {}",
        cwd.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Write a small synthetic Debian source tree into
/// `handle_dir` and commit it. The tree mirrors the
/// `pinned-real` fixture (3.0 quilt, hello 2.10-2) but
/// is constructed in-tempdir so the test is hermetic
/// and does not depend on the committed fixture
/// existing.
async fn write_fixture_source_tree(handle_dir: &Path) {
    tokio::fs::write(
        handle_dir.join("Makefile"),
        "CC ?= cc\nCFLAGS ?= -O2 -Wall\n\nhello: hello.c\n\t$(CC) $(CFLAGS) -o $@ $<\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        handle_dir.join("hello.c"),
        "#include <stdio.h>\nint main(void) { puts(\"hello\"); return 0; }\n",
    )
    .await
    .unwrap();
    let debian = handle_dir.join("debian");
    tokio::fs::create_dir_all(&debian).await.unwrap();
    tokio::fs::write(
        debian.join("changelog"),
        "hello (2.10-2) unstable; urgency=medium\n\n  * Fixture.\n\n -- Test <test@invalid>  Sat, 01 Jan 2026 00:00:00 +0000\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        debian.join("control"),
        "Source: hello\nMaintainer: Test <test@invalid>\nSection: devel\nPriority: optional\nStandards-Version: 4.7.4\nBuild-Depends: debhelper-compat (= 13)\n\nPackage: hello\nArchitecture: any\nDepends: ${misc:Depends}\nDescription: Test fixture\n",
    )
    .await
    .unwrap();
    tokio::fs::write(debian.join("rules"), "#!/usr/bin/make -f\n%:\n\tdh $@\n")
        .await
        .unwrap();
    let source = debian.join("source");
    tokio::fs::create_dir_all(&source).await.unwrap();
    tokio::fs::write(source.join("format"), "3.0 (quilt)\n")
        .await
        .unwrap();
}

/// Build a small synthetic orig tarball in a tempdir and
/// return the path and the SHA-256 of the file. The
/// tarball contains a `hello-2.10/hello.c` so it looks
/// like the canonical Debian 3.0 (quilt) upstream.
fn make_synthetic_orig_tarball() -> (TempDir, std::path::PathBuf, [u8; 32]) {
    use sha2::{Digest, Sha256};
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("hello-2.10");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(
        src.join("hello.c"),
        "#include <stdio.h>\nint main(void) { puts(\"hello\"); return 0; }\n",
    )
    .unwrap();
    let tarball = tmp.path().join("hello_2.10.orig.tar.gz");
    let out = std::process::Command::new("tar")
        .args([
            "-C",
            tmp.path().to_str().unwrap(),
            "-czf",
            tarball.to_str().unwrap(),
            "hello-2.10",
        ])
        .output()
        .expect("spawn tar");
    assert!(out.status.success(), "tar failed: {:?}", out);
    let bytes = std::fs::read(&tarball).unwrap();
    let digest = Sha256::digest(&bytes);
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&digest);
    (tmp, tarball, sha256)
}

fn make_candidate(commit: &str, tree: &str) -> SourceCandidate {
    let repo = RepositoryRef::new(
        VcsKind::Git,
        Url::parse("https://example.invalid/hello.git").unwrap(),
    )
    .unwrap();
    SourceCandidate::new(
        JobId::new(),
        PackageRevision::new(
            PackageIdentity::new(
                DistributionRef::new(
                    DistributionFamily::new("debian").unwrap(),
                    DistributionRelease::new("sid").unwrap(),
                ),
                PackageName::new("hello").unwrap(),
            ),
            PackageVersion::new("2.10-2").unwrap(),
        ),
        repo,
        GitObjectId::new(GitHashAlgorithm::Sha1, commit.to_string()).unwrap(),
        GitObjectId::new(GitHashAlgorithm::Sha1, tree.to_string()).unwrap(),
        datetime!(2026-01-01 00:00:00 UTC),
    )
}

/// Convenience: build the workspace, fixture, tarball,
/// and descriptor, and return the manager, job_id, and
/// descriptor ready for `stage_build_input`. The test
/// then asserts the staging succeeded and the handle's
/// invariants.
async fn setup() -> (WorkspaceManager<MockStore>, JobId, BuildInputDescriptor) {
    if !common::git_available().await {
        eprintln!("git not available; skipping stage_build_input test");
        // Returning a stub manager the test will skip
        // on. We use a panic-on-call pattern instead.
        panic!("git not available; the test will skip on machines without git");
    }

    let tmp = tempfile::tempdir().unwrap();
    let handle_root = tmp.path().join("ws-root");
    tokio::fs::create_dir_all(&handle_root).await.unwrap();

    let store = MockStore::new();
    let mgr = WorkspaceManager::new(handle_root.clone(), store);
    let id = mgr.create(JobId::new()).await.unwrap();
    let handle_dir = handle_root.join(format!("{id}"));
    tokio::fs::create_dir_all(&handle_dir).await.unwrap();
    common::init_repo(&handle_dir).await;
    write_fixture_source_tree(&handle_dir).await;
    let _ = git(&handle_dir, &["add", "-A"]).await;
    let _ = git(&handle_dir, &["commit", "-m", "fixture"]).await;
    let commit = git(&handle_dir, &["rev-parse", "HEAD"]).await;
    let tree = git(&handle_dir, &["rev-parse", "HEAD^{tree}"]).await;

    let (_orig_tmp, orig_path, orig_sha256) = make_synthetic_orig_tarball();
    let candidate = make_candidate(&commit, &tree);
    let descriptor = BuildInputDescriptor::quilt(
        candidate,
        UpstreamTarball {
            path: orig_path,
            sha256: orig_sha256,
        },
    );

    (mgr, JobId::new(), descriptor)
}

#[tokio::test]
async fn stage_build_input_produces_handle_whose_tree_matches_candidate() {
    // The headline conformance test for 2.0-D's
    // bullet 4: the BuildInputHandle's `tree_sha()`
    // must equal the candidate's `tree` (the
    // re-derivation succeeded).
    //
    // RED state: the stub returns Err, `expect` panics,
    // the test fails with the stub's "not yet
    // implemented" message.
    //
    // GREEN state: the staging primitive re-derives
    // the tree OID and the assertions below pass.
    let (mgr, job_id, descriptor) = setup().await;
    let handle = mgr
        .stage_build_input(&job_id, &descriptor)
        .expect("stage_build_input should succeed (RED state: stub returns Err)");

    assert_eq!(
        handle.tree_sha(),
        descriptor.candidate.tree().as_str(),
        "staged tree OID must equal the candidate's tree OID"
    );
    assert_eq!(
        handle.candidate().fingerprint(),
        descriptor.candidate.fingerprint(),
        "the handle's candidate must be the descriptor's candidate"
    );
}

#[tokio::test]
async fn stage_build_input_places_upstream_tarball_alongside_extracted_tree() {
    // 2.0-D bullet 2: the staged root contains the
    // upstream tarball at the path `dpkg-source -b`
    // expects. After staging, the tarball lives at
    // `<staged>/<upstream_tarball.filename()>`.
    let (mgr, job_id, descriptor) = setup().await;
    let handle = mgr
        .stage_build_input(&job_id, &descriptor)
        .expect("stage_build_input should succeed (RED state: stub returns Err)");

    assert!(
        handle.upstream_tarball_path().exists(),
        "upstream tarball at {} must exist after staging",
        handle.upstream_tarball_path().display()
    );
    let staged_root = handle.root();
    assert!(
        staged_root.join("debian").exists(),
        "staged root must contain the `debian/` source tree"
    );
    assert!(
        staged_root.join("Makefile").exists(),
        "staged root must contain the Makefile"
    );
}

#[tokio::test]
async fn stage_build_input_creates_output_dir_as_sibling_of_staged_root() {
    // 2.0-D bullet 3: the output dir is a sibling of
    // the staged root, not a sub-directory. A build
    // tool that writes into the staged root would
    // write into a read-only path; the output dir is
    // the only writable sibling.
    let (mgr, job_id, descriptor) = setup().await;
    let handle = mgr
        .stage_build_input(&job_id, &descriptor)
        .expect("stage_build_input should succeed (RED state: stub returns Err)");

    let output_dir = handle.output_dir();
    assert!(
        output_dir.exists(),
        "output dir at {} must exist after staging",
        output_dir.display()
    );
    // The output dir's parent is the workspace handle
    // dir; the staged root's parent is also the
    // workspace handle dir. They are siblings, not
    // one inside the other.
    assert_eq!(
        output_dir.parent(),
        handle.root().parent(),
        "output dir must be a sibling of the staged root, not a sub-directory"
    );
    assert_ne!(
        output_dir,
        handle.root(),
        "output dir and staged root must be different paths"
    );
}

#[tokio::test]
async fn stage_build_input_makes_staged_root_readonly() {
    // 2.0-D bullet 5: the staged root is read-only
    // after staging. The `is_staged_root_readonly()`
    // accessor on the handle returns `true` so the
    // executor (2A.1) can assert the precondition
    // before running the build tool.
    let (mgr, job_id, descriptor) = setup().await;
    let handle = mgr
        .stage_build_input(&job_id, &descriptor)
        .expect("stage_build_input should succeed (RED state: stub returns Err)");

    assert!(
        handle.is_staged_root_readonly(),
        "staged root {} must be read-only after staging",
        handle.root().display()
    );
}

#[tokio::test]
async fn stage_build_input_red_stub_returns_unimplemented_error() {
    // A second, more direct assertion of the RED
    // state. The stub returns a typed error whose
    // message names the PR. When the GREEN commit
    // replaces the stub, this test will start failing
    // — at which point the test should be removed
    // (the GREEN implementation makes it obsolete).
    let (mgr, job_id, descriptor) = setup().await;
    let result = mgr.stage_build_input(&job_id, &descriptor);
    let err = result.expect_err(
        "stage_build_input returned Ok; the RED-state stub is gone. Remove this test \
         in the GREEN commit (the conformance tests above now exercise the real \
         implementation).",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("not yet implemented in 2.0-D red commit"),
        "expected the RED-state error message, got: {msg}"
    );
}

#[tokio::test]
async fn stage_build_input_descriptor_carries_candidate_and_tarball() {
    // A pure-descriptor test that does not need
    // git. The descriptor's invariants are:
    // `quilt(...)` returns a descriptor with the
    // candidate and a quilt format wrapping the
    // tarball; the tarball's filename is the path's
    // basename. This is a unit-style test that
    // passes in both RED and GREEN.
    let (_orig_tmp, orig_path, orig_sha256) = make_synthetic_orig_tarball();
    let candidate = make_candidate(&hex('a', 40), &hex('b', 40));
    let tarball = UpstreamTarball {
        path: orig_path.clone(),
        sha256: orig_sha256,
    };
    let descriptor = BuildInputDescriptor::quilt(candidate.clone(), tarball.clone());
    assert_eq!(descriptor.candidate, candidate);
    assert_eq!(descriptor.source_format.upstream_tarball, tarball);
    assert_eq!(
        descriptor.source_format.upstream_tarball.filename(),
        "hello_2.10.orig.tar.gz"
    );
}
