//! `debian.inspect.source_analysis` — the 1C.2 real check.
//!
//! PHASE-1.md §18 enumerates the field set the
//! comprehensive `DebianSourceReportV1` carries. The
//! 1C.1 parser stops at the first paragraph of
//! `debian/control` (only `Source:`) and the first
//! changelog entry's header (only source / version /
//! distribution / urgency). The 1C.2 parser walks the
//! full §18 surface:
//!
//! - `debian/changelog` — first entry's full trailer
//!   (Maintainer: + Date: lines)
//! - `debian/control` — source paragraph (Maintainer,
//!   Uploaders, Standards-Version, Rules-Requires-Root,
//!   Section, Priority, Homepage, Vcs-Git, Vcs-Browser,
//!   Build-Depends, Build-Depends-Indep, Build-Conflicts,
//!   debhelper-compat) + every binary paragraph (Package,
//!   Architecture, Multi-Arch, Depends, Provides, Breaks,
//!   Conflicts, Replaces, Maintainer, Description)
//! - `debian/copyright` (DEP-5) — Format declaration,
//!   Files stanza count, License identifier set
//! - `debian/source/format` — the canonical value
//! - `debian/watch` v4 — version, URL pattern, pgpsig
//! - `debian/patches/series` + each patch's DEP-3 header
//! - `debian/tests/control` (DEP-8) — Tests, Restrictions,
//!   Features, Depends
//! - `debian/rules` — executable bit, dh-style detection,
//!   override target list
//! - `debian/{preinst,postinst,prerm,postrm,triggers}` —
//!   presence + executable bit
//! - `debian/gbp.conf` + `debian/gbp-dch.conf` —
//!   presence + branch settings
//!
//! Like 1C.1, this is pure-Rust (no shell-out to
//! `dpkg-parsechangelog` etc.) per §16's "focused parser
//! crates" preference. `deb822-lossless` (already a dep)
//! covers the RFC 822-ish grammars; the watch v4 grammar,
//! the DEP-3 patch header grammar, the
//! `debian/patches/series` grammar, the extended changelog
//! trailer, the build-depends comma-split, the dh-style
//! detection, and the gbp.conf INI parse are small
//! hand-rolled parsers.
//!
//! ## The RED/GREEN split
//!
//! The 1C.2 RED commit's `run()` returned a *minimal*
//! `DebianSourceReportV1` — just the §17 tri-state, the
//! candidate identity, and the first-entry changelog
//! block. The 1C.2 GREEN commit replaces the minimal
//! struct with the §18 comprehensive shape and rewrites
//! `run()` to populate it via the parser composition
//! above. The §97 schema-snapshot tool (`xtask
//! verify-schemas`) catches the GREEN-side drift: the
//! GREEN commit's first sub-step is `cargo run -p xtask
//! -- verify-schemas --write` to regenerate
//! `schemas/DebianSourceReportV1.json` against the new
//! struct.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Deserialize;

use crate::report::{
    Autopkgtest, BinaryPackage, BuildMetadata, ChangelogIdentity, CopyrightInfo,
    DebianSourceReportV1, Diagnostic, GbpBranchSettings, GbpConfig, MaintainerInfo,
    MaintainerScripts, NamedScript, Patch, Provenance, RulesInfo, SOURCE_ANALYSIS_REPORT_KIND,
    SOURCE_ANALYSIS_SCHEMA_VERSION, ScriptPresence, SourceFormat, SourceMetadata, Verdict,
    WatchConfig,
};

/// The runtime's `ironmaint.candidate_input.v3` payload
/// (PHASE-1.md §7).
///
/// The 1C.2 extension over v2 adds `commit` and `tree` so
/// the report's `provenance` block can carry the
/// candidate's Git identity (PHASE-1.md §18 lines
/// 842-846). The `serde(default)` attributes make the v2
/// payload continue to parse (without `commit` / `tree`);
/// the 1C.2 tool reads `commit` / `tree` and surfaces
/// them on the report's `provenance`.
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

// =============================================================================
// §17 aggregation.
// =============================================================================

/// A failed attempt to read or parse the workspace tree.
///
/// The public surface is a [`DebianSourceReportV1`] —
/// every error path produces one, with `verdict` set
/// appropriately. This enum is a private intermediate
/// only used to thread the right `verdict` through the
/// fallible parsing steps.
#[derive(Debug)]
enum AnalysisError {
    /// A file the tool expected was absent or
    /// unreadable. Maps to `verdict: "infrastructure_error"`
    /// per §17.
    Infra(String),
    /// A field was malformed in a way that prevents
    /// establishing the fact at all (vs. merely
    /// recording a finding). The most common case
    /// is `debian/control` or `debian/changelog`
    /// being unparseable — the parser cannot even
    /// tell what the package is. Maps to
    /// `verdict: "fail"` per §17.
    Fail(String),
}

