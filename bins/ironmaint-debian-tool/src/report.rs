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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangelogIdentity {
    pub source: String,
    pub version: String,
    pub distribution: String,
    pub urgency: String,
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
