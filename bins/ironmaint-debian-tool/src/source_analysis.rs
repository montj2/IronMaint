//! `debian.inspect.source_analysis` — the 1C.2 real check.
//!
//! PHASE-1.md §18 enumerates the field set the
//! comprehensive `DebianSourceReportV1` carries. The 1C.1
//! parser stops at the first paragraph of `debian/control`
//! (only `Source:`) and the first changelog entry's header
//! (only source / version / distribution / urgency). The
//! 1C.2 parser walks the full `debian/control` (source
//! paragraph + every binary paragraph), the full first
//! changelog entry (closing-paren trailer + Maintainer +
//! Timestamp), `debian/copyright` (DEP-5), `debian/watch`
//! v4, `debian/patches/series`, the DEP-3 header of every
//! patch, `debian/tests/control` (DEP-8), and the presence
//! + executable bit of `debian/rules` and the four
//! maintainer scripts.
//!
//! Like the 1C.1 tool, this is pure-Rust (no shell-out to
//! `dpkg-parsechangelog` etc.) per §16's "focused parser
//! crates" preference. `deb822-lossless` (already a dep)
//! covers the RFC 822-ish grammars; the watch v4 grammar,
//! the DEP-3 patch header grammar, the
//! `debian/patches/series` grammar, and the extended
//! changelog trailer are small hand-rolled parsers.
//!
//! ## The RED/GREEN split
//!
//! The RED commit's `run()` returns a *minimal*
//! `DebianSourceReportV1` — just the §17 tri-state, the
//! candidate identity, and the first-entry changelog
//! block. The integration and unit tests assert the
//! comprehensive fields are populated; the minimal struct
//! cannot satisfy them; the tests fail. The GREEN commit
//! extends the struct with the §18 sub-structs and rewrites
//! `run()` to populate them. The §97 schema-snapshot tool
//! (`xtask verify-schemas`) catches the GREEN-side drift:
//! if the GREEN commit forgets to regenerate the snapshot,
//! `verify-schemas` reports the mismatch and the gate
//! fails. The first sub-step of GREEN is therefore
//! `cargo run -p xtask -- verify-schemas --write`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::report::{
    ChangelogIdentity, DebianSourceReportV1, Diagnostic, SOURCE_ANALYSIS_REPORT_KIND,
    SOURCE_ANALYSIS_SCHEMA_VERSION, Verdict,
};

/// A failed attempt to read or parse the workspace tree.
///
/// The public surface is a [`DebianSourceReportV1`] —
/// every error path produces one, with `verdict` set
/// appropriately. This enum is a private intermediate
/// only used to thread the right `verdict` through the
/// fallible parsing steps.
#[derive(Debug)]
enum AnalysisError {
    /// A file the tool expected was absent or unreadable.
    /// Maps to `verdict: "infrastructure_error"` per §17.
    Infra(String),
}

/// The runtime's `ironmaint.candidate_input.v3` payload (PHASE-1.md §7).
///
/// The 1C.2 extension over v2 adds `commit` and `tree` so
/// the report can carry the candidate's Git identity
/// (PHASE-1.md §18 lines 842-846). The `serde(default)`
/// attributes make the v2 payload continue to parse
/// (without `commit` / `tree`); the 1C.2 tool reads
/// `commit` / `tree` and surfaces them on the report.
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
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub tree: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
}