// =============================================================================
// Top-level `run`.
// =============================================================================

/// Run the 1C.2 check against `workspace_path` for the
/// candidate whose `source_name` and `version` are the
/// values the runtime passed in.
///
/// Public so `tests/source_analysis.rs` and the daemon's
/// integration test can drive the logic without a real
/// subprocess. The `main.rs` entry point is a thin wrapper
/// around this function that reads stdin and writes stdout.
#[must_use]
pub fn run(input: &CandidateInput) -> DebianSourceReportV1 {
    let workspace_path = input.workspace_path.clone();

    let workspace_path = match workspace_path {
        Some(p) => p,
        None => {
            return infrastructure_error_report(
                input,
                Diagnostic {
                    code: "E_WORKSPACE_PATH_MISSING".to_string(),
                    message: "candidate input did not include workspace_path".to_string(),
                    path: None,
                },
            );
        }
    };

    if !workspace_path.is_dir() {
        return infrastructure_error_report(
            input,
            Diagnostic {
                code: "E_WORKSPACE_NOT_DIR".to_string(),
                message: format!(
                    "workspace_path {} is not a directory",
                    workspace_path.display()
                ),
                path: Some(workspace_path.clone()),
            },
        );
    }

    let debian_dir = workspace_path.join("debian");
    if !debian_dir.is_dir() {
        return DebianSourceReportV1 {
            schema_version: SOURCE_ANALYSIS_SCHEMA_VERSION,
            report_kind: SOURCE_ANALYSIS_REPORT_KIND.to_string(),
            verdict: Verdict::Fail,
            diagnostics: vec![Diagnostic {
                code: "E_DEBIAN_DIR_MISSING".to_string(),
                message: "debian/ does not exist".to_string(),
                path: Some(debian_dir.clone()),
            }],
            identity: None,
            maintainers: MaintainerInfo::default(),
            source_metadata: SourceMetadata::default(),
            build_metadata: BuildMetadata::default(),
            binary_packages: Vec::new(),
            source_format: SourceFormat::OtherLegacy(String::new()),
            patches: Vec::new(),
            tests: Vec::new(),
            watch: None,
            copyright: CopyrightInfo::default(),
            maintainer_scripts: MaintainerScripts::default(),
            rules: RulesInfo::default(),
            gbp: GbpConfig::default(),
            provenance: provenance_from(input),
        };
    }

    // §17 tri-state verdict: a hard-fail during the
    // initial parse (control, changelog) is a Fail, not
    // an InfrastructureError — the tool *could* read
    // the workspace, the contents just don't make
    // sense. The §17 "parser binary missing →
    // InfrastructureError" rule applies to subprocess
    // shells, not to in-process Rust parsers.
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // 1. `debian/changelog` — first entry, full trailer.
    let changelog_path = debian_dir.join("changelog");
    let identity = match read_changelog_full(&changelog_path) {
        Ok(id) => Some(id),
        Err(AnalysisError::Infra(msg)) => {
            diagnostics.push(Diagnostic {
                code: "E_CHANGELOG_UNREADABLE".to_string(),
                message: msg,
                path: Some(changelog_path.clone()),
            });
            None
        }
        Err(AnalysisError::Fail(msg)) => {
            diagnostics.push(Diagnostic {
                code: "E_CHANGELOG_UNPARSABLE".to_string(),
                message: msg,
                path: Some(changelog_path.clone()),
            });
            None
        }
    };

    // 2. `debian/control` — source paragraph + binary paragraphs.
    let control_path = debian_dir.join("control");
    let (maintainers, source_metadata, build_metadata, binary_packages) =
        match read_control(&control_path) {
            Ok(parts) => parts,
            Err(AnalysisError::Infra(msg)) => {
                diagnostics.push(Diagnostic {
                    code: "E_CONTROL_UNREADABLE".to_string(),
                    message: msg,
                    path: Some(control_path.clone()),
                });
                (
                    MaintainerInfo::default(),
                    SourceMetadata::default(),
                    BuildMetadata::default(),
                    Vec::new(),
                )
            }
            Err(AnalysisError::Fail(msg)) => {
                diagnostics.push(Diagnostic {
                    code: "E_CONTROL_UNPARSABLE".to_string(),
                    message: msg,
                    path: Some(control_path.clone()),
                });
                (
                    MaintainerInfo::default(),
                    SourceMetadata::default(),
                    BuildMetadata::default(),
                    Vec::new(),
                )
            }
        };

    // 3. `debian/source/format` — the canonical value.
    let source_format = read_source_format(&debian_dir.join("source/format"));

    // 4. `debian/copyright` (DEP-5).
    let copyright = read_copyright(&debian_dir.join("copyright"));

    // 5. `debian/watch` v4.
    let watch = read_watch(&debian_dir.join("watch"));

    // 6. `debian/patches/series` + each patch's DEP-3 header.
    let (patches, patch_diagnostics) = read_patches(&debian_dir.join("patches"));
    diagnostics.extend(patch_diagnostics);

    // 7. `debian/tests/control` (DEP-8).
    let tests = read_tests_control(&debian_dir.join("tests/control"));

    // 8. `debian/rules` — executable bit, dh-style, overrides.
    let rules = read_rules(&debian_dir.join("rules"));

    // 9. Maintainer scripts.
    let maintainer_scripts = read_maintainer_scripts(&debian_dir);

    // 10. GBP config.
    let gbp = read_gbp_config(&debian_dir);

    // §17 verdict: a Fail diagnostic on a §18
    // hard-requirement (changelog unparsable, control
    // unparsable, missing patches referenced in
    // series) means the verdict is Fail. Otherwise
    // (everything parsed) the verdict is Pass.
    let verdict = if diagnostics.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Fail
    };

    DebianSourceReportV1 {
        schema_version: SOURCE_ANALYSIS_SCHEMA_VERSION,
        report_kind: SOURCE_ANALYSIS_REPORT_KIND.to_string(),
        verdict,
        diagnostics,
        identity,
        maintainers,
        source_metadata,
        build_metadata,
        binary_packages,
        source_format,
        patches,
        tests,
        watch,
        copyright,
        maintainer_scripts,
        rules,
        gbp,
        provenance: provenance_from(input),
    }
}

