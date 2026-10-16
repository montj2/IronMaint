//! `DebianSourcePreparationV1` — the wire format the 1C.1 tool
//! emits on stdout.
//!
//! PHASE-1.md §17 enumerates six checks for
//! `debian.inspect.source_preparation`:
//!
//!   1. `debian/` exists
//!   2. `debian/control` parseable
//!   3. `debian/changelog` parseable
//!   4. `Source:` from changelog/control == candidate `source_name`
//!   5. current changelog version == candidate `version`
//!   6. source format identifiable
//!
//! Each check has a flag in [`SourcePreparationFindings`] so an
//! agent can read the report and tell which rule fired. The
//! `verdict` field is the §17 "pass / fail / infrastructure_error"
//! tri-state: the normalizer maps it to
//! `EvidenceStatus::{Pass, Fail}` or to
//! `NormalizationError::Unclassifiable` (which the runtime
//! turns into `InfrastructureError`).
//!
//! A diagnostic is a single line of
//! `(code, message[, path])`. The schema-snapshot verifier
//! (`xtask verify-schemas`) walks the struct and pins the
//! wire format; any change here is a deliberate wire-format
//! change and must be matched by a schema-snapshot update.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The report kind discriminator. Always
/// `"debian.source_preparation"` for 1C.1; future subcommands
/// (`debian.inspect.source_analysis`, etc.) get their own
/// discriminators.
pub const REPORT_KIND: &str = "debian.source_preparation";

/// Wire-format version. Bump when the shape changes.
pub const SCHEMA_VERSION: u32 = 1;

/// The §17 tri-state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every check in §17 succeeded.
    Pass,
    /// At least one §17 check failed (mismatched name,
    /// mismatched version, unparsable `debian/control`,
    /// unparsable `debian/changelog`, missing `debian/`,
    /// unidentifiable source format, unparsable mandatory
    /// metadata). Diagnostics list the specific failure.
    Fail,
    /// The tool could not establish the fact at all: the
    /// workspace path is unreadable, an IO error occurred,
    /// a parser binary is missing. The §17 rule "parser
    /// binary missing → InfrastructureError" lives here.
    InfrastructureError,
}

/// A single failure or informational line from the tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Diagnostic {
    /// A stable, machine-readable error code, e.g.
    /// `E_DEBIAN_DIR_MISSING`. Future tools can match on
    /// this without parsing the message.
    pub code: String,
    /// Human-readable explanation. The tool does not embed
    /// the offending value (a) to keep PII out of the report
    /// and (b) so the same diagnostic text can serve any
    /// fixture.
    pub message: String,
    /// The on-disk path that triggered the diagnostic, when
    /// applicable (e.g. `debian/`, `debian/changelog`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// Per-check results. The fields are populated whether the
/// tool's `verdict` is `pass`, `fail`, or `infrastructure_error`;
/// the agent (or the next normalizer down the line) can use
/// them to decide what to do. A `false` flag here is *not* a
/// re-statement of the verdict — the verdict aggregates the
/// flags plus any cross-checks (e.g. a `Fail` is set even when
/// all flags are `true` if the changelog says version `1.0.0-1`
/// but the candidate says `1.0.0`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourcePreparationFindings {
    pub debian_directory_present: bool,
    pub control_parsable: bool,
    /// Source name parsed from `debian/control`. The value is
    /// always present when `control_parsable` is `true`;
    /// `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_source_name: Option<String>,
    /// Whether the control-parsed source name matches the
    /// candidate's `source_name`.
    pub control_source_name_matches_candidate: bool,
    pub changelog_parsable: bool,
    /// Source name parsed from `debian/changelog` (the
    /// first-entry's name field). The value is always
    /// present when `changelog_parsable` is `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changelog_source_name: Option<String>,
    pub changelog_source_name_matches_candidate: bool,
    /// Version parsed from `debian/changelog` (the
    /// first-entry's version field). The value is always
    /// present when `changelog_parsable` is `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changelog_version: Option<String>,
    pub changelog_version_matches_candidate: bool,
    pub format_identifiable: bool,
    /// `3.0 (quilt)`, `3.0 (native)`, `1.0`, or `unrecognised`
    /// (the literal value of `debian/source/format`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_format: Option<String>,
    /// `debian/rules` exists and is executable. Read but not
    /// enforced for the 1C.1 verdict (a non-executable rules
    /// is a build-time concern, not a source-preparation one).
    pub rules_executable: bool,
}

/// The first-entry identity block from `debian/changelog`.
///
/// 1C.2 extends this with `maintainer` and `timestamp`
/// (the closing-paren trailer's `Maintainer:` and `Date:`
/// lines). Both new fields are `Option<String>` so a
/// changelog whose trailer is malformed still parses
/// (the report's `diagnostics` carries the
/// `E_CHANGELOG_TRAILER_MALFORMED` line; the verdict
/// aggregates it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangelogIdentity {
    pub source: String,
    pub version: String,
    pub distribution: String,
    pub urgency: String,
    /// The trailer's `Maintainer:` line, e.g.
    /// `IronMaint Test <test@example.invalid>`. `None`
    /// when the trailer has no `Maintainer:` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer: Option<String>,
    /// The trailer's `Date:` line, in the RFC 2822
    /// form `dpkg-parsechangelog` emits (e.g.
    /// `Thu, 09 Oct 2026 00:00:00 +0000`). `None` when
    /// the trailer has no `Date:` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// The full `DebianSourcePreparationV1` report.
///
/// The runtime binds this to the Evidence row as a
/// `Report` artifact (PHASE-1.md §31, 1A.3). The
/// `evidence.artifact.read` MCP tool (1A.4) reads it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DebianSourcePreparationV1 {
    /// Always `1` for this revision. Bump on incompatible
    /// wire-format changes.
    pub schema_version: u32,
    /// Always `debian.source_preparation`. Discriminates this
    /// report from `debian.source_analysis_v1` (1C.2) and
    /// `debian.maintenance_context_v1` (1C.3). Stored as
    /// `String` rather than `&'static str` so the
    /// deserialised struct can be a normal owned value
    /// (a `&'static str` field forces the deserialiser to
    /// require `'static` lifetime on its input).
    pub report_kind: String,
    /// The candidate the tool was invoked against. Echoed
    /// back so the Evidence row is self-describing without
    /// a join.
    pub candidate_fingerprint: String,
    /// The §17 tri-state.
    pub verdict: Verdict,
    /// Per-check failure / informational lines. Empty when
    /// `verdict` is `pass`.
    pub diagnostics: Vec<Diagnostic>,
    /// Per-check results. Populated even when the verdict
    /// is `fail`; the agent can see *which* check failed.
    pub findings: SourcePreparationFindings,
    /// The first-entry identity from `debian/changelog`.
    /// `None` when `changelog_parsable` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<ChangelogIdentity>,
}