/// Run the 1C.2 check against `workspace_path` for the
/// candidate whose `source_name` and `version` are the
/// values the runtime passed in.
///
/// Public so `tests/source_analysis.rs` and the daemon's
/// integration test can drive the logic without a real
/// subprocess. The `main.rs` entry point is a thin wrapper
/// around this function that reads stdin and writes stdout.
///
/// **RED-state stub.** The 1C.2 RED commit's body
/// returns a *minimal* `DebianSourceReportV1` —
/// the §17 tri-state, the candidate identity, and the
/// first-entry changelog block. The comprehensive §18
/// fields are absent; the integration and unit tests
/// fail because they assert those fields are populated.
/// The GREEN commit replaces this body with the full
/// parser composition.
#[must_use]
pub fn run(input: &CandidateInput) -> DebianSourceReportV1 {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut identity: Option<ChangelogIdentity> = None;

    // RED: read the changelog header (carried over from 1C.1)
    // and surface the candidate's Git identity so the report
    // is at least structurally valid. The comprehensive
    // parser composition is the GREEN commit's work.
    let workspace_path = input.workspace_path.as_ref();

    let verdict = if let Some(workspace_path) = workspace_path {
        if !workspace_path.is_dir() {
            return DebianSourceReportV1 {
                schema_version: SOURCE_ANALYSIS_SCHEMA_VERSION,
                report_kind: SOURCE_ANALYSIS_REPORT_KIND.to_string(),
                candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
                candidate_commit: input.commit.clone().unwrap_or_default(),
                candidate_tree: input.tree.clone().unwrap_or_default(),
                verdict: Verdict::InfrastructureError,
                diagnostics: vec![Diagnostic {
                    code: "E_WORKSPACE_NOT_DIR".to_string(),
                    message: format!(
                        "workspace_path {} is not a directory",
                        workspace_path.display()
                    ),
                    path: Some(workspace_path.clone()),
                }],
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
            // Read the changelog header; populate `identity`.
            let changelog_path = debian_dir.join("changelog");
            match read_changelog_header(&changelog_path) {
                Ok(entry) => {
                    identity = Some(ChangelogIdentity {
                        source: entry.source,
                        version: entry.version,
                        distribution: entry.distribution,
                        urgency: entry.urgency,
                        ..ChangelogIdentity::default()
                    });
                }
                Err(AnalysisError::Infra(msg)) => {
                    diagnostics.push(Diagnostic {
                        code: "E_CHANGELOG_UNREADABLE".to_string(),
                        message: msg,
                        path: Some(changelog_path.clone()),
                    });
                }
            }
        }
        // §17 aggregation: the RED-state stub produces Pass
        // when there are no diagnostics; the GREEN commit
        // adds the comprehensive fields and re-aggregates.
        if diagnostics.is_empty() {
            Verdict::Pass
        } else {
            Verdict::Fail
        }
    } else {
        return DebianSourceReportV1 {
            schema_version: SOURCE_ANALYSIS_SCHEMA_VERSION,
            report_kind: SOURCE_ANALYSIS_REPORT_KIND.to_string(),
            candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
            candidate_commit: input.commit.clone().unwrap_or_default(),
            candidate_tree: input.tree.clone().unwrap_or_default(),
            verdict: Verdict::InfrastructureError,
            diagnostics: vec![Diagnostic {
                code: "E_WORKSPACE_PATH_MISSING".to_string(),
                message: "candidate input did not include workspace_path".to_string(),
                path: None,
            }],
            identity: None,
        };
    };

    DebianSourceReportV1 {
        schema_version: SOURCE_ANALYSIS_SCHEMA_VERSION,
        report_kind: SOURCE_ANALYSIS_REPORT_KIND.to_string(),
        candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
        candidate_commit: input.commit.clone().unwrap_or_default(),
        candidate_tree: input.tree.clone().unwrap_or_default(),
        verdict,
        diagnostics,
        identity,
    }
}

/// The first entry of a parsed `debian/changelog`. Same
/// shape as 1C.1's; duplicated here to keep the
/// 1C.2 module self-contained.
struct ChangelogEntry {
    source: String,
    version: String,
    distribution: String,
    urgency: String,
}

fn read_changelog_header(path: &Path) -> Result<ChangelogEntry, AnalysisError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| AnalysisError::Infra(format!("cannot read changelog: {e}")))?;
    parse_changelog_first_entry(&text)
        .ok_or_else(|| AnalysisError::Infra("changelog has no parseable first entry".to_string()))
}

fn parse_changelog_first_entry(body: &str) -> Option<ChangelogEntry> {
    let header = body.lines().find(|line| !line.trim().is_empty())?;
    parse_changelog_header(header)
}

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
    let after = line[close + 1..].trim();
    let (distribution, urgency) = parse_changelog_trailer(after);
    Some(ChangelogEntry {
        source,
        version,
        distribution,
        urgency,
    })
}

fn parse_changelog_trailer(trailer: &str) -> (String, String) {
    let (distribution_part, urgency) = match trailer.find(';') {
        Some(semicolon) => {
            let distribution = trailer[..semicolon].trim().to_string();
            let after = &trailer[semicolon + 1..];
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use deb822_lossless::Deb822;

    /// The header of an `unstable; urgency=medium` changelog
    /// entry is the canonical form.
    #[test]
    fn changelog_header_canonical_form() {
        let parsed = parse_changelog_header("example (1.0.0-1) unstable; urgency=medium").unwrap();
        assert_eq!(parsed.source, "example");
        assert_eq!(parsed.version, "1.0.0-1");
        assert_eq!(parsed.distribution, "unstable");
        assert_eq!(parsed.urgency, "medium");
    }

    /// A header with no `; urgency=X` trailer parses with
    /// `urgency = ""`.
    #[test]
    fn changelog_header_missing_urgency() {
        let parsed = parse_changelog_header("foo (1.0-1) unstable").unwrap();
        assert_eq!(parsed.distribution, "unstable");
        assert_eq!(parsed.urgency, "");
    }

    /// Round-trip `Deb822` for a multi-paragraph `debian/control`
    /// is what 1C.2's GREEN body will exercise; the unit test
    /// here pins the parser smoke check.
    #[test]
    fn deb822_parses_multi_paragraph_control() {
        let text = "Source: example\nMaintainer: Test <test@example.invalid>\nSection: utils\n\n\
                    Package: example\nArchitecture: any\nDescription: trivial example package\n";
        let parsed = Deb822::from_str(text).unwrap();
        let paragraphs: Vec<_> = parsed.paragraphs().collect();
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(paragraphs[0].get("Source").unwrap().trim(), "example");
        assert_eq!(paragraphs[1].get("Package").unwrap().trim(), "example");
    }
}
