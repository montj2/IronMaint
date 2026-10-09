//! 1B.2 RED/GREEN — dpkg oracle-driven version comparison corpus.
//!
//! The corpus is a fixed set of `(left, right, expected)` triples
//! that exercise the §15 grammar: epoch, upstream, debian_revision,
//! tilde, plus, colon, binNMU, numeric/non-numeric ordering. The
//! unit half walks every pair in both directions against the
//! Debian stub's comparator and asserts it produces the expected
//! `Ordering`. The oracle half shells out to
//! `dpkg --compare-versions` (when `dpkg` is on `$PATH`) and asserts
//! the comparator agrees with the dpkg native result.
//!
//! The oracle half is the teeth of §15's "compare a corpus against
//! `dpkg --compare-versions`" requirement. The unit half is the
//! teeth of the broken-then-fixed pattern: the RED commit's
//! `compare` body returns `Ordering::Equal` for everything, so
//! every `Less` and `Greater` assertion in the corpus goes red.
//! The GREEN commit's `debversion`-backed `compare` makes them pass.
//!
//! `dpkg` is part of base Debian, so the oracle runs automatically
//! inside the §97 gate (which executes in `ironmaint/workspace:0.1`,
//! derived from `rust:1.94.0-bookworm` → `buildpack-deps:bookworm`).
//! On a host without `dpkg` (macOS, minimal CI), the oracle half
//! is a no-op skip; the unit half still runs and the gate is
//! still meaningful.
//!
//! The corpus is derived from the dpkg §5.6.12 algorithm
//! (debian-policy 5.6.12 "Version" — the same algorithm
//! `dpkg --compare-versions` implements). The pairs that distinguish
//! dpkg from bytewise `String::cmp` are the most important:
//!
//!   - `1.0.0-2 < 1.0.0-10` — dpkg compares numeric runs as
//!     integers; bytewise says `2 > 1` and would say `1.0.0-10
//!     < 1.0.0-2`.
//!   - `1.0.0~rc1 < 1.0.0` — dpkg puts `~` before any non-`~`
//!     character and before end-of-string; bytewise would say
//!     `0x7E > 0x2E` and get this wrong.
//!   - `1.0-0 == 1.0` — dpkg's "missing debian_revision defaults
//!     to 0" rule; bytewise would say they're different lengths
//!     and get the order wrong.
//!   - `2.1-1 < 2.1-1+b1` — binNMU `+bN` in debian_revision sorts
//!     after the un-bumped revision.

use std::cmp::Ordering;
use std::process::Command;

use debian_stub::DebianStubAdapter;
use ironmaint_adapter_api::DistributionAdapter;
use ironmaint_core::PackageVersion;

/// A single corpus triple. `expected` is the dpkg-native ordering.
struct CorpusEntry {
    left: &'static str,
    right: &'static str,
    expected: Ordering,
}

const CORPUS: &[CorpusEntry] = &[
    // ---- equality (load-bearing: dpkg normalises "missing revision
    //      is 0" — that is, "1.0" and "1.0-0" are equal) ----
    CorpusEntry {
        left: "1.0",
        right: "1.0-0",
        expected: Ordering::Equal,
    },
    CorpusEntry {
        left: "0:0-0",
        right: "0:0-0",
        expected: Ordering::Equal,
    },
    CorpusEntry {
        left: "1.0",
        right: "1.0",
        expected: Ordering::Equal,
    },
    // ---- epoch (numeric; absent epoch = 0) ----
    CorpusEntry {
        left: "2:1.0",
        right: "1:99.0",
        expected: Ordering::Greater,
    },
    // ---- tilde (sorts before any non-~; before end-of-string) ----
    CorpusEntry {
        left: "1.0.0~rc1",
        right: "1.0.0",
        expected: Ordering::Less,
    },
    CorpusEntry {
        left: "1.0~rc1",
        right: "1.0~~rc1",
        // Position-by-position: 1=1, .=., 0=0, ~=~, then `~` (in
        // ~~rc1) vs `r` (in ~rc1): ~ < r, so ~~rc1 < ~rc1. The
        // corpus walks the pair in this direction; the reverse
        // direction is exercised by the unit half's swap.
        expected: Ordering::Greater,
    },
    // ---- binNMU (+bN in debian_revision) ----
    CorpusEntry {
        left: "2.1-1",
        right: "2.1-1+b1",
        expected: Ordering::Less,
    },
    CorpusEntry {
        left: "2.1-1+b1",
        right: "2.1-1+b2",
        expected: Ordering::Less,
    },
    // ---- numeric runs in upstream / revision (this is the
    //      bytewise-String::cmp failure mode) ----
    CorpusEntry {
        left: "1.0.0-2",
        right: "1.0.0-10",
        expected: Ordering::Less,
    },
    // ---- debian_revision lex order (non-digit char sorts after
    //      end of string; .fcNN etc. sort after the bare number) ----
    CorpusEntry {
        left: "1.9.0-1",
        right: "1.9.0-1.fc45",
        expected: Ordering::Less,
    },
    CorpusEntry {
        left: "1.9.0-1.fc45",
        right: "1.9.0-1",
        expected: Ordering::Greater,
    },
    // ---- plus in upstream (the + is a non-digit char in upstream;
    //      - separates upstream from debian_revision) ----
    CorpusEntry {
        left: "1.0+dfsg-1",
        right: "1.0+dfsg-2",
        expected: Ordering::Less,
    },
];

