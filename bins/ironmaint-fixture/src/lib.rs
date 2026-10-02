//! Lib target for the `ironmaint-fixture` crate.
//!
//! The crate is primarily a binary (`src/main.rs`). This lib exists
//! for one reason: to answer "where is that binary, and can it
//! actually run here?" for the crates that execute it.
//!
//! **Why this exists rather than `CARGO_BIN_EXE_ironmaint-fixture`.**
//! Cargo sets that variable only for tests inside the package that
//! owns the `[[bin]]`. `ironmaint-executor` and `ironmaint-testkit`
//! are different packages, so the variable is genuinely unset for
//! them — `env!("CARGO_BIN_EXE_ironmaint-fixture")` fails to compile
//! and `std::env::var` returns `None`. The comment this file used to
//! carry claimed the opposite, and four test sites each responded by
//! walking `CARGO_MANIFEST_DIR/../..` to `target/` themselves. Three
//! of them guarded the result with `path.exists()` and returned it
//! regardless.
//!
//! `exists()` is not the question worth asking. A `target/` directory
//! shared with a build for another platform holds a real file that this
//! host cannot load: it existed, so the fallback never fired, and the
//! Linux `fork` succeeded while `exec` failed `ENOEXEC` — surfacing as
//! exit code 127, indistinguishable from a shell's "command not
//! found". That is how a test-harness defect got filed as a suspected
//! bug in `ProcessExecutor::spawn` (register entry D-18) and survived a
//! full audit cycle.
//!
//! `containers/compose.yml` no longer creates that situation — the
//! container's build output is a named volume, not the host's
//! `target/`. The check below stays because the resolver must not
//! depend on every caller having configured its build layout
//! correctly, and because a resolver that trusts `exists()` cannot tell
//! a developer what is wrong when one of them does not.
//!
//! So the resolver below checks that the candidate is a binary this
//! host can load, and if it is not, it says which file, what it
//! actually is, and what was expected — by name, at resolution time,
//! instead of 127 three frames later.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    // A wrong-arch fixture binary is not an error this crate can
    // return to a caller. Every consumer is a test that is about to
    // exec the result, so the message *is* the API: a panic naming
    // the file, its real format, and the expected one is the whole
    // deliverable. See the module doc for what the alternative
    // produced.
    clippy::panic
)]

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Stable name of the fixture binary, for assertions and
/// `CARGO_BIN_EXE_<name>` lookups.
pub const FIXTURE_BINARY_NAME: &str = "ironmaint-fixture";

/// Bytes of an ELF header needed to reach `e_machine`.
const ELF_HEADER_BYTES: usize = 20;

/// `e_ident[EI_DATA]` — most-significant byte first. The other legal
/// values (0 = none, 1 = least-significant first) both read as
/// little-endian, which is the only other ordering this crate supports.
const ELF_DATA_MSB: u8 = 2;

/// Offset of `e_machine` within the ELF header.
const ELF_E_MACHINE_OFFSET: usize = 18;

/// What a candidate file's magic bytes say it is.
///
/// `Elf` carries the `e_machine` value rather than an arch name,
/// because the check that matters is numeric: an ELF for the wrong
/// architecture is just as unloadable as a Mach-O on Linux, and a
/// bare magic-byte check cannot see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryFormat {
    /// ELF object, with the `e_machine` field from the header.
    Elf(u16),
    /// Mach-O, in either byte order or the universal ("fat") form.
    MachO,
    /// PE / COFF, the Windows executable format.
    Pe,
    /// A shebang script — loadable, but not a native binary.
    Script,
    /// Something else, or a file shorter than an ELF header.
    Unknown,
}

impl fmt::Display for BinaryFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Elf(machine) => {
                write!(f, "ELF {}", arch_name(*machine).unwrap_or("unknown-arch"))
            }
            Self::MachO => f.write_str("Mach-O"),
            Self::Pe => f.write_str("PE/COFF"),
            Self::Script => f.write_str("shebang script"),
            Self::Unknown => f.write_str("unrecognised format"),
        }
    }
}

/// Render an ELF `e_machine` value for a diagnostic.
fn arch_name(machine: u16) -> Option<&'static str> {
    Some(match machine {
        3 => "x86",
        40 => "arm",
        62 => "x86_64",
        183 => "aarch64",
        243 => "riscv64",
        _ => return None,
    })
}