impl DebianSourcePreparationV1 {
    /// The schema-version constant for the type.
    #[must_use]
    pub const fn schema_version() -> u32 {
        SCHEMA_VERSION
    }

    /// The report-kind constant for the type.
    #[must_use]
    pub const fn report_kind() -> &'static str {
        REPORT_KIND
    }

    /// Build a new report with the `report_kind` set to
    /// the constant. Callers do not need to spell the
    /// literal; the constant is the single source of
    /// truth.
    #[must_use]
    pub fn with_kind(
        schema_version: u32,
        candidate_fingerprint: String,
        verdict: Verdict,
        diagnostics: Vec<Diagnostic>,
        findings: SourcePreparationFindings,
        identity: Option<ChangelogIdentity>,
    ) -> Self {
        Self {
            schema_version,
            report_kind: REPORT_KIND.to_string(),
            candidate_fingerprint,
            verdict,
            diagnostics,
            findings,
            identity,
        }
    }
}

// =============================================================================
// 1C.2 — `debian.inspect.source_analysis` + the comprehensive
// `DebianSourceReportV1` schema (PHASE-1.md §18).
//
// The 1C.1 report is a six-check summary; the 1C.2 report is
// the *comprehensive* source-tree inventory covering all
// 14 top-level groups §18 enumerates: identity, maintainers,
// source metadata, build metadata, binary packages, source
// format, patches, tests, watch, copyright, maintainer
// scripts, rules, GBP, and provenance. The verdict tri-state
// is the same; the observational rule (§18 lines 847-849) is
// the same: this report does not claim Policy compliance.
//
// The 1C.2 RED commit's struct was the minimal shape; this
// 1C.2 GREEN commit extends it with the §18 sub-structs and
// the `source_analysis::run` parser composition populates
// them. The §97 schema-snapshot tool (`xtask verify-schemas`)
// pins the wire format; the GREEN commit's first sub-step is
// `cargo run -p xtask -- verify-schemas --write` to regenerate
// `schemas/DebianSourceReportV1.json` against the new struct.
// =============================================================================

/// Discriminator for the 1C.2 report. Always
/// `"debian.source_analysis_v1"` — 1C.1's
/// `REPORT_KIND = "debian.source_preparation"` is the
/// `debian.inspect.source_preparation` discriminator;
/// 1C.2's is the comprehensive source-tree inventory.
pub const SOURCE_ANALYSIS_REPORT_KIND: &str = "debian.source_analysis_v1";

/// Wire-format version for the 1C.2 report. Independent of
/// the 1C.1 `SCHEMA_VERSION` constant; both happen to be
/// `1` but they may diverge.
pub const SOURCE_ANALYSIS_SCHEMA_VERSION: u32 = 1;

/// The canonical string form of `debian/source/format`.
///
/// Serialized as the literal file value (`"3.0 (quilt)"`,
/// `"3.0 (native)"`, or the catch-all `"other/legacy"`),
/// not the Rust variant name. A custom
/// `Serialize`/`Deserialize` impl carries the canonical
/// string through; schemars gets the same surface.
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
pub enum SourceFormat {
    /// The `3.0 (quilt)` source format. The default for
    /// modern Debian source packages.
    ThreeZeroQuilt,
    /// The `3.0 (native)` source format. Used when the
    /// upstream tarball is itself the Debian source.
    ThreeZeroNative,
    /// Any other value in `debian/source/format` (e.g.
    /// `1.0`, the native format pre-3.0). The raw
    /// string is preserved so an agent can tell what
    /// the tool saw.
    OtherLegacy(String),
}

impl serde::Serialize for SourceFormat {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let value = match self {
            Self::ThreeZeroQuilt => "3.0 (quilt)",
            Self::ThreeZeroNative => "3.0 (native)",
            Self::OtherLegacy(s) => s.as_str(),
        };
        ser.serialize_str(value)
    }
}

impl<'de> serde::Deserialize<'de> for SourceFormat {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        Ok(match s.as_str() {
            "3.0 (quilt)" => Self::ThreeZeroQuilt,
            "3.0 (native)" => Self::ThreeZeroNative,
            _ => Self::OtherLegacy(s),
        })
    }
}

/// The source paragraph's `Maintainer:` and `Uploaders:`
/// fields from `debian/control` (§18 maintainers group).
///
/// `Maintainer` is the primary contact for the source
/// package; `Uploaders` is a comma-separated list of
/// additional maintainers. The split into a `Vec<String>`
/// makes downstream analysis (e.g. "is this upload
/// co-maintained?") straightforward.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MaintainerInfo {
    /// The `Maintainer:` field, e.g.
    /// `IronMaint Test <test@example.invalid>`. Empty
    /// when the source paragraph omits it (the
    /// `Maintainer` field is mandatory by Policy, but
    /// the tool reports what is there).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub maintainer: String,
    /// The `Uploaders:` field, split on the RFC 822
    /// comma separator. Empty when the source paragraph
    /// has no `Uploaders:` field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uploaders: Vec<String>,
}

/// The source paragraph's metadata fields from
/// `debian/control` (§18 source metadata group).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceMetadata {
    /// The `Standards-Version:` field (e.g. `4.6.2`).
    /// §22's Standards-Version delta evaluator (1D.2)
    /// reads this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standards_version: Option<String>,
    /// The `Rules-Requires-Root:` field (e.g. `no`,
    /// `binary-targets`). `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_requires_root: Option<String>,
    /// The `Section:` field (e.g. `utils`, `devel`).
    /// `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The `Priority:` field (e.g. `optional`,
    /// `required`, `standard`, `important`).
    /// `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// The `Homepage:` field. `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// The `Vcs-Git:` field. `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_git: Option<String>,
    /// The `Vcs-Browser:` field. `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_browser: Option<String>,
}

