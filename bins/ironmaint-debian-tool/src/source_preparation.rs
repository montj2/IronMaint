//! `debian.inspect.source_preparation` — the 1C.1 real check.
//!
//! PHASE-1.md §17 enumerates six checks; the implementation
//! walks them in order, recording the per-check results in
//! [`SourcePreparationFindings`] and aggregating them into
//! the [`Verdict`] tri-state.
//!
//! Pure-Rust parsers (no shell-out to `dpkg-parsechangelog`).
//! The §16 spec calls for "focused parser crates over
//! pulling a large Debian workspace framework into IronMaint";
//! `deb822-lossless` (for `debian/control`) and a hand-rolled
//! `debian/changelog` parser (the format is a degenerate
//! RFC 822) cover what 1C.1 needs. The opt-in oracle test
//! (in `tests/source_preparation_oracle.rs`) compares the
//! pure-Rust result against `dpkg-parsechangelog` when that
//! binary is available — the oracle runs in the
//! `ironmaint/debian-tools:0.1` image but does not gate the
//! §97 set.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use deb822_lossless::Deb822;
use serde::Deserialize;

use crate::report::{
    ChangelogIdentity, DebianSourcePreparationV1, Diagnostic, REPORT_KIND, SCHEMA_VERSION,
    SourcePreparationFindings, Verdict,
};

/// A failed attempt to read or parse the workspace tree.
///
/// The public surface is a [`DebianSourcePreparationV1`] —
/// every error path produces one, with `verdict` set
/// appropriately. This enum is a private intermediate
/// only used to thread the right `verdict` through the
/// fallible parsing steps.
#[derive(Debug)]
enum PrepError {
    /// A file the tool expected was absent or unreadable.
    /// Maps to `verdict: "infrastructure_error"` per §17.
    Infra(String),
}

/// The runtime's `ironmaint.candidate_input.v2` payload (PHASE-1.md §7).
///
/// Only the fields the tool actually uses are decoded; the
/// rest are tolerated (serde's `deny_unknown_fields` is
/// deliberately off) so an old v1 payload that pre-dates the
/// `workspace_path` field does not reject the tool. The
/// unused fields are read by the `Debug` derive and are
/// pinned by the `parse_keeps_unknown_fields_off` test, so
/// the `dead_code` deny is satisfied via `#[allow]`.
#[derive(Debug, Deserialize)]
pub struct CandidateInput {
    #[allow(dead_code)]
    pub schema: String,
    #[allow(dead_code)]
    pub job_id: String,
    #[allow(dead_code)]
    pub check_id: String,
    pub package: Package,
    #[serde(default)]
    pub workspace_path: Option<PathBuf>,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
}