/// The `e_machine` value a binary built for this host would carry.
///
/// Returns `None` for an architecture this crate does not enumerate,
/// which is treated as "cannot check" rather than "reject" — an
/// unlisted host running its own correctly-built binary must not be
/// failed by a lookup table written for the common ones.
#[must_use]
pub fn expected_machine() -> Option<u16> {
    Some(match std::env::consts::ARCH {
        "x86" => 3,
        "arm" => 40,
        "x86_64" => 62,
        "aarch64" => 183,
        "riscv64" => 243,
        _ => return None,
    })
}

/// Classify a file from its leading bytes.
///
/// Reads [`ELF_HEADER_BYTES`]; a shorter file is [`BinaryFormat::Unknown`],
/// which covers a file caught mid-link as well as a genuinely unrecognised
/// one. Neither is loadable, so the two cases do not need distinguishing
/// here.
fn classify_bytes(header: &[u8]) -> BinaryFormat {
    if header.len() < ELF_HEADER_BYTES {
        return BinaryFormat::Unknown;
    }
    if header.starts_with(b"\x7fELF") {
        return BinaryFormat::Elf(read_e_machine(header));
    }
    // Mach-O in both byte orders, plus the universal "fat" magic. The
    // byte-swapped spellings are checked as raw literals because the
    // file is a byte slice, not a host integer.
    const MACHO_MAGICS: [[u8; 4]; 5] = [
        [0xfe, 0xed, 0xfa, 0xce],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xca, 0xfe, 0xba, 0xbe],
    ];
    if MACHO_MAGICS.iter().any(|m| header.starts_with(m)) {
        return BinaryFormat::MachO;
    }
    if header.starts_with(b"MZ") {
        return BinaryFormat::Pe;
    }
    if header.starts_with(b"#!") {
        return BinaryFormat::Script;
    }
    BinaryFormat::Unknown
}

/// Read `e_machine` from an ELF header, honouring `EI_DATA` so a
/// big-endian object is not read as little-endian.
fn read_e_machine(header: &[u8]) -> u16 {
    let bytes = [
        header[ELF_E_MACHINE_OFFSET],
        header[ELF_E_MACHINE_OFFSET + 1],
    ];
    match header[5] {
        ELF_DATA_MSB => u16::from_be_bytes(bytes),
        _ => u16::from_le_bytes(bytes),
    }
}

/// Classify the file at `path`.
fn classify_path(path: &Path) -> BinaryFormat {
    let mut header = [0u8; ELF_HEADER_BYTES];
    let mut file = match File::open(path) {
        Ok(f) => f,
        // An unreadable file is indistinguishable, for the caller's
        // purposes, from one that is not there — both mean "this
        // candidate is not usable", and the resolver reports every
        // candidate it rejected either way.
        Err(_) => return BinaryFormat::Unknown,
    };
    let read = match file.read(&mut header) {
        Ok(n) => n,
        Err(_) => return BinaryFormat::Unknown,
    };
    classify_bytes(&header[..read])
}

/// Whether an ELF `e_machine` can run where `host_machine` is expected.
///
/// `host_machine` is `None` when the host's architecture is not one
/// this crate enumerates, and the check is then skipped rather than
/// failed — a lookup table of the common architectures must not reject
/// a correctly-built binary on an uncommon one.
///
/// This is a separate function from [`is_loadable_here`] on purpose.
/// The `cfg!` in that one is a compile-time constant, so a test of it
/// can only run on the architecture it was written for: a test that
/// returns early on the developer's machine and on every other
/// developer or CI machine is a test that cannot fail. Taking the
/// expected machine as an argument makes the comparison testable
/// everywhere, which is the only reason it can be trusted.
fn elf_arch_matches(machine: u16, host_machine: Option<u16>) -> bool {
    host_machine.is_none_or(|expected| expected == machine)
}