/// The source paragraph's build-related fields
/// (§18 build metadata group).
///
/// `Build-Depends` and friends are RFC 822
/// comma-separated alternation lists; the tool
/// splits them on `,` and trims each entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildMetadata {
    /// The `Build-Depends:` field, split into individual
    /// entries (e.g. `["debhelper-compat (= 13)",
    /// "libfoo-dev"]`). Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_depends: Vec<String>,
    /// The `Build-Depends-Indep:` field, split into
    /// individual entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_depends_indep: Vec<String>,
    /// The `Build-Conflicts:` field, split into
    /// individual entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_conflicts: Vec<String>,
    /// The `debhelper-compat (= N)` value, parsed
    /// out of the `Build-Depends` list. `None` when
    /// no `debhelper-compat` entry is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debhelper_compat: Option<u32>,
}

/// One binary package stanza from `debian/control`
/// (§18 binary packages group).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BinaryPackage {
    /// The `Package:` field.
    pub name: String,
    /// The `Architecture:` field (e.g. `any`, `all`,
    /// `amd64`, `any-amd64`).
    pub architecture: String,
    /// The `Multi-Arch:` field (e.g. `same`, `foreign`,
    /// `allowed`). `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_arch: Option<String>,
    /// The `Depends:` field, split into individual
    /// entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends: Vec<String>,
    /// The `Provides:` field, split into individual
    /// entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<String>,
    /// The `Breaks:` field, split into individual
    /// entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<String>,
    /// The `Conflicts:` field, split into individual
    /// entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// The `Replaces:` field, split into individual
    /// entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    /// The `Maintainer:` field (may differ from the
    /// source paragraph's `Maintainer`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer: Option<String>,
    /// The `Description:` field, first paragraph only
    /// (the synopsis). Multi-paragraph descriptions
    /// are concatenated by dpkg-gencontrol at build
    /// time; the report captures the synopsis for
    /// observational use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One patch in `debian/patches/series` with its
/// DEP-3 header (§18 patches group).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Patch {
    /// The patch file name as listed in `series`
    /// (e.g. `01-fix-typo.patch`).
    pub name: String,
    /// The DEP-3 `Description:` header. `None` when
    /// the patch file has no DEP-3 header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The DEP-3 `Author:` header. `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// The DEP-3 `Forwarded:` header (e.g. `yes`,
    /// `no`, `not-needed`). `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded: Option<String>,
    /// The DEP-3 `Last-Update:` header (ISO 8601 date).
    /// `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update: Option<String>,
}

/// One autopkgtest stanza from `debian/tests/control`
/// (§18 tests group).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Autopkgtest {
    /// The `Tests:` field, first token (the test name).
    pub name: String,
    /// Subsequent tokens on the `Tests:` line
    /// (the command to run, when it's not the
    /// `debian/tests/<name>` default). `None` when
    /// the `Tests:` line is just the test name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The `Restrictions:` field, split into
    /// individual tokens (e.g. `["rw-build-tree"]`).
    /// Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restrictions: Vec<String>,
    /// The `Features:` field, split into individual
    /// tokens. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// The `Depends:` field, split into individual
    /// entries. Empty when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends: Vec<String>,
}

/// The parsed `debian/watch` file (§18 watch group).
///
/// The Debian watch file is a small RFC 822-ish
/// grammar: `version=N\n<url-pattern>` with optional
/// `pgpsigurlmangle=` and `opts=...` directives. The
/// tool captures the version, the URL pattern, and any
/// PGP-signature-mangling directive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WatchConfig {
    /// The watch file's `version=N` directive
    /// (typically `4`).
    pub version: u32,
    /// The watch file's URL pattern (the first
    /// non-comment, non-directive line).
    pub url_pattern: String,
    /// The `pgpsigurlmangle=...` directive, when
    /// present. `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pgpsig: Option<String>,
}

/// The parsed `debian/copyright` DEP-5 file
/// (§18 copyright group).
///
/// The report captures the machine-readable
/// indicator, the `Format:` URL, the count of
/// `Files:` stanzas, and the set of `License:`
/// identifiers across all stanzas (deduplicated,
/// order-preserving).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CopyrightInfo {
    /// `true` when the `Format:` field's value is a
    /// URL (i.e. DEP-5 machine-readable form).
    /// `false` for legacy "paragraph prose" form.
    pub machine_readable: bool,
    /// The `Format:` URL, when present. `None` when
    /// the copyright file omits the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_declaration: Option<String>,
    /// The number of `Files:` stanzas in the file.
    /// Zero for a header-only copyright file.
    pub files_stanza_count: usize,
    /// The set of `License:` values, deduplicated
    /// in declaration order. Empty when no
    /// `License:` field is present in any stanza.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub license_identifiers: Vec<String>,
}

/// One maintainer script's presence + executable bit
/// (§18 maintainer scripts group).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScriptPresence {
    /// `true` when the script file exists and the
    /// executable bit is set.
    pub executable: bool,
}

/// The full maintainer-script inventory
/// (§18 maintainer scripts group).
///
/// The four canonical scripts are
/// `preinst`/`postinst`/`prerm`/`postrm`. The
/// `triggers` file (a non-executable control file)
/// and "other recognized scripts" (e.g.
/// `postinst-config`, `preinst-base`) are
/// observational — the report captures what is
/// there without claiming anything about
/// what *should* be there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MaintainerScripts {
    /// `Some` when `debian/preinst` exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preinst: Option<ScriptPresence>,
    /// `Some` when `debian/postinst` exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postinst: Option<ScriptPresence>,
    /// `Some` when `debian/prerm` exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerm: Option<ScriptPresence>,
    /// `Some` when `debian/postrm` exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postrm: Option<ScriptPresence>,
    /// `Some` when `debian/triggers` exists. The
    /// `triggers` file is a control file (not
    /// executable) that declares the package's
    /// interest in dpkg triggers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggers: Option<ScriptPresence>,
    /// Other recognized scripts in `debian/`
    /// (e.g. `preinst-base`, `postinst-config`).
    /// Each entry is a `ScriptPresence` keyed by
    /// the script's name. Excluded: the four
    /// canonical scripts (above), `rules`
    /// (covered separately), and the patch
    /// series (also covered separately).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_scripts: Vec<NamedScript>,
}