/// Test failures in this binary are returned as `Result` so we
/// can `?` out instead of panicking; `expect` and `panic` are
/// workspace-denied by `clippy::expect_used` and `clippy::panic`.
type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn pv(s: &str) -> Result<PackageVersion, ironmaint_core::CoreError> {
    PackageVersion::new(s)
}

/// The unit half: every corpus entry is asserted against the
/// Debian stub's comparator. This is the teeth of the
/// broken-then-fixed pattern: the RED commit's `compare` returns
/// `Ok(Ordering::Equal)` for any two valid versions, so every
/// `Less` and `Greater` entry fails here.
#[test]
fn corpus_unit_matches_debian_stub() -> TestResult<()> {
    let adapter = DebianStubAdapter::new();
    let v = adapter.versioning();
    for entry in CORPUS {
        let l = pv(entry.left)?;
        let r = pv(entry.right)?;
        let got = v.compare(&l, &r).map_err(|e| {
            format!(
                "compare({:?}, {:?}) errored: {}",
                entry.left, entry.right, e
            )
        })?;
        if got != entry.expected {
            return Err(format!(
                "debian-stub.compare({:?}, {:?}) returned {:?}, expected {:?}",
                entry.left, entry.right, got, entry.expected
            )
            .into());
        }
        // The reverse direction is the same property under swap.
        let rev = v.compare(&r, &l).map_err(|e| {
            format!(
                "compare({:?}, {:?}) errored: {}",
                entry.right, entry.left, e
            )
        })?;
        if rev != entry.expected.reverse() {
            return Err(format!(
                "debian-stub.compare({:?}, {:?}) returned {:?}, expected {:?} (reversed)",
                entry.right,
                entry.left,
                rev,
                entry.expected.reverse()
            )
            .into());
        }
    }
    Ok(())
}

/// `dpkg` is on `$PATH`? If yes, run the oracle half. If no,
/// skip (the test still passes; the gate runs in the workspace
/// image where `dpkg` is part of base Debian).
fn dpkg_available() -> bool {
    Command::new("dpkg")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `dpkg --compare-versions <left> <op> <right>` and return
/// whether dpkg agrees the relation holds.
fn dpkg_says(left: &str, op: &str, right: &str) -> TestResult<bool> {
    let output = Command::new("dpkg")
        .args(["--compare-versions", left, op, right])
        .output()
        .map_err(|e| format!("failed to spawn dpkg: {}", e))?;
    Ok(output.status.success())
}

/// Ask dpkg to compare two version strings. dpkg's exit status
/// is the only thing that matters; it does not write the result
/// to stdout in a stable form. We synthesise the result by
/// running `--compare-versions` three times with the three
/// relations and seeing which one is true.
fn dpkg_ordering(left: &str, right: &str) -> TestResult<Ordering> {
    if dpkg_says(left, "eq", right)? {
        return Ok(Ordering::Equal);
    }
    if dpkg_says(left, "lt", right)? {
        return Ok(Ordering::Less);
    }
    if dpkg_says(left, "gt", right)? {
        return Ok(Ordering::Greater);
    }
    Err(format!(
        "dpkg --compare-versions returned non-success for all three relations on ({:?}, {:?})",
        left, right
    )
    .into())
}

/// The oracle half: every corpus entry is checked against
/// `dpkg --compare-versions`. If `dpkg` is not available, this
/// test is a no-op (it still passes). If `dpkg` is available
/// and the Debian stub disagrees with dpkg, this test fails.
#[test]
fn corpus_matches_dpkg_oracle() -> TestResult<()> {
    if !dpkg_available() {
        // No dpkg — skip. The unit half still runs.
        return Ok(());
    }
    let adapter = DebianStubAdapter::new();
    let v = adapter.versioning();
    for entry in CORPUS {
        let l = pv(entry.left)?;
        let r = pv(entry.right)?;
        let dpkg = dpkg_ordering(entry.left, entry.right)?;
        let stub = v.compare(&l, &r).map_err(|e| {
            format!(
                "compare({:?}, {:?}) errored: {}",
                entry.left, entry.right, e
            )
        })?;
        if stub != dpkg {
            return Err(format!(
                "debian-stub.compare({:?}, {:?}) = {:?}, but dpkg --compare-versions says {:?}",
                entry.left, entry.right, stub, dpkg
            )
            .into());
        }
    }
    Ok(())
}