/// Whether a binary of this format can be executed on this host.
///
/// Format is checked against the host OS, and — for ELF — the
/// architecture is checked too, which is the case a magic-byte-only
/// test would let through: an ELF is a valid ELF on every Linux host
/// regardless of whether the CPU can run it.
#[must_use]
pub fn is_loadable_here(format: BinaryFormat) -> bool {
    match format {
        BinaryFormat::Elf(machine) => {
            cfg!(target_os = "linux") && elf_arch_matches(machine, expected_machine())
        }
        BinaryFormat::MachO => cfg!(target_os = "macos"),
        BinaryFormat::Pe => cfg!(target_os = "windows"),
        BinaryFormat::Script => cfg!(unix),
        BinaryFormat::Unknown => false,
    }
}

/// Workspace root, resolved from this crate's manifest directory.
///
/// This crate lives at `bins/ironmaint-fixture`, so the root is two
/// levels up — and, unlike the four copies this replaces, it is
/// resolved from *this* crate regardless of which crate is asking.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

/// Directory cargo is writing build output to.
///
/// `CARGO_TARGET_DIR` wins when set, because that is where cargo
/// actually put the binary and looking anywhere else guarantees a
/// miss. Cargo re-exports the variable to the test process (verified:
/// `CARGO_TARGET_DIR=/tmp/x cargo test` prints `Ok("/tmp/x")` inside
/// the test), so this is a read of cargo's own answer rather than a
/// guess.
///
/// This is not the "no env-var reads inside domain types" rule from
/// `CLAUDE.md` — that rule is about time and identity, which must be
/// injected so a value cannot change under a decision. This is a
/// build-layout question in a test-support crate, and the process
/// environment is cargo's own channel for it.
///
/// Public because it is the one piece of the resolver's decision that a
/// caller may legitimately need to know, and — more to the point —
/// because [`crate::tests`] has to be able to assert on it from a
/// *child* process. An earlier version of that test reimplemented this
/// function inside the probe, and the reimplementation kept passing
/// after this one was broken: the test was checking a copy. See
/// `tests/ctd_probe.rs`.
#[must_use]
pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_root().join("target"), PathBuf::from)
}

/// The outcome of scanning candidate paths for a loadable binary.
struct Selection {
    /// The first candidate that exists and is loadable here.
    chosen: Option<PathBuf>,
    /// One line per candidate that existed but was refused.
    rejected: Vec<String>,
}

/// Scan `candidates` in order, returning the first that is both
/// present and loadable, plus a line for each one refused.
///
/// Split out from [`fixture_binary_path`] so the refusal path can be
/// tested with synthetic files. Asserting only on the real artifact
/// cannot test it: on any host where the workspace's own build is
/// loadable — which is every host that runs the suite — the loadable
/// candidate is found on the first try and the rejection branch is
/// never entered. That is how a resolver can have no test for the one
/// behaviour it exists to provide.
fn select(candidates: &[PathBuf]) -> Selection {
    let mut rejected = Vec::new();
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let format = classify_path(candidate);
        if is_loadable_here(format) {
            return Selection {
                chosen: Some(candidate.clone()),
                rejected,
            };
        }
        rejected.push(format!("  {} — {format}", candidate.display()));
    }
    Selection {
        chosen: None,
        rejected,
    }
}