/// A named maintainer script entry for the
/// `other_scripts` list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NamedScript {
    /// The script's file name (e.g. `preinst-base`).
    pub name: String,
    /// Presence + executable bit.
    pub presence: ScriptPresence,
}

/// The `debian/rules` presence + analysis
/// (§18 rules group).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RulesInfo {
    /// `true` when `debian/rules` is executable.
    pub executable: bool,
    /// `true` when the rules file's first line is
    /// `#!/usr/bin/make -f` and the body contains
    /// a `dh` invocation. The standard `debhelper`
    /// style. The tool's detection is a simple
    /// substring check; it does not parse the
    /// makefile.
    #[serde(default)]
    pub dh_style: bool,
    /// The names of `override_*:` targets in the
    /// rules file. Empty when no override targets
    /// are detected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub override_targets: Vec<String>,
}

/// The GBP config inventory (§18 GBP group).
///
/// The tool reports presence of `debian/gbp.conf`
/// and `debian/gbp-dch.conf` and, when present,
/// the branch / tag settings from the INI-formatted
/// `[BUILD-DEFAULTS]` / `[DCH]` sections. The
/// `branch_settings` block is a structural mirror
/// of the most-commonly-used keys; it is not a
/// full parse of every gbp.conf key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GbpConfig {
    /// `true` when `debian/gbp.conf` exists.
    pub gbp_conf_present: bool,
    /// `true` when `debian/gbp-dch.conf` exists.
    pub gbp_dch_conf_present: bool,
    /// The branch / tag settings parsed from
    /// `gbp.conf`. `None` when `gbp.conf` is
    /// absent or has no `[BUILD-DEFAULTS]`
    /// section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_settings: Option<GbpBranchSettings>,
}

/// The branch / tag settings from
/// `debian/gbp.conf`'s `[BUILD-DEFAULTS]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GbpBranchSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debian_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debian_tag: Option<String>,
}

/// The provenance block (§18 provenance group).
///
/// The candidate fingerprint, Git commit, and Git
/// tree are the candidate's identity (the §18 lines
/// 842-846 requirement). `inspector_version` is
/// the version of the `ironmaint-debian-tool` binary
/// that produced the report, so a downstream
/// consumer can tell which parser version emitted
/// the report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    /// The candidate's BLAKE3 fingerprint.
    pub candidate_fingerprint: String,
    /// The candidate's Git commit OID.
    pub candidate_commit: String,
    /// The candidate's Git tree OID.
    pub candidate_tree: String,
    /// The `ironmaint-debian-tool` binary's version
    /// (e.g. `0.1.0`).
    pub inspector_version: String,
}

/// The §18 comprehensive `DebianSourceReportV1`.
///
/// This is the GREEN commit's struct: every §18
/// top-level group is present, every field the
/// spec names under each group is typed and
/// serialized. The 1C.2 RED commit's struct was
/// the minimal shape; this commit extends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DebianSourceReportV1 {
    /// Always `1` for this revision. Bump on
    /// incompatible wire-format changes.
    pub schema_version: u32,
    /// Always `debian.source_analysis_v1`.
    pub report_kind: String,
    /// The §17 tri-state.
    pub verdict: Verdict,
    /// Per-check failure / informational lines.
    /// Empty when `verdict` is `pass`.
    pub diagnostics: Vec<Diagnostic>,

    // -- §18 top-level groups (in spec order) ---------------
    /// `identity` — the first-entry changelog block
    /// plus the trailer's `Maintainer:` and `Date:`
    /// fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<ChangelogIdentity>,

    /// `maintainers` — the source paragraph's
    /// `Maintainer:` and `Uploaders:` fields.
    #[serde(default)]
    pub maintainers: MaintainerInfo,

    /// `source metadata` — the source paragraph's
    /// `Standards-Version`, `Rules-Requires-Root`,
    /// `Section`, `Priority`, `Homepage`, `Vcs-Git`,
    /// `Vcs-Browser` fields.
    #[serde(default)]
    pub source_metadata: SourceMetadata,

    /// `build metadata` — the source paragraph's
    /// `Build-Depends`, `Build-Depends-Indep`,
    /// `Build-Conflicts`, and the parsed
    /// `debhelper-compat` version.
    #[serde(default)]
    pub build_metadata: BuildMetadata,

    /// `binary packages` — every binary package
    /// stanza in `debian/control`.
    #[serde(default)]
    pub binary_packages: Vec<BinaryPackage>,

    /// `source format` — the value of
    /// `debian/source/format`.
    pub source_format: SourceFormat,

    /// `patches` — every patch in
    /// `debian/patches/series` with its DEP-3
    /// header.
    #[serde(default)]
    pub patches: Vec<Patch>,

    /// `tests` — every autopkgtest stanza in
    /// `debian/tests/control`.
    #[serde(default)]
    pub tests: Vec<Autopkgtest>,

    /// `watch` — the parsed `debian/watch` file.
    /// `None` when the file is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<WatchConfig>,

    /// `copyright` — the parsed `debian/copyright`
    /// DEP-5 file.
    #[serde(default)]
    pub copyright: CopyrightInfo,

    /// `maintainer scripts` — presence + executable
    /// bit for the four canonical scripts, the
    /// `triggers` file, and other recognized
    /// scripts.
    #[serde(default)]
    pub maintainer_scripts: MaintainerScripts,

    /// `rules` — the `debian/rules` executable bit,
    /// the `dh`-style indicator, and the list of
    /// `override_*:` targets.
    #[serde(default)]
    pub rules: RulesInfo,

    /// `GBP` — presence of `debian/gbp.conf` and
    /// `debian/gbp-dch.conf` and the parsed
    /// `[BUILD-DEFAULTS]` branch settings.
    #[serde(default)]
    pub gbp: GbpConfig,

    /// `provenance` — the candidate's fingerprint,
    /// Git commit, Git tree, and the inspector
    /// binary's version.
    pub provenance: Provenance,
}