fn infrastructure_error_report(
    input: &CandidateInput,
    diagnostic: Diagnostic,
) -> DebianSourceReportV1 {
    DebianSourceReportV1 {
        schema_version: SOURCE_ANALYSIS_SCHEMA_VERSION,
        report_kind: SOURCE_ANALYSIS_REPORT_KIND.to_string(),
        verdict: Verdict::InfrastructureError,
        diagnostics: vec![diagnostic],
        identity: None,
        maintainers: MaintainerInfo::default(),
        source_metadata: SourceMetadata::default(),
        build_metadata: BuildMetadata::default(),
        binary_packages: Vec::new(),
        source_format: SourceFormat::OtherLegacy(String::new()),
        patches: Vec::new(),
        tests: Vec::new(),
        watch: None,
        copyright: CopyrightInfo::default(),
        maintainer_scripts: MaintainerScripts::default(),
        rules: RulesInfo::default(),
        gbp: GbpConfig::default(),
        provenance: provenance_from(input),
    }
}

fn provenance_from(input: &CandidateInput) -> Provenance {
    Provenance {
        candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
        candidate_commit: input.commit.clone().unwrap_or_default(),
        candidate_tree: input.tree.clone().unwrap_or_default(),
        inspector_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

// =============================================================================
// `debian/changelog` — first entry, full trailer.
// =============================================================================

/// Read the first entry of `debian/changelog` and
/// thread the closing-paren trailer (Maintainer: + Date:
/// lines) into a `ChangelogIdentity`.
///
/// The trailer lives below the closing `) <distribution>;
/// urgency=<level>` line, separated by a blank line. The
/// parser walks line-by-line, finds the header, then
/// collects the `Maintainer:` and `Date:` lines that
/// follow (until EOF or a non-trailer blank-line run).
fn read_changelog_full(path: &Path) -> Result<ChangelogIdentity, AnalysisError> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| AnalysisError::Infra(format!("cannot read changelog: {e}")))?;
    parse_changelog_full(&body)
}