/// Run the 1C.1 check against `workspace_path` for the
/// candidate whose `source_name` and `version` are the
/// values the runtime passed in.
///
/// Public so `tests/source_preparation.rs` and the daemon's
/// integration test can drive the logic without a real
/// subprocess. The `main.rs` entry point is a thin wrapper
/// around this function that reads stdin and writes stdout.
#[must_use]
pub fn run(input: &CandidateInput) -> DebianSourcePreparationV1 {
    let mut findings = SourcePreparationFindings {
        debian_directory_present: false,
        control_parsable: false,
        control_source_name: None,
        control_source_name_matches_candidate: false,
        changelog_parsable: false,
        changelog_source_name: None,
        changelog_source_name_matches_candidate: false,
        changelog_version: None,
        changelog_version_matches_candidate: false,
        format_identifiable: false,
        source_format: None,
        rules_executable: false,
    };
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut identity: Option<ChangelogIdentity> = None;

    // §17 #1: `debian/` exists. A missing `debian/` is
    // `verdict: "fail"` (the candidate is not a Debian
    // source package) per §17, distinct from
    // `infrastructure_error` (the tool could not read the
    // workspace at all).
    let Some(workspace_path) = input.workspace_path.as_ref() else {
        return DebianSourcePreparationV1 {
            schema_version: SCHEMA_VERSION,
            report_kind: REPORT_KIND.to_string(),
            candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
            verdict: Verdict::InfrastructureError,
            diagnostics: vec![Diagnostic {
                code: "E_WORKSPACE_PATH_MISSING".to_string(),
                message: "candidate input did not include workspace_path".to_string(),
                path: None,
            }],
            findings,
            identity: None,
        };
    };
    if !workspace_path.is_dir() {
        return DebianSourcePreparationV1 {
            schema_version: SCHEMA_VERSION,
            report_kind: REPORT_KIND.to_string(),
            candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
            verdict: Verdict::InfrastructureError,
            diagnostics: vec![Diagnostic {
                code: "E_WORKSPACE_NOT_DIR".to_string(),
                message: format!(
                    "workspace_path {} is not a directory",
                    workspace_path.display()
                ),
                path: Some(workspace_path.clone()),
            }],
            findings,
            identity: None,
        };
    }

    let debian_dir = workspace_path.join("debian");
    if !debian_dir.is_dir() {
        diagnostics.push(Diagnostic {
            code: "E_DEBIAN_DIR_MISSING".to_string(),
            message: "debian/ does not exist".to_string(),
            path: Some(debian_dir.clone()),
        });
    } else {
        findings.debian_directory_present = true;
    }

    // §17 #2: `debian/control` parseable.
    if findings.debian_directory_present {
        let control_path = debian_dir.join("control");
        match read_control(&control_path) {
            Ok(control) => {
                findings.control_parsable = true;
                findings.control_source_name = Some(control.source.clone());
                findings.control_source_name_matches_candidate =
                    control.source == input.package.name;
                if !findings.control_source_name_matches_candidate {
                    diagnostics.push(Diagnostic {
                        code: "E_SOURCE_NAME_MISMATCH".to_string(),
                        message: format!(
                            "debian/control Source: {} does not match candidate source name {}",
                            control.source, input.package.name
                        ),
                        path: Some(control_path.clone()),
                    });
                }
            }
            Err(PrepError::Infra(msg)) => {
                diagnostics.push(Diagnostic {
                    code: "E_CONTROL_UNREADABLE".to_string(),
                    message: msg,
                    path: Some(control_path.clone()),
                });
            }
        }
    }

    // §17 #3: `debian/changelog` parseable, plus the
    // identity cross-checks (#4 and #5).
    if findings.debian_directory_present {
        let changelog_path = debian_dir.join("changelog");
        match read_changelog(&changelog_path) {
            Ok(entry) => {
                findings.changelog_parsable = true;
                findings.changelog_source_name = Some(entry.source.clone());
                findings.changelog_version = Some(entry.version.clone());
                findings.changelog_source_name_matches_candidate =
                    entry.source == input.package.name;
                findings.changelog_version_matches_candidate =
                    entry.version == input.package.version;
                if !findings.changelog_source_name_matches_candidate {
                    diagnostics.push(Diagnostic {
                        code: "E_CHANGELOG_SOURCE_NAME_MISMATCH".to_string(),
                        message: format!(
                            "debian/changelog first entry source {} does not match candidate \
                             source name {}",
                            entry.source, input.package.name
                        ),
                        path: Some(changelog_path.clone()),
                    });
                }
                if !findings.changelog_version_matches_candidate {
                    diagnostics.push(Diagnostic {
                        code: "E_VERSION_MISMATCH".to_string(),
                        message: format!(
                            "debian/changelog first entry version {} does not match candidate \
                             version {}",
                            entry.version, input.package.version
                        ),
                        path: Some(changelog_path.clone()),
                    });
                }
                identity = Some(ChangelogIdentity {
                    source: entry.source,
                    version: entry.version,
                    distribution: entry.distribution,
                    urgency: entry.urgency,
                    ..ChangelogIdentity::default()
                });
            }
            Err(PrepError::Infra(msg)) => {
                diagnostics.push(Diagnostic {
                    code: "E_CHANGELOG_UNREADABLE".to_string(),
                    message: msg,
                    path: Some(changelog_path.clone()),
                });
            }
        }
    }

    // §17 #6: source format identifiable.
    if findings.debian_directory_present {
        let format_path = debian_dir.join("source").join("format");
        match read_source_format(&format_path) {
            Ok(fmt) => {
                findings.format_identifiable = true;
                findings.source_format = Some(fmt.clone());
            }
            Err(PrepError::Infra(msg)) => {
                diagnostics.push(Diagnostic {
                    code: "E_FORMAT_UNREADABLE".to_string(),
                    message: msg,
                    path: Some(format_path.clone()),
                });
            }
        }
    }

    // §17 informational: `debian/rules` is executable. Not
    // part of the verdict, but recorded so a 1C.2 / 1C.3
    // tool can correlate.
    if findings.debian_directory_present {
        let rules_path = debian_dir.join("rules");
        if rules_path.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                findings.rules_executable = rules_path
                    .metadata()
                    .is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
            }
            #[cfg(not(unix))]
            #[allow(dead_code)]
            {
                findings.rules_executable = true;
            }
        }
    }

    // Aggregate the verdict. §17 says: a mismatch is `Fail`,
    // a parser binary missing is `InfrastructureError`.
    // There is no parser binary in 1C.1 (the parsers are
    // pure-Rust), so the only `InfrastructureError` paths
    // are: missing `workspace_path`, `workspace_path` not
    // a directory, IO error reading a file (which we
    // surface as `E_*_UNREADABLE` and the §17 distinction
    // for unreadable means the verdict is `Fail` because
    // the tool *attempted* the check — the failure is in
    // the source tree, not in the toolchain).
    let verdict = if findings.debian_directory_present
        && findings.control_parsable
        && findings.changelog_parsable
        && findings.format_identifiable
        && findings.control_source_name_matches_candidate
        && findings.changelog_source_name_matches_candidate
        && findings.changelog_version_matches_candidate
    {
        Verdict::Pass
    } else {
        Verdict::Fail
    };

    DebianSourcePreparationV1 {
        schema_version: SCHEMA_VERSION,
        report_kind: REPORT_KIND.to_string(),
        candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
        verdict,
        diagnostics,
        findings,
        identity,
    }
}