// =============================================================================
// 1C.3 — `debian.inspect.maintenance_context` + the
// `DebianMaintenanceContextV1` schema (PHASE-1.md §13, §19, §26).
//
// The 1C.2 `DebianSourceReportV1` is the *source-tree inventory*;
// the 1C.3 `DebianMaintenanceContextV1` is the *maintenance
// context*: the same Debian source tree data, framed as "how is
// this package being maintained?" (patch stack vs. applied
// patches, gbp branch settings vs. raw config presence, upstream
// relationship metadata). Both tools surface the same source-tree
// artifacts, but the framing differs (§26).
//
// The 1C.3 RED commit's struct is the *minimal* shape (§17
// tri-state verdict + provenance + diagnostics only); the 1C.3
// GREEN commit extends it with the §26 comprehensive fields
// (patches, tests, watch, gbp, maintainer scripts,
// upstream_relationship). The §97 schema-snapshot tool
// (`xtask verify-schemas`) catches GREEN-side drift: `cargo
// run -p xtask -- verify-schemas --write` regenerates
// `schemas/DebianMaintenanceContextV1.json` against the new
// struct. The decision to use composition (re-emit 1C.2 fields
// + add new UpstreamRelationship) — rather than extending
// `DebianSourceReportV1` in place — keeps the §13 routing-key
// separation (one tool = one struct = one snapshot) and the
// `candidate_fingerprint` in `provenance` is the join key for
// any consumer that needs the union of both reports.
// =============================================================================

/// Discriminator for the 1C.3 report. Always
/// `"debian.maintenance_context_v1"` — distinct from 1C.1's
/// `"debian.source_preparation"` and 1C.2's
/// `"debian.source_analysis_v1"`.
pub const MAINTENANCE_CONTEXT_REPORT_KIND: &str = "debian.maintenance_context_v1";

/// Wire-format version for the 1C.3 report. Independent of
/// the 1C.1 / 1C.2 `SCHEMA_VERSION` constants; all happen to
/// be `1` but they may diverge.
pub const MAINTENANCE_CONTEXT_SCHEMA_VERSION: u32 = 1;

/// The upstream-relationship metadata block (§26 upstream
/// relationship group; 1C.3's only new sub-type).
///
/// Carries the raw `Option<String>` fields from
/// `debian/copyright` (Upstream-Name, Upstream-Contact,
/// Source) and `debian/control` (Vcs-Git, Vcs-Browser,
/// Homepage). The `team_maintained: bool` classification
/// (§26 "Team maintained" bullet) is intentionally deferred
/// to a future PR — the spec does not define the heuristic
/// and committing to one prematurely risks rework.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpstreamRelationship {
    /// The DEP-5 `Upstream-Name:` field from
    /// `debian/copyright`. `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_name: Option<String>,
    /// The DEP-5 `Upstream-Contact:` field from
    /// `debian/copyright`. `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_contact: Option<String>,
    /// The DEP-5 `Source:` field from
    /// `debian/copyright` (the canonical upstream URL
    /// for the Files stanzas that follow). `None`
    /// when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_url: Option<String>,
    /// The `Vcs-Git:` field from `debian/control`.
    /// `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_git: Option<String>,
    /// The `Vcs-Browser:` field from `debian/control`.
    /// `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_browser: Option<String>,
    /// The `Homepage:` field from `debian/control`.
    /// `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
}

/// The 1C.3 maintenance-context report. The 1C.3 RED
/// commit's struct is the *minimal* shape (§17 tri-state
/// + provenance + diagnostics); the 1C.3 GREEN commit
///   extends it with the §26 comprehensive fields
///   (patches, tests, watch, gbp, maintainer scripts,
///   upstream_relationship). The sub-types `Patch`,
///   `Autopkgtest`, `WatchConfig`, `GbpConfig`,
///   `MaintainerScripts`, and `Provenance` are reused
///   from the 1C.2 `DebianSourceReportV1` — they live in
///   the same `report` module and the 1C.3 parser
///   composition re-uses the 1C.2 helpers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DebianMaintenanceContextV1 {
    /// Wire-format version. Always
    /// `MAINTENANCE_CONTEXT_SCHEMA_VERSION` (= 1).
    pub schema_version: u32,
    /// Discriminator. Always
    /// `MAINTENANCE_CONTEXT_REPORT_KIND` (= `"debian.maintenance_context_v1"`).
    pub report_kind: String,
    /// The candidate's BLAKE3 fingerprint, Git
    /// commit, and Git tree; the inspector
    /// binary's version. 1C.2's `Provenance`
    /// type, reused.
    #[serde(default)]
    pub provenance: Provenance,
    /// The §17 tri-state verdict.
    pub verdict: Verdict,
    /// Diagnostics (e.g. `E_DEBIAN_DIR_MISSING`).
    /// Empty when `verdict` is `pass`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// §26 maintenance dimension: applied patches
    /// with DEP-3 metadata. Reused from 1C.2's
    /// `Patch` type. Empty when `debian/patches/`
    /// is absent or `series` is empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patches: Vec<Patch>,
    /// §26 maintenance dimension: autopkgtest
    /// stanzas from `debian/tests/control`. Reused
    /// from 1C.2's `Autopkgtest` type. Empty when
    /// the file is absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<Autopkgtest>,
    /// §26 maintenance dimension: `debian/watch`
    /// v4 configuration. Reused from 1C.2's
    /// `WatchConfig` type. `None` when the file is
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<WatchConfig>,
    /// §26 maintenance dimension: gbp / dch
    /// configuration presence. Reused from 1C.2's
    /// `GbpConfig` type. Always present (the
    /// struct's `gbp_conf_present: false` carries
    /// the absence signal).
    #[serde(default)]
    pub gbp: GbpConfig,
    /// §26 maintenance dimension: presence and
    /// executable bit of `debian/{preinst,
    /// postinst, prerm, postrm}`. Reused from
    /// 1C.2's `MaintainerScripts` type.
    #[serde(default)]
    pub maintainer_scripts: MaintainerScripts,
    /// §26 maintenance dimension: upstream
    /// relationship metadata (Upstream-Name,
    /// Upstream-Contact, Vcs-Git, Vcs-Browser,
    /// Homepage). New in 1C.3.
    #[serde(default)]
    pub upstream_relationship: UpstreamRelationship,
}