fn parse_changelog_full(body: &str) -> Result<ChangelogIdentity, AnalysisError> {
    let header_line = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| AnalysisError::Fail("changelog is empty".to_string()))?;
    let header = parse_changelog_header(header_line).ok_or_else(|| {
        AnalysisError::Fail(format!("changelog header is malformed: {header_line:?}"))
    })?;

    // The Debian changelog format is:
    //
    //   <header>
    //
    //     * changelog entry
    //     * more changelog
    //     -- long-line continuation
    //
    //    -- Maintainer Name <email>  Date
    //
    // The trailer marker is the line that starts with
    // `--` (after optional leading whitespace). On
    // that line, the maintainer and the date are
    // separated by *two or more spaces*; the
    // maintainer is everything between the `--` and
    // the first run of two or more spaces, and the
    // date is the rest.
    //
    // `dpkg-parsechangelog` reports the trailer as
    // `Maintainer:` and `Date:` fields, which is the
    // shape §18's identity trailer carries.

    let mut maintainer: Option<String> = None;
    let mut timestamp: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim_start();
        // The trailer marker is a line whose first
        // non-whitespace characters are `--`.
        if let Some(rest) = trimmed.strip_prefix("--") {
            // Skip the space(s) between `--` and the
            // maintainer name.
            let rest = rest.trim_start();
            // Split on the two-or-more-space
            // separator. The first run is the
            // maintainer; the rest is the date.
            let mut split_at: Option<usize> = None;
            let bytes = rest.as_bytes();
            let mut i = 0;
            while i + 1 < bytes.len() {
                if bytes[i] == b' ' && bytes[i + 1] == b' ' {
                    split_at = Some(i);
                    break;
                }
                i += 1;
            }
            let (m, d) = match split_at {
                Some(idx) => (&rest[..idx], rest[idx..].trim()),
                None => (rest, ""),
            };
            let m = m.trim();
            if !m.is_empty() {
                maintainer = Some(m.to_string());
            }
            if !d.is_empty() {
                timestamp = Some(d.to_string());
            }
            break;
        }
    }

    Ok(ChangelogIdentity {
        source: header.source,
        version: header.version,
        distribution: header.distribution,
        urgency: header.urgency,
        maintainer,
        timestamp,
    })
}

struct ChangelogHeader {
    source: String,
    version: String,
    distribution: String,
    urgency: String,
}

fn parse_changelog_header(line: &str) -> Option<ChangelogHeader> {
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
    Some(ChangelogHeader {
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

// =============================================================================
// `debian/control` — source paragraph + binary paragraphs.
// =============================================================================

fn read_control(
    path: &Path,
) -> Result<
    (
        MaintainerInfo,
        SourceMetadata,
        BuildMetadata,
        Vec<BinaryPackage>,
    ),
    AnalysisError,
> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| AnalysisError::Infra(format!("cannot read control: {e}")))?;
    parse_control(&body)
}

fn parse_control(
    body: &str,
) -> Result<
    (
        MaintainerInfo,
        SourceMetadata,
        BuildMetadata,
        Vec<BinaryPackage>,
    ),
    AnalysisError,
> {
    use deb822_lossless::Deb822;

    let parsed = Deb822::from_str(body)
        .map_err(|e| AnalysisError::Fail(format!("control is not valid deb822: {e}")))?;
    let paragraphs: Vec<_> = parsed.paragraphs().collect();
    if paragraphs.is_empty() {
        return Err(AnalysisError::Fail("control has no paragraphs".to_string()));
    }

    let source = &paragraphs[0];
    if source.get("Source").is_none() {
        return Err(AnalysisError::Fail(
            "source paragraph has no Source field".to_string(),
        ));
    }

    let maintainers = MaintainerInfo {
        maintainer: field(source, "Maintainer"),
        uploaders: match opt_field(source, "Uploaders") {
            Some(s) => split_comma_list(&s),
            None => Vec::new(),
        },
    };

    let source_metadata = SourceMetadata {
        standards_version: opt_field(source, "Standards-Version"),
        rules_requires_root: opt_field(source, "Rules-Requires-Root"),
        section: opt_field(source, "Section"),
        priority: opt_field(source, "Priority"),
        homepage: opt_field(source, "Homepage"),
        vcs_git: opt_field(source, "Vcs-Git"),
        vcs_browser: opt_field(source, "Vcs-Browser"),
    };

    let build_depends = match opt_field(source, "Build-Depends") {
        Some(s) => split_comma_list(&s),
        None => Vec::new(),
    };
    let build_depends_indep = match opt_field(source, "Build-Depends-Indep") {
        Some(s) => split_comma_list(&s),
        None => Vec::new(),
    };
    let build_conflicts = match opt_field(source, "Build-Conflicts") {
        Some(s) => split_comma_list(&s),
        None => Vec::new(),
    };
    let debhelper_compat = build_depends
        .iter()
        .find_map(|entry| parse_debhelper_compat(entry));
    let build_metadata = BuildMetadata {
        build_depends,
        build_depends_indep,
        build_conflicts,
        debhelper_compat,
    };

    let binary_packages: Vec<BinaryPackage> = paragraphs
        .iter()
        .skip(1)
        .filter_map(|para| -> Option<BinaryPackage> {
            let name = para.get("Package")?.trim().to_string();
            let architecture = para
                .get("Architecture")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "any".to_string());
            Some(BinaryPackage {
                name,
                architecture,
                multi_arch: opt_field(para, "Multi-Arch"),
                depends: match opt_field(para, "Depends") {
                    Some(s) => split_comma_list(&s),
                    None => Vec::new(),
                },
                provides: match opt_field(para, "Provides") {
                    Some(s) => split_comma_list(&s),
                    None => Vec::new(),
                },
                breaks: match opt_field(para, "Breaks") {
                    Some(s) => split_comma_list(&s),
                    None => Vec::new(),
                },
                conflicts: match opt_field(para, "Conflicts") {
                    Some(s) => split_comma_list(&s),
                    None => Vec::new(),
                },
                replaces: match opt_field(para, "Replaces") {
                    Some(s) => split_comma_list(&s),
                    None => Vec::new(),
                },
                maintainer: opt_field(para, "Maintainer"),
                description: opt_field(para, "Description"),
            })
        })
        .collect();

    Ok((
        maintainers,
        source_metadata,
        build_metadata,
        binary_packages,
    ))
}