/// The result of a successful `debian/control` parse.
struct ControlFields {
    /// The `Source:` field. The package name (not the
    /// `Package:` stanzas — those are binary-package
    /// declarations, not the source name).
    source: String,
}

/// Read and parse `debian/control`, returning the `Source:`
/// field. `deb822-lossless`'s `Deb822` parses the RFC 822
/// dialect `debian/control` uses.
fn read_control(path: &Path) -> Result<ControlFields, PrepError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| PrepError::Infra(format!("cannot read control file: {e}")))?;
    let parsed = Deb822::from_str(&text)
        .map_err(|e| PrepError::Infra(format!("control file is not a valid deb822 stanza: {e}")))?;
    // `debian/control` has one source paragraph at the
    // top, then one paragraph per binary package; the
    // source name is in the first paragraph.
    let first_paragraph = parsed
        .paragraphs()
        .next()
        .ok_or_else(|| PrepError::Infra("control file has no paragraphs".to_string()))?;
    let source = first_paragraph
        .get("Source")
        .ok_or_else(|| PrepError::Infra("control file has no Source: field".to_string()))?
        .trim()
        .to_string();
    if source.is_empty() {
        return Err(PrepError::Infra(
            "control file has an empty Source: field".to_string(),
        ));
    }
    Ok(ControlFields { source })
}

/// The first entry of a parsed `debian/changelog`.
struct ChangelogEntry {
    source: String,
    version: String,
    distribution: String,
    urgency: String,
}

/// Read and parse `debian/changelog`, returning the first
/// (topmost, most-recent) entry. The format is a degenerate
/// RFC 822: the first non-blank line is
///
/// ```text
/// <source> (<version>) <distribution>; urgency=<urgency>
/// ```
///
/// followed by a free-form change body, then a blank line, then
/// the next entry, and so on. `dpkg-parsechangelog` accepts
/// the same grammar. We hand-roll it because pulling the full
/// `devscripts` parser in is more dependency than 1C.1
/// justifies (PHASE-1.md §16's "focused parser crates"
/// preference), and the format is simple enough that the
/// pure-Rust parser is what every other Debian-aware Rust
/// tool does.
fn read_changelog(path: &Path) -> Result<ChangelogEntry, PrepError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| PrepError::Infra(format!("cannot read changelog: {e}")))?;
    parse_changelog_first_entry(&text)
        .ok_or_else(|| PrepError::Infra("changelog has no parseable first entry".to_string()))
}

/// Parse the first-entry header from a `debian/changelog`
/// body. Pure function, exported for unit tests.
fn parse_changelog_first_entry(body: &str) -> Option<ChangelogEntry> {
    // The first non-blank line is the header. `dpkg-parsechangelog`
    // is more permissive (it accepts a leading blank line,
    // a leading "urgency=..." line, etc.), but the standard
    // form begins with `<source> (<version>) ...`.
    let header = body.lines().find(|line| !line.trim().is_empty())?;
    parse_changelog_header(header)
}