// =============================================================================
// 1D.2 — `debian.policy.delta_review` + the
// `DebianPolicyDeltaReviewV1` schema (PHASE-1.md §19, §22).
//
// The 1C.x reports (source-preparation, source-analysis,
// maintenance-context) inventory the source tree; the
// 1D.2 report evaluates the candidate's policy
// posture against the §19 manifest baseline. The
// §22 gate ("IronMaint's Debian Policy knowledge is
// stale; refresh the authority snapshot before
// evaluating this candidate") is the load-bearing
// rule: a candidate whose `debian/control` declares
// a `Standards-Version` newer than the manifest's
// `standards_version_target` (currently `4.7.4`)
// yields `Verdict::InfrastructureError` with the
// `E_POLICY_BASELINE_STALE` diagnostic — the §20
// spec is explicit that a candidate above baseline
// "must not be marked bad" (the manifest is stale,
// not the package).
//
// The 1D.2 RED commit's struct is the *minimal*
// shape (verdict + diagnostics + the new
// `candidate_standards_version`, `baseline_standards_version`,
// and `comparison_outcome` fields); the
// 1D.2 GREEN commit's struct is the comprehensive
// shape (the same fields, fully populated, plus
// the candidate-vs-baseline delta rationale + the
// §20 upgrading-checklist entries that name the
// §22 narrative). The §97 schema-snapshot tool
// (`xtask verify-schemas`) catches GREEN-side
// drift: `cargo run -p xtask -- verify-schemas
// --write` regenerates
// `schemas/DebianPolicyDeltaReviewV1.json` against
// the new struct.
//
// PHASE-1.1 PR 1.1.D (policy baseline correctness):
// the pre-1.1.D comparator was the manifest's
// `baseline_policy_version` (= `"4.7.4.1"`, the
// *Policy Manual document* version). The spec
// is explicit (PHASE-1.md line 978) that the
// baseline is the `Standards-Version target`
// (= `"4.7.4"`, not the document version) and
// that a candidate above baseline must be
// `Verdict::InfrastructureError` (not `Fail`).
// 1.1.D splits the manifest into
// `policy_document_version` and
// `standards_version_target`, routes the
// comparator through the latter, flips the
// `AboveBaseline` verdict, and adds the
// `intervening_checklist_entries` field that
// §20 promised for `BelowBaseline` reports.
// =============================================================================

/// Discriminator for the 1D.2 report. Always
/// `"debian.policy_delta_review_v1"` — distinct from
/// 1C.1's `"debian.source_preparation"`, 1C.2's
/// `"debian.source_analysis_v1"`, and 1C.3's
/// `"debian.maintenance_context_v1"`.
pub const POLICY_DELTA_REPORT_KIND: &str = "debian.policy_delta_review_v1";

/// Wire-format version for the 1D.2 report.
/// Independent of the 1C.x `SCHEMA_VERSION`
/// constants; all happen to be `1` but they may
/// diverge.
pub const POLICY_DELTA_SCHEMA_VERSION: u32 = 1;

/// The §22 Standards-Version comparison outcome.
///
/// `AboveBaseline` is the gate-triggering case:
/// the candidate declares a `Standards-Version`
/// newer than the manifest's
/// `baseline_policy_version`, so the candidate
/// cannot be evaluated against the adapter's known
/// Policy and the §22 gate is blocked. `AtBaseline`
/// and `BelowBaseline` are both pass-path cases
/// (the candidate's declared version is known).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StandardsVersionComparison {
    /// The candidate's `Standards-Version` is
    /// strictly newer than the manifest's
    /// `baseline_policy_version`. The §22 gate is
    /// blocked; the verdict is `Fail`.
    AboveBaseline,
    /// The candidate's `Standards-Version` equals
    /// the manifest's `baseline_policy_version`. The
    /// candidate is at the manifest's known Policy;
    /// the verdict is `Pass`.
    AtBaseline,
    /// The candidate's `Standards-Version` is
    /// strictly older than the manifest's
    /// `baseline_policy_version`. The candidate is
    /// within the manifest's known Policy; the
    /// verdict is `Pass`.
    BelowBaseline,
}

/// The 1D.2 policy-delta report.
///
/// The 1D.2 RED commit's struct is the
/// *minimal* shape: just `schema_version`,
/// `report_kind`, `verdict`, `diagnostics`,
/// plus the three core fields
/// (`candidate_standards_version`,
/// `baseline_standards_version`, and
/// `comparison_outcome`). The 1D.2 GREEN
/// commit's struct extends the same shape
/// with the §22 comprehensive fields (the
/// `diagnostics` carries the
/// `E_POLICY_BASELINE_STALE` line, the
/// `comparison_outcome` is the
/// `AboveBaseline` / `AtBaseline` /
/// `BelowBaseline` tri-state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DebianPolicyDeltaReviewV1 {
    /// Wire-format version. Always
    /// `POLICY_DELTA_SCHEMA_VERSION` (= 1).
    pub schema_version: u32,
    /// Discriminator. Always
    /// `POLICY_DELTA_REPORT_KIND` (=
    /// `"debian.policy_delta_review_v1"`).
    pub report_kind: String,
    /// The candidate's BLAKE3 fingerprint, Git
    /// commit, and Git tree; the inspector
    /// binary's version. 1C.2's `Provenance`
    /// type, reused.
    #[serde(default)]
    pub provenance: Provenance,
    /// The §17 tri-state verdict.
    pub verdict: Verdict,
    /// Per-check failure / informational lines.
    /// Empty when `verdict` is `pass`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// The candidate's declared
    /// `Standards-Version` from `debian/control`.
    /// `None` when the field is absent
    /// (Policy-violation, but the report records
    /// the absence — `verdict: "fail"` with
    /// `E_STANDARDS_VERSION_MISSING`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_standards_version: Option<String>,
    /// The manifest's `standards_version_target`
    /// (currently `"4.7.4"`). Always present
    /// when the manifest loaded successfully.
    ///
    /// PHASE-1.1 PR 1.1.D (policy baseline
    /// correctness): renamed from
    /// `baseline_policy_version` to match the
    /// manifest field name. The pre-1.1.D
    /// value was the *Policy Manual document*
    /// version (4.7.4.1); the post-1.1.D value
    /// is the `Standards-Version` target (4.7.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_standards_version: Option<String>,
    /// The §22 comparison outcome.
    /// `None` when one or both `Standards-Version`
    /// values are missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_outcome: Option<StandardsVersionComparison>,
    /// The §20 upgrading-checklist entries
    /// whose interval lies *on* the path from
    /// the candidate's `Standards-Version` to
    /// the manifest's `standards_version_target`
    /// — containment
    /// (`from >= candidate && to <= baseline`).
    /// Populated only when
    /// `comparison_outcome` is `BelowBaseline`
    /// (the operator's remediation is the
    /// entries' narrative). Empty for
    /// `AtBaseline`, `AboveBaseline`, and `None`.
    ///
    /// PHASE-1.1 PR 1.1.D (policy baseline
    /// correctness) added this field. The pre-1.1.D
    /// report did not carry the §20 promise; the
    /// 1.1.D-corrected report does.
    ///
    /// PHASE-1.1 PR 1.1.J (policy delta evidence)
    /// tightened the predicate from *overlap*
    /// (`to >= candidate && from <= baseline`) to
    /// *containment* (`from >= candidate &&
    /// to <= baseline`). The overlap predicate
    /// returned entries whose interval was
    /// entirely *above* the candidate (e.g. the
    /// bundled `4.7.4 → 4.7.4.1` entry when the
    /// candidate is `4.6.0` and the baseline is
    /// `4.7.4`); containment returns only the
    /// entries whose interval sits on the
    /// candidate-to-baseline path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intervening_checklist_entries: Vec<ChecklistEntry>,
}