/// Read the value of a `Field:` line, returning an
/// empty string when the field is absent. The empty
/// string is filtered out by serde's
/// `skip_serializing_if = "String::is_empty"` on the
/// target field, so a missing control field
/// round-trips as the field being absent in the
/// report.
fn field(para: &deb822_lossless::Paragraph, key: &str) -> String {
    match para.get(key) {
        Some(v) => v.trim().to_string(),
        None => String::new(),
    }
}

/// Read the value of a `Field:` line, returning
/// `None` when the field is absent or empty. Use
/// this for fields that are genuinely optional —
/// `Some("")` and `None` both round-trip to `None` on
/// the wire.
fn opt_field(para: &deb822_lossless::Paragraph, key: &str) -> Option<String> {
    let v = field(para, key);
    if v.is_empty() { None } else { Some(v) }
}

/// Split an RFC 822 comma-separated alternation list
/// into individual entries, trimming whitespace. The
/// commas inside parentheses (version pins) are
/// preserved — the entries are split on the top-level
/// `,` only.
fn split_comma_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0_i32;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    out.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    out
}

/// Parse `debhelper-compat (= N)` and return the `N`
/// as a `u32`. Returns `None` for any other entry or
/// for a malformed version pin.
fn parse_debhelper_compat(entry: &str) -> Option<u32> {
    let entry = entry.trim();
    if !entry.starts_with("debhelper-compat") {
        return None;
    }
    let paren_open = entry.find('(')?;
    let paren_close = entry[paren_open..].find(')')? + paren_open;
    let inside = entry[paren_open + 1..paren_close].trim();
    let eq = inside.find('=')?;
    let version = inside[eq + 1..].trim();
    version.parse::<u32>().ok()
}

// =============================================================================
// `debian/source/format`.
// =============================================================================

fn read_source_format(path: &Path) -> SourceFormat {
    let Ok(body) = std::fs::read_to_string(path) else {
        return SourceFormat::OtherLegacy(String::new());
    };
    let trimmed = body.trim().to_string();
    match trimmed.as_str() {
        "3.0 (quilt)" => SourceFormat::ThreeZeroQuilt,
        "3.0 (native)" => SourceFormat::ThreeZeroNative,
        _ => SourceFormat::OtherLegacy(trimmed),
    }
}

// =============================================================================
// `debian/copyright` (DEP-5).
// =============================================================================

fn read_copyright(path: &Path) -> CopyrightInfo {
    let Ok(body) = std::fs::read_to_string(path) else {
        return CopyrightInfo::default();
    };
    parse_copyright(&body)
}

fn parse_copyright(body: &str) -> CopyrightInfo {
    use deb822_lossless::Deb822;

    let Ok(parsed) = Deb822::from_str(body) else {
        return CopyrightInfo::default();
    };
    let paragraphs: Vec<_> = parsed.paragraphs().collect();
    let first = paragraphs.first();
    let format_declaration = first.and_then(|p| opt_field(p, "Format"));
    let machine_readable = format_declaration
        .as_deref()
        .map(|s| s.starts_with("http://") || s.starts_with("https://"))
        .unwrap_or(false);

    let mut license_set: BTreeSet<String> = BTreeSet::new();
    let mut license_order: Vec<String> = Vec::new();
    let mut files_stanza_count: usize = 0;
    for para in &paragraphs {
        if opt_field(para, "Files").is_some() {
            files_stanza_count += 1;
        }
        if let Some(lic) = opt_field(para, "License")
            && license_set.insert(lic.clone())
        {
            license_order.push(lic);
        }
    }

    CopyrightInfo {
        machine_readable,
        format_declaration,
        files_stanza_count,
        license_identifiers: license_order,
    }
}

// =============================================================================
// `debian/watch` v4.
// =============================================================================

fn read_watch(path: &Path) -> Option<WatchConfig> {
    let body = std::fs::read_to_string(path).ok()?;
    parse_watch(&body)
}