/// Parse a single changelog header line.
///
///   foo (1.2.3-1) unstable; urgency=medium
///
/// is the canonical form. The line is split into three parts
/// at the matching parentheses and the closing
/// `; distribution; urgency=X` separator.
fn parse_changelog_header(line: &str) -> Option<ChangelogEntry> {
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    if close <= open + 1 {
        return None;
    }
    let source = line[..open].trim().to_string();
    let version = line[open + 1..close].trim().to_string();
    if source.is_empty() || version.is_empty() {
        return None;
    }
    // After the closing paren: ` <distribution>; urgency=<X>`.
    // A header without the `; urgency=X` separator is
    // tolerated (real Debian packages always have an
    // urgency, but the parser is robust to missing one);
    // the entire after-closing-paren string is treated
    // as the distribution and urgency is empty.
    let after = line[close + 1..].trim();
    let (distribution, urgency) = parse_changelog_trailer(after);
    Some(ChangelogEntry {
        source,
        version,
        distribution,
        urgency,
    })
}

/// Parse `<distribution>; urgency=<X>` and any subsequent
/// fields. Returns `(distribution, urgency)`. The trailer
/// may have a trailing space, then a `Maintainer:` /
/// `Timestamp:` block on the next line(s); we ignore those.
/// Returns `(distribution, urgency)` even when the
/// `; urgency=X` is absent; `urgency` is empty in that
/// case. `distribution` is the entire after-closing-paren
/// string trimmed, which the rest of the tool treats as
/// the target suite name.
fn parse_changelog_trailer(trailer: &str) -> (String, String) {
    let (distribution_part, urgency) = match trailer.find(';') {
        Some(semicolon) => {
            let distribution = trailer[..semicolon].trim().to_string();
            let after = &trailer[semicolon + 1..];
            // `urgency=medium` is the only field we care
            // about; other fields like `binary-only=yes`
            // are tolerated and ignored.
            let urgency = after
                .split_whitespace()
                .find_map(|kv| kv.strip_prefix("urgency=").map(str::to_string))
                .unwrap_or_default();
            (distribution, urgency)
        }
        None => (trailer.trim().to_string(), String::new()),
    };
    (distribution_part, urgency)
}

/// Read `debian/source/format` and return the trimmed value
/// of the single line.
fn read_source_format(path: &Path) -> Result<String, PrepError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| PrepError::Infra(format!("cannot read source/format: {e}")))?;
    let value = text.trim().to_string();
    if value.is_empty() {
        return Err(PrepError::Infra(
            "debian/source/format is empty".to_string(),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header of an `unstable; urgency=medium` changelog
    /// entry is the canonical form every Debian package uses.
    #[test]
    fn changelog_header_canonical_form() {
        let parsed = parse_changelog_header("example (1.0.0-1) unstable; urgency=medium").unwrap();
        assert_eq!(parsed.source, "example");
        assert_eq!(parsed.version, "1.0.0-1");
        assert_eq!(parsed.distribution, "unstable");
        assert_eq!(parsed.urgency, "medium");
    }

    /// The header of a `bookworm; urgency=low` entry uses a
    /// different distribution and urgency.
    #[test]
    fn changelog_header_bookworm_urgency_low() {
        let parsed = parse_changelog_header("foo (2.0.0-1+deb12u1) bookworm; urgency=low").unwrap();
        assert_eq!(parsed.source, "foo");
        assert_eq!(parsed.version, "2.0.0-1+deb12u1");
        assert_eq!(parsed.distribution, "bookworm");
        assert_eq!(parsed.urgency, "low");
    }

    /// BinNMU versions like `1.0-1+b1` are valid in the
    /// version field. The parser does not need to validate
    /// the version (the `debversion` crate does that on the
    /// candidate side); it just has to round-trip the
    /// parenthesised value.
    #[test]
    fn changelog_header_bin_nmu_version() {
        let parsed = parse_changelog_header("foo (1.0-1+b1) unstable; urgency=medium").unwrap();
        assert_eq!(parsed.version, "1.0-1+b1");
    }

    /// The first non-blank line is the header.
    #[test]
    fn changelog_first_entry_skips_leading_blank_lines() {
        let body = "\n\nexample (1.0.0-1) unstable; urgency=medium\n\n  * First change.\n";
        let parsed = parse_changelog_first_entry(body).unwrap();
        assert_eq!(parsed.source, "example");
        assert_eq!(parsed.version, "1.0.0-1");
    }

    /// A header with no `; urgency=X` trailer parses with
    /// `urgency = ""`, which the report serialises as the
    /// empty string. Real Debian packages always have an
    /// urgency, but the parser is robust to missing one.
    #[test]
    fn changelog_header_missing_urgency() {
        let parsed = parse_changelog_header("foo (1.0-1) unstable").unwrap();
        assert_eq!(parsed.distribution, "unstable");
        assert_eq!(parsed.urgency, "");
    }
}