/// One entry in
/// `policy/debian/upgrading-checklist.v1.json` —
/// the §20 narrative of a single Standards-Version
/// interval.
///
/// PHASE-1.1 PR 1.1.D (policy baseline correctness):
/// the `ChecklistEntry` struct is the data the
/// `BelowBaseline` reports name. The 4.7.4 → 4.7.4.1
/// interval is the only entry the bundled
/// `upgrading-checklist.v1.json` carries today; a
/// future PR can extend the JSON to cover more
/// intervals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChecklistEntry {
    /// The older end of the interval. The
    /// operator's `Standards-Version` is at or
    /// above this value.
    pub from_version: String,
    /// The newer end of the interval. The
    /// operator's `Standards-Version` is at or
    /// below this value.
    pub to_version: String,
    /// The §20 narrative — a one-paragraph
    /// summary of the changes in the interval.
    pub summary: String,
    /// Where the §20 narrative came from (e.g.
    /// a Debian Policy release-notes URL or a
    /// manual note).
    pub source: String,
}

// ============================================================================
// 1E.2 — `DebianIssueReportV1` (PHASE-1.md §23, §24)
// ============================================================================
//
// The 1E.2 BTS snapshot is the §23 read-only transport's
// data product. The wire format mirrors the canonical-JSON
// shape the 1E.1 `bts-helper.py` shim emits, plus a §17
// `verdict` tri-state that the normalizer classifies.
//
// The 1E.2 verdict is binary — `pass` or
// `infrastructure_error` — because §24 says "A package
// having 50 open bugs is not a tool failure" and
// "DNS / TLS / timeout / SOAP / malformed service
// response" all collapse to `InfrastructureError →
// Blocked`. There is no `Fail` case for the BTS
// snapshot: the snapshot either succeeded (the bug
// list is observed regardless of size) or the
// transport failed (the runtime's `Blocked` gate
// runs the recovery path).
//
// The 1E.2 RED commit's struct is the *minimal* shape:
// just `schema_version`, `report_kind`, and `verdict`.
// The 1E.2 GREEN commit's struct extends the same
// shape with the §23 comprehensive fields
// (`source_package`, `snapshot_at`, `bugs`,
// `summary`).
//
// The 1E.2 broken-then-fixed sentinel lives in the
// normalizer: RED maps malformed JSON to
// `NormalizationError::Malformed(...)` (the existing
// pattern for the 1C.x / 1D.x normalizers); GREEN
// maps malformed JSON to
// `NormalizationError::Unclassifiable(...)` per
// §24 "malformed service response" →
// `InfrastructureError → Blocked`.

/// Discriminator value for the 1E.2 BTS snapshot report.
/// Stable across the lifetime of `DebianIssueReportV1`.
pub const BTS_SNAPSHOT_REPORT_KIND: &str = "debian.issue.bts_snapshot_v1";

/// Wire-format version. Bump on incompatible wire-format
/// changes.
pub const BTS_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// A single bug record from the BTS snapshot. The shape
/// matches the 1E.1 `bts-helper.py` output's `bugs[]`
/// elements; downstream consumers (the agent, the exit
/// artifact) read `id`, `severity`, `status`, and the
/// `tags` list. The `forwarded` and `patch_tagged`
/// booleans are derived from the `tags` list (the
/// `bts-helper.py` does the derivation; the wire
/// format exposes both the raw tags and the derived
/// booleans so a consumer that reads only the wire
/// format can answer §23 "is this bug patch-tagged?
/// is it forwarded upstream?" without re-implementing
/// the derivation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BugRecord {
    /// The BTS bug number. The BTS uses integers; the
    /// wire format keeps the integer type.
    pub id: u64,
    /// The bug's severity as the BTS reports it. Common
    /// values: `critical`, `grave`, `serious`, `important`,
    /// `normal`, `minor`, `wishlist`. The wire format
    /// stores the raw string (the BTS does not constrain
    /// the set; new Debian releases can add severities
    /// without a wire-format change).
    pub severity: String,
    /// The bug's status as the BTS reports it. Common
    /// values: `open`, `done`, `forwarded`, `pending`
    /// (the BTS uses `pending-fixed`, `pending-upload`,
    /// etc.; the wire format stores the raw string).
    pub status: String,
    /// The bug's subject line. Stored verbatim from the
    /// BTS (the BTS does not enforce length; a §23
    /// consumer can truncate for display).
    pub subject: String,
    /// The bug's tags. The BTS exposes a free-form tag
    /// list (e.g. `patch`, `upstream`, `security`,
    /// `l10n`, `d-i`). The wire format stores the raw
    /// strings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Derived flag: `true` iff the bug is `forwarded`
    /// upstream (the tag list contains `upstream` or
    /// `forwarded`). The 1E.1 shim computes this; the
    /// wire format exposes it so the §23 "forwarded
    /// upstream" question is answerable without
    /// re-deriving.
    #[serde(default)]
    pub forwarded: bool,
    /// Derived flag: `true` iff the bug has a patch
    /// attached (the tag list contains `patch`). The
    /// 1E.1 shim computes this; the wire format
    /// exposes it so the §23 "patch-tagged" question
    /// is answerable without re-deriving.
    #[serde(default)]
    pub patch_tagged: bool,
}