fn parse_watch(body: &str) -> Option<WatchConfig> {
    let mut version: Option<u32> = None;
    let mut url_pattern: Option<String> = None;
    let mut pgpsig: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("version=") {
            version = rest.trim().parse::<u32>().ok();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("pgpsigurlmangle=") {
            pgpsig = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("opts=") {
            // Compressed options line; for now we ignore
            // the contents (the spec calls for a summary,
            // and a 30-line compressed-options parser is
            // out of scope for the observational report).
            let _ = rest;
            continue;
        }
        if url_pattern.is_none() {
            // First non-directive, non-comment line is
            // the URL pattern.
            url_pattern = Some(trimmed.to_string());
        }
    }
    let version = version?;
    let url_pattern = url_pattern?;
    Some(WatchConfig {
        version,
        url_pattern,
        pgpsig,
    })
}

// =============================================================================
// `debian/patches/series` + each patch's DEP-3 header.
// =============================================================================

fn read_patches(patches_dir: &Path) -> (Vec<Patch>, Vec<Diagnostic>) {
    let mut patches: Vec<Patch> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let series_path = patches_dir.join("series");
    let Ok(body) = std::fs::read_to_string(&series_path) else {
        return (patches, diagnostics);
    };
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let patch_path = patches_dir.join(trimmed);
        if !patch_path.is_file() {
            diagnostics.push(Diagnostic {
                code: "E_PATCH_FILE_MISSING".to_string(),
                message: format!("patches/series references {trimmed:?} but the file is missing"),
                path: Some(patch_path.clone()),
            });
            patches.push(Patch {
                name: trimmed.to_string(),
                ..Patch::default()
            });
            continue;
        }
        let patch = read_patch(&patch_path, trimmed);
        patches.push(patch);
    }
    (patches, diagnostics)
}

fn read_patch(path: &Path, name: &str) -> Patch {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Patch {
            name: name.to_string(),
            ..Patch::default()
        };
    };
    let mut description: Option<String> = None;
    let mut author: Option<String> = None;
    let mut forwarded: Option<String> = None;
    let mut last_update: Option<String> = None;
    let mut current_key: Option<&str> = None;
    let mut current_value = String::new();
    for raw in body.lines() {
        // The DEP-3 header ends at the first blank line
        // followed by the diff. We treat any line that
        // doesn't look like `Key:` as the end of the
        // header.
        if raw.starts_with("--- ") || raw.starts_with("diff --git") || raw.starts_with("Index:") {
            break;
        }
        if raw.trim().is_empty() {
            break;
        }
        if let Some((key, value)) = raw.split_once(':') {
            // Flush previous.
            if let Some(k) = current_key.take() {
                let v = std::mem::take(&mut current_value);
                assign_dep3(
                    k,
                    &v,
                    &mut description,
                    &mut author,
                    &mut forwarded,
                    &mut last_update,
                );
            }
            current_key = Some(key.trim());
            current_value = value.trim().to_string();
        } else if let Some(_k) = current_key {
            // Continuation.
            current_value.push(' ');
            current_value.push_str(raw.trim());
        }
    }
    if let Some(k) = current_key.take() {
        let v = std::mem::take(&mut current_value);
        assign_dep3(
            k,
            &v,
            &mut description,
            &mut author,
            &mut forwarded,
            &mut last_update,
        );
    }
    Patch {
        name: name.to_string(),
        description,
        author,
        forwarded,
        last_update,
    }
}

fn assign_dep3(
    key: &str,
    value: &str,
    description: &mut Option<String>,
    author: &mut Option<String>,
    forwarded: &mut Option<String>,
    last_update: &mut Option<String>,
) {
    let owned = if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    };
    match key {
        "Description" => *description = owned,
        "Author" => *author = owned,
        "Forwarded" => *forwarded = owned,
        "Last-Update" => *last_update = owned,
        _ => {}
    }
}

// =============================================================================
// `debian/tests/control` (DEP-8).
// =============================================================================

fn read_tests_control(path: &Path) -> Vec<Autopkgtest> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_tests_control(&body)
}