/// Resolve the `ironmaint-fixture` binary, returning a path this host
/// can actually load.
///
/// Looks in `target/debug/`, then `target/release/`, under
/// [`target_dir`], returning the first candidate that is present *and*
/// loadable. A candidate that is present but not loadable does not
/// satisfy the request: it is recorded and reported rather than
/// returned, because returning it reproduces the `ENOEXEC`-becomes-127
/// failure this exists to prevent.
///
/// # Panics
///
/// If no candidate is usable. The message names every path tried and
/// what was wrong with each, so the failure names its own cause —
/// a stale cross-platform `target/` read as an unexplained 127.
#[must_use]
pub fn fixture_binary_path() -> PathBuf {
    let root = target_dir();
    let candidates: Vec<PathBuf> = ["debug", "release"]
        .iter()
        .map(|profile| root.join(profile).join(FIXTURE_BINARY_NAME))
        .collect();

    let Selection { chosen, rejected } = select(&candidates);
    chosen.unwrap_or_else(|| {
        panic!(
            "no loadable `{FIXTURE_BINARY_NAME}` binary for this host ({os}/{arch}).\n\
             \n\
             Looked under: {root}\n\
             Checked, and rejected:\n{rejected_detail}\n\
             \n\
             A binary that exists but cannot be loaded here is almost always a \
             `target/` directory shared with a build for another platform — a \
             macOS Mach-O binary sitting where cargo will put the Linux one.\n\
             \n\
             Build it for this host first:\n    \
             cargo build -p {FIXTURE_BINARY_NAME}\n",
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            root = root.display(),
            rejected_detail = if rejected.is_empty() {
                "  (neither the debug/ nor the release/ profile directory exists)".to_owned()
            } else {
                rejected.join("\n")
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF header carrying `e_machine`, little-endian.
    fn elf_header(machine: u16) -> [u8; ELF_HEADER_BYTES] {
        let mut header = [0u8; ELF_HEADER_BYTES];
        header[0..4].copy_from_slice(b"\x7fELF");
        // EI_DATA = 1, least-significant byte first.
        header[5] = 1;
        header[ELF_E_MACHINE_OFFSET..ELF_E_MACHINE_OFFSET + 2]
            .copy_from_slice(&machine.to_le_bytes());
        header
    }

    #[test]
    fn classify_reads_the_elf_machine() {
        assert_eq!(classify_bytes(&elf_header(62)), BinaryFormat::Elf(62));
        assert_eq!(classify_bytes(&elf_header(183)), BinaryFormat::Elf(183));
    }

    /// The teeth-check.
    ///
    /// An ELF for the wrong architecture is still an ELF, so a
    /// magic-byte-only check accepts it and the failure does not
    /// surface until `exec` returns 127.
    ///
    /// Two earlier drafts of this test were not able to fail. The
    /// first asserted that `classify` reads two `e_machine` values
    /// apart, which is a fact about the parser and says nothing about
    /// the decision — deleting the comparison left it green. The
    /// second asserted through `is_loadable_here`, whose `cfg!` is a
    /// compile-time constant, so on a non-Linux host it returned
    /// before asserting anything. Both are the shape D-14 records:
    /// a test that cannot fail on the machine running it.
    #[test]
    fn an_elf_for_the_wrong_architecture_is_rejected() {
        assert!(elf_arch_matches(62, Some(62)), "x86_64 on x86_64");
        assert!(elf_arch_matches(183, Some(183)), "aarch64 on aarch64");
        assert!(
            !elf_arch_matches(183, Some(62)),
            "aarch64 ELF accepted on an x86_64 host"
        );
        assert!(
            !elf_arch_matches(62, Some(183)),
            "x86_64 ELF accepted on an aarch64 host"
        );
    }

    /// An unlisted host architecture skips the check rather than
    /// failing it: the table is not exhaustive, and a correctly-built
    /// binary must not be rejected because of a gap in it.
    #[test]
    fn an_unlisted_host_architecture_skips_the_check() {
        assert!(elf_arch_matches(0x9999, None));
    }

    /// `classify` must read `e_machine` rather than collapsing every
    /// ELF to one value, or the comparison above has nothing to
    /// compare.
    #[test]
    fn classify_reports_the_elf_machine_it_read() {
        assert_eq!(classify_bytes(&elf_header(62)), BinaryFormat::Elf(62));
        assert_eq!(classify_bytes(&elf_header(183)), BinaryFormat::Elf(183));
    }

    /// The paired claim for the *format* branch: whatever this host
    /// is, the formats that are not its own are refused.
    #[test]
    fn foreign_formats_are_refused_on_this_host() {
        let natives: &[BinaryFormat] = if cfg!(target_os = "macos") {
            &[BinaryFormat::MachO]
        } else if cfg!(target_os = "windows") {
            &[BinaryFormat::Pe]
        } else {
            &[BinaryFormat::Elf(expected_machine().unwrap_or(0))]
        };
        for foreign in [BinaryFormat::MachO, BinaryFormat::Pe, BinaryFormat::Unknown] {
            if natives.contains(&foreign) {
                continue;
            }
            assert!(
                !is_loadable_here(foreign),
                "{foreign} was accepted on this host"
            );
        }
    }

    #[test]
    fn classify_honours_endianness() {
        let mut header = elf_header(62);
        header[5] = ELF_DATA_MSB;
        header[ELF_E_MACHINE_OFFSET..ELF_E_MACHINE_OFFSET + 2]
            .copy_from_slice(&62u16.to_be_bytes());
        assert_eq!(classify_bytes(&header), BinaryFormat::Elf(62));
    }

    #[test]
    fn classify_recognises_non_elf_formats() {
        for (magic, expected) in [
            (&b"\xcf\xfa\xed\xfe"[..], BinaryFormat::MachO),
            (&b"\xce\xfa\xed\xfe"[..], BinaryFormat::MachO),
            (&b"\xca\xfe\xba\xbe"[..], BinaryFormat::MachO),
            (&b"MZ\x90\x00"[..], BinaryFormat::Pe),
            (&b"#!/bin/sh\n"[..], BinaryFormat::Script),
        ] {
            let mut header = [0u8; ELF_HEADER_BYTES];
            header[..magic.len()].copy_from_slice(magic);
            assert_eq!(classify_bytes(&header), expected, "magic {magic:?}");
        }
    }

    /// A file shorter than an ELF header — a half-written binary
    /// caught mid-link — must not classify as anything loadable.
    #[test]
    fn classify_rejects_short_files() {
        for len in 0..ELF_HEADER_BYTES {
            let header = vec![0u8; len];
            assert_eq!(classify_bytes(&header), BinaryFormat::Unknown, "len {len}");
            assert!(!is_loadable_here(classify_bytes(&header)), "len {len}");
        }
    }

    #[test]
    fn unrecognised_bytes_are_not_loadable() {
        let header = b"this is a text file, not a binary at all....";
        assert_eq!(classify_bytes(header), BinaryFormat::Unknown);
        assert!(!is_loadable_here(BinaryFormat::Unknown));
    }

    /// The host's own format is always loadable. This is the check
    /// that would otherwise reject a correctly-built fixture on macOS,
    /// where the binary is Mach-O and never ELF — a resolver written
    /// for the Linux container alone would break the host build.
    #[test]
    fn the_hosts_own_format_is_loadable() {
        let native = if cfg!(target_os = "macos") {
            BinaryFormat::MachO
        } else if cfg!(target_os = "windows") {
            BinaryFormat::Pe
        } else {
            BinaryFormat::Elf(expected_machine().unwrap_or(0))
        };
        assert!(is_loadable_here(native), "{native} should load here");
    }

    /// Write a file with the given leading bytes into a fresh temp dir.
    fn plant(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("plant candidate");
        path
    }

    /// A header this host would accept, so a positive result is
    /// attributable to the candidate under test and not to the host.
    fn native_header() -> [u8; ELF_HEADER_BYTES] {
        if cfg!(target_os = "macos") {
            let mut h = [0u8; ELF_HEADER_BYTES];
            h[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
            h
        } else {
            let mut h = elf_header(expected_machine().unwrap_or(62));
            if cfg!(target_os = "windows") {
                h[..2].copy_from_slice(b"MZ");
            }
            h
        }
    }

    /// A header this host would refuse, built the same way so the only
    /// difference is the architecture — the case a magic-byte-only
    /// check cannot see.
    fn foreign_arch_header() -> [u8; ELF_HEADER_BYTES] {
        let native = expected_machine().unwrap_or(62);
        elf_header(if native == 62 { 183 } else { 62 })
    }

    /// The resolver's reason for existing, tested directly.
    ///
    /// An ELF for the wrong architecture is present and passes
    /// `is_file()` — exactly the condition the three resolvers this
    /// replaced were guarding with `exists()`. The candidate must be
    /// skipped, not returned, and must appear in the rejection report
    /// by path and by what it actually is.
    #[test]
    fn a_present_but_foreign_binary_is_refused_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let foreign = plant(dir.path(), "ironmaint-fixture", &foreign_arch_header());

        let selection = select(std::slice::from_ref(&foreign));

        assert_eq!(
            selection.chosen,
            None,
            "a binary this host cannot load was returned: {}",
            foreign.display()
        );
        assert_eq!(selection.rejected.len(), 1, "{:?}", selection.rejected);
        let report = &selection.rejected[0];
        assert!(
            report.contains(&foreign.display().to_string()),
            "report does not name the file: {report}"
        );
        assert!(
            !report.contains("unrecognised"),
            "the foreign file was not identified: {report}"
        );
    }

    /// And the paired positive: the same scan accepts a native
    /// candidate. Without this pair, "always return None" would pass
    /// the test above.
    #[test]
    fn a_present_and_native_binary_is_chosen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let native = plant(dir.path(), "ironmaint-fixture", &native_header());

        let selection = select(std::slice::from_ref(&native));

        assert_eq!(selection.chosen, Some(native));
        assert!(selection.rejected.is_empty(), "{:?}", selection.rejected);
    }

    /// A foreign `debug` must not shadow a native `release`: the
    /// fallback exists precisely for that case, and the old
    /// `exists()`-guarded walk never reached it.
    #[test]
    fn a_native_fallback_is_preferred_over_a_foreign_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let foreign = plant(dir.path(), "foreign", &foreign_arch_header());
        let native = plant(dir.path(), "native", &native_header());

        let selection = select(&[foreign, native.clone()]);

        assert_eq!(selection.chosen, Some(native));
        assert_eq!(selection.rejected.len(), 1, "{:?}", selection.rejected);
    }

    /// Missing candidates are skipped silently, not reported as
    /// rejected — they were never candidates in the first place.
    #[test]
    fn a_missing_candidate_is_skipped_without_a_rejection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let absent = dir.path().join("nope");
        let native = plant(dir.path(), "ironmaint-fixture", &native_header());

        let selection = select(&[absent, native.clone()]);

        assert_eq!(selection.chosen, Some(native));
        assert!(selection.rejected.is_empty(), "{:?}", selection.rejected);
    }

    /// End to end against the real artifact. The resolved path must
    /// classify as something this host can load, or the resolver is
    /// handing back the very file it exists to reject.
    #[test]
    fn resolved_path_is_loadable_here() {
        let path = fixture_binary_path();
        assert!(path.is_file(), "{} is not a file", path.display());
        let format = classify_path(&path);
        assert!(
            is_loadable_here(format),
            "resolved {} but it is {format} and this host is {}/{}",
            path.display(),
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
    }

    /// `CARGO_TARGET_DIR` must win over the workspace default, because
    /// that is the directory cargo actually wrote the binary to.
    ///
    /// This is the arrangement `containers/compose.yml` relies on: the
    /// container's build output is a named volume at
    /// `/var/cargo-target`, not the host's `target/` on the bind mount.
    /// A resolver that looked only at `<workspace>/target` would miss
    /// every binary in the container and report a build that plainly
    /// succeeded.
    ///
    /// Setting a process-wide env var is not safe to do inside a test
    /// binary — other tests in this crate read the same process — so
    /// this asserts the property through a child `cargo test`
    /// invocation with the variable set, and reads back what the child
    /// resolved. That is also the only way to prove the part that
    /// actually matters: that cargo re-exports the variable *to the
    /// test process*, which is the assumption the whole branch rests
    /// on. If cargo ever stopped doing that, this test fails instead
    /// of the container failing mysteriously.
    #[test]
    fn cargo_target_dir_overrides_the_workspace_default() {
        let dir = tempfile::tempdir().expect("temp dir");

        // A child cargo test, same as this one, with CARGO_TARGET_DIR
        // pointed somewhere the workspace default does not reach.
        let output = std::process::Command::new(env!("CARGO"))
            .args(["test", "--quiet", "--package", "ironmaint-fixture"])
            .args(["--test", "ctd_probe", "--", "--nocapture"])
            .env("CARGO_TARGET_DIR", dir.path())
            // A probe target dir is fine to throw away, but keep the
            // build output next to the probe's own temp dir so this
            // test does not evict the workspace's incremental cache.
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run child cargo test");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "child cargo test failed\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
        );
        // The probe prints the directory it resolved. If cargo did not
        // re-export CARGO_TARGET_DIR, the child would have printed the
        // workspace default and this assertion fails.
        assert!(
            stdout.contains(&format!("PROBE_TARGET_DIR={}", dir.path().display())),
            "child did not resolve CARGO_TARGET_DIR; it reported something else\n\
             --- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
        );
    }
}