/// The §23 summary fields. Computed from the `bugs`
/// list by the 1E.1 shim; the wire format exposes
/// the pre-computed values so a consumer that only
/// reads the summary doesn't have to walk the bug
/// list.
///
/// The §24 example output names all four fields:
/// `open`, `serious`, `patch_tagged`, `forwarded`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BugSummary {
    /// Number of bugs with `status == "open"`.
    pub open: u32,
    /// Number of bugs with `status == "open"` AND
    /// `severity == "serious"` (or higher: `critical`,
    /// `grave`; the 1E.1 shim treats all of these as
    /// "serious" for the summary count).
    pub serious: u32,
    /// Number of bugs with `patch_tagged == true`.
    pub patch_tagged: u32,
    /// Number of bugs with `forwarded == true`.
    pub forwarded: u32,
}

/// The 1E.2 BTS snapshot report.
///
/// 1E.2 RED: the struct is the *minimal* shape
/// (`schema_version`, `report_kind`, `verdict`).
///
/// 1E.2 GREEN: the struct gains the §23 comprehensive
/// fields (`source_package`, `snapshot_at`, `bugs`,
/// `summary`). The minimal shape is the JSON shape
/// the 1E.1 `bts-helper.py` shim emits; the
/// comprehensive shape is the same shape a 1E.2
/// consumer can read directly (the `bugs` list and
/// `summary` are the §23 inventory of observed
/// issues). Both shapes are RFC 8259 JSON; both
/// carry the same `report_kind` discriminator and
/// `verdict` tri-state; the comprehensive shape
/// adds the observation fields.
///
/// The `verdict` is the §17 tri-state minus `Fail`
/// (the BTS snapshot has no "fail" semantics per
/// §24 — a snapshot either succeeded or the
/// transport failed). The normalizer maps
/// `verdict: pass` to `EvidenceStatus::Pass` and
/// `verdict: infrastructure_error` to
/// `NormalizationError::Unclassifiable(...)` per
/// §24.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DebianIssueReportV1 {
    /// Wire-format version. Always
    /// `BTS_SNAPSHOT_SCHEMA_VERSION` (= 1).
    pub schema_version: u32,
    /// Discriminator. Always
    /// `BTS_SNAPSHOT_REPORT_KIND` (=
    /// `"debian.issue.bts_snapshot_v1"`).
    pub report_kind: String,
    /// The §17 tri-state (minus `Fail` per §24).
    /// `pass` for a successful snapshot (regardless
    /// of how many bugs); `infrastructure_error`
    /// for any transport failure (DNS / TLS /
    /// timeout / SOAP / malformed service
    /// response).
    pub verdict: IssueVerdict,
    /// The source package name the snapshot was
    /// taken against. Mirrors the 1E.1 shim's
    /// `source_package` field. Empty when the
    /// snapshot could not be retrieved
    /// (`verdict: "infrastructure_error"`).
    #[serde(default)]
    pub source_package: String,
    /// The RFC3339 UTC timestamp of when the
    /// snapshot was taken. Mirrors the 1E.1
    /// shim's `snapshot_at` field. Empty when the
    /// snapshot could not be retrieved.
    #[serde(default)]
    pub snapshot_at: String,
    /// The list of bugs the BTS reported for
    /// `source_package`. Empty when the snapshot
    /// could not be retrieved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bugs: Vec<BugRecord>,
    /// The pre-computed summary fields. Mirrors the
    /// 1E.1 shim's `summary` field. `BugSummary::default()`
    /// when the snapshot could not be retrieved.
    #[serde(default)]
    pub summary: BugSummary,
}

/// The 1E.2 verdict tri-state. `Fail` is intentionally
/// absent: §24 says the BTS snapshot has no `Fail`
/// semantics — a snapshot either succeeded (the
/// runtime observes the bug data) or the transport
/// failed (the runtime gets `InfrastructureError →
/// Blocked`). The `Pass` and `InfrastructureError`
/// cases are the only two the 1E.2 normalizer
/// distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueVerdict {
    /// The snapshot was retrieved successfully. The
    /// runtime records `EvidenceStatus::Pass`; the
    /// `bugs` list and `summary` (in the GREEN
    /// struct) carry the observation regardless of
    /// how many bugs there are — §24 "A package
    /// having 50 open bugs is not a tool failure."
    Pass,
    /// The snapshot could not be retrieved
    /// (DNS / TLS / timeout / SOAP unavailable /
    /// malformed service response). The runtime
    /// records `NormalizationError::Unclassifiable(...)`
    /// which becomes `ExecutorError::InfrastructureFailed`
    /// which becomes `GateStatus::Blocked` (§24).
    /// The state-machine recovery path is
    /// 1F.1 / 1G.1's work.
    InfrastructureError,
}

impl DebianIssueReportV1 {
    /// The schema-version constant for the type.
    #[must_use]
    pub const fn schema_version() -> u32 {
        BTS_SNAPSHOT_SCHEMA_VERSION
    }

    /// The report-kind constant for the type.
    #[must_use]
    pub const fn report_kind() -> &'static str {
        BTS_SNAPSHOT_REPORT_KIND
    }

    /// Build a new report with the `report_kind` set
    /// to the constant. Callers do not need to spell
    /// the literal. The comprehensive §23 fields are
    /// populated from the arguments.
    #[must_use]
    pub fn with_kind(
        schema_version: u32,
        verdict: IssueVerdict,
        source_package: String,
        snapshot_at: String,
        bugs: Vec<BugRecord>,
        summary: BugSummary,
    ) -> Self {
        Self {
            schema_version,
            report_kind: BTS_SNAPSHOT_REPORT_KIND.to_string(),
            verdict,
            source_package,
            snapshot_at,
            bugs,
            summary,
        }
    }
}