fn parse_tests_control(body: &str) -> Vec<Autopkgtest> {
    use deb822_lossless::Deb822;

    let Ok(parsed) = Deb822::from_str(body) else {
        return Vec::new();
    };
    parsed
        .paragraphs()
        .map(|para| {
            // The DEP-8 `Tests:` field can have
            // multiple values (one per test) or a
            // single test name with a command. The
            // convention: the first token is the test
            // name; subsequent tokens are the
            // command. The 1C.2 report captures the
            // first test's name + command when the
            // field is a single line; multiple
            // `Tests:` lines aggregate into one
            // `Autopkgtest` per name.
            let tests_raw = opt_field(&para, "Tests");
            let (name, command) = tests_raw
                .as_deref()
                .and_then(|s| {
                    let mut tokens = s.split_whitespace();
                    let first = tokens.next()?.to_string();
                    let rest: Vec<&str> = tokens.collect();
                    let cmd = if rest.is_empty() {
                        None
                    } else {
                        Some(rest.join(" "))
                    };
                    Some((first, cmd))
                })
                .unwrap_or_else(|| (String::new(), None));
            Autopkgtest {
                name,
                command,
                restrictions: opt_field(&para, "Restrictions")
                    .map(|s| s.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default(),
                features: opt_field(&para, "Features")
                    .map(|s| s.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default(),
                depends: opt_field(&para, "Depends")
                    .map(|s| split_comma_list(&s))
                    .unwrap_or_default(),
            }
        })
        .filter(|t| !t.name.is_empty())
        .collect()
}

// =============================================================================
// `debian/rules` — executable, dh-style, override targets.
// =============================================================================

fn read_rules(path: &Path) -> RulesInfo {
    let Ok(body) = std::fs::read_to_string(path) else {
        return RulesInfo {
            executable: false,
            dh_style: false,
            override_targets: Vec::new(),
        };
    };
    let executable = executable_bit(path);
    let dh_style = parse_dh_style(&body);
    let override_targets = parse_override_targets(&body);
    RulesInfo {
        executable,
        dh_style,
        override_targets,
    }
}

fn executable_bit(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `dh`-style detection: the rules file's first line
/// is `#!/usr/bin/make -f` and the body contains
/// `dh $@` or `dh $@` somewhere. A simple substring
/// check; a real parse would tokenise the makefile.
fn parse_dh_style(body: &str) -> bool {
    let first_line = body.lines().next().unwrap_or("");
    let is_makefile_shebang =
        first_line.starts_with("#!/usr/bin/make") || first_line.starts_with("#! /usr/bin/make");
    let uses_dh = body.contains("dh $@") || body.contains("dh \"$@\"");
    is_makefile_shebang && uses_dh
}

fn parse_override_targets(body: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("override_") {
            // Match `override_<target>:`. The colon may
            // be followed by whitespace or be the end of
            // the line. Continuation lines (`\`-ended)
            // are skipped.
            if let Some(colon) = rest.find(':') {
                let target = rest[..colon].trim();
                if !target.is_empty() {
                    targets.push(target.to_string());
                }
            }
        }
    }
    targets
}

// =============================================================================
// Maintainer scripts.
// =============================================================================

fn read_maintainer_scripts(debian_dir: &Path) -> MaintainerScripts {
    let canonical = ["preinst", "postinst", "prerm", "postrm"];
    let preinst = read_script_presence(&debian_dir.join("preinst"));
    let postinst = read_script_presence(&debian_dir.join("postinst"));
    let prerm = read_script_presence(&debian_dir.join("prerm"));
    let postrm = read_script_presence(&debian_dir.join("postrm"));
    let triggers = if debian_dir.join("triggers").is_file() {
        Some(ScriptPresence { executable: false })
    } else {
        None
    };
    let other_scripts: Vec<NamedScript> = std::fs::read_dir(debian_dir)
        .map(|entries| {
            entries
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let name = entry.file_name().to_string_lossy().to_string();
                    if canonical.contains(&name.as_str()) {
                        return None;
                    }
                    if name == "rules" || name == "control" || name == "changelog" {
                        return None;
                    }
                    if name == "copyright" || name == "source" || name == "patches" {
                        return None;
                    }
                    if name == "tests" || name == "watch" {
                        return None;
                    }
                    if name == "triggers" {
                        return None;
                    }
                    if name == "gbp.conf" || name == "gbp-dch.conf" {
                        return None;
                    }
                    if name.starts_with('.') {
                        return None;
                    }
                    if !entry.path().is_file() {
                        return None;
                    }
                    Some(NamedScript {
                        name: name.clone(),
                        presence: ScriptPresence {
                            executable: executable_bit(&entry.path()),
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    MaintainerScripts {
        preinst,
        postinst,
        prerm,
        postrm,
        triggers,
        other_scripts,
    }
}

fn read_script_presence(path: &Path) -> Option<ScriptPresence> {
    if !path.is_file() {
        return None;
    }
    Some(ScriptPresence {
        executable: executable_bit(path),
    })
}

// =============================================================================
// GBP config.
// =============================================================================

fn read_gbp_config(debian_dir: &Path) -> GbpConfig {
    let gbp_conf_path = debian_dir.join("gbp.conf");
    let gbp_dch_conf_path = debian_dir.join("gbp-dch.conf");
    let gbp_conf_present = gbp_conf_path.is_file();
    let gbp_dch_conf_present = gbp_dch_conf_path.is_file();
    let branch_settings = if gbp_conf_present {
        std::fs::read_to_string(&gbp_conf_path)
            .ok()
            .and_then(|body| parse_gbp_conf_branch(&body))
    } else {
        None
    };
    GbpConfig {
        gbp_conf_present,
        gbp_dch_conf_present,
        branch_settings,
    }
}

fn parse_gbp_conf_branch(body: &str) -> Option<GbpBranchSettings> {
    let mut in_section = false;
    let mut out = GbpBranchSettings::default();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with('[') {
            in_section = trimmed == "[BUILD-DEFAULTS]";
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            let slot = match key {
                "branch" => Some(&mut out.branch),
                "debian-branch" => Some(&mut out.debian_branch),
                "upstream-branch" => Some(&mut out.upstream_branch),
                "upstream-tag" => Some(&mut out.upstream_tag),
                "debian-tag" => Some(&mut out.debian_tag),
                _ => None,
            };
            if let Some(slot) = slot {
                *slot = Some(value.to_string());
            }
        }
    }
    if out.branch.is_none()
        && out.debian_branch.is_none()
        && out.upstream_branch.is_none()
        && out.upstream_tag.is_none()
        && out.debian_tag.is_none()
    {
        None
    } else {
        Some(out)
    }
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use deb822_lossless::Deb822;

    #[test]
    fn changelog_header_canonical_form() {
        let parsed = parse_changelog_header("example (1.0.0-1) unstable; urgency=medium").unwrap();
        assert_eq!(parsed.source, "example");
        assert_eq!(parsed.version, "1.0.0-1");
        assert_eq!(parsed.distribution, "unstable");
        assert_eq!(parsed.urgency, "medium");
    }

    #[test]
    fn changelog_header_missing_urgency() {
        let parsed = parse_changelog_header("foo (1.0-1) unstable").unwrap();
        assert_eq!(parsed.distribution, "unstable");
        assert_eq!(parsed.urgency, "");
    }

    #[test]
    fn changelog_full_parses_trailer() {
        let body = "\
example (1.0.0-1) unstable; urgency=medium

  * Initial release.
 -- IronMaint Test <test@example.invalid>  Thu, 09 Oct 2026 00:00:00 +0000

example (0.9.0-1) unstable; urgency=low

  * Old.
 -- IronMaint Test <test@example.invalid>  Wed, 08 Oct 2026 00:00:00 +0000
";
        let id = parse_changelog_full(body).unwrap();
        assert_eq!(id.source, "example");
        assert_eq!(id.version, "1.0.0-1");
        assert_eq!(id.distribution, "unstable");
        assert_eq!(id.urgency, "medium");
        assert_eq!(
            id.maintainer.as_deref(),
            Some("IronMaint Test <test@example.invalid>")
        );
        assert_eq!(
            id.timestamp.as_deref(),
            Some("Thu, 09 Oct 2026 00:00:00 +0000")
        );
    }

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

    #[test]
    fn split_comma_list_handles_pinned_versions() {
        let s = "debhelper-compat (= 13), libfoo-dev, libbar-dev (>= 2.0)";
        let parts = split_comma_list(s);
        assert_eq!(
            parts,
            vec![
                "debhelper-compat (= 13)".to_string(),
                "libfoo-dev".to_string(),
                "libbar-dev (>= 2.0)".to_string(),
            ]
        );
    }

    #[test]
    fn parse_debhelper_compat_finds_the_version() {
        assert_eq!(parse_debhelper_compat("debhelper-compat (= 13)"), Some(13));
        assert_eq!(parse_debhelper_compat("debhelper-compat (= 12)"), Some(12));
        assert_eq!(parse_debhelper_compat("libfoo-dev"), None);
    }

    #[test]
    fn parse_watch_v4_basic() {
        let body = "version=4\nhttps://example.org/example-(.+)\\.tar\\.gz\n";
        let parsed = parse_watch(body).unwrap();
        assert_eq!(parsed.version, 4);
        assert_eq!(
            parsed.url_pattern,
            "https://example.org/example-(.+)\\.tar\\.gz"
        );
        assert_eq!(parsed.pgpsig, None);
    }

    #[test]
    fn parse_dh_style_detects_standard() {
        let body = "#!/usr/bin/make -f\n\n%:\n\tdh $@\n\noverride_dh_auto_test:\n\tdh_auto_test\n";
        assert!(parse_dh_style(body));
    }

    #[test]
    fn parse_override_targets_lists_them() {
        let body = "#!/usr/bin/make -f\n\noverride_dh_auto_test:\n\tdh_auto_test\n\
                    override_dh_auto_install:\n\tdh_auto_install --destdir=$(pwd)/debian/tmp\n";
        let targets = parse_override_targets(body);
        assert_eq!(
            targets,
            vec!["dh_auto_test".to_string(), "dh_auto_install".to_string()]
        );
    }
}
