//! Evidence — the central object (§22).
//!
//! An [`Evidence`] value records "this producer (with this status,
//! at this time, optionally carrying these artifacts) attests to
//! something about this candidate." The state machine in
//! `ironmaint-state` consumes Evidence via [`GateDefinition`] /
//! [`GateResult`] (§27–§29); adapters and the policy engine produce it.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use ironmaint_core::{CandidateFingerprint, EvidenceId, OperationId, SchemaVersion};

use crate::artifact::ArtifactRef;
use crate::scope::EvidenceScope;

const NOTES_MAX: usize = 4096;

/// Status of an evidence evaluation (§23).
///
/// `Pass` and `Fail` are the normal outcomes; `NotApplicable` exempts
/// the producing gate; `Inconclusive` is "we ran the check but the
/// result wasn't actionable" (different from `NotEvaluated` which means
/// "we haven't run it yet"); `InfrastructureError` is a host/sandbox
/// problem that must not be confused with `Fail` (§2.6:
/// "Tool failure ≠ infrastructure error").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Pass,
    Fail,
    NotApplicable,
    Inconclusive,
    InfrastructureError,
}

impl fmt::Display for EvidenceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl EvidenceStatus {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::NotApplicable => "not_applicable",
            Self::Inconclusive => "inconclusive",
            Self::InfrastructureError => "infrastructure_error",
        }
    }

    /// Whether this status, combined with a `Pass`-only
    /// [`crate::gate::RequiredEvidenceStatus`], satisfies a gate.
    #[must_use]
    pub fn satisfies_pass_only(self) -> bool {
        matches!(self, Self::Pass)
    }

    /// Whether this status, combined with a `PassOrNotApplicable`
    /// [`crate::gate::RequiredEvidenceStatus`], satisfies a gate.
    #[must_use]
    pub fn satisfies_pass_or_not_applicable(self) -> bool {
        matches!(self, Self::Pass | Self::NotApplicable)
    }
}

/// Producer of an evidence value (§24).
///
/// Examples (none hard-coded into the type): `sbuild 0.x`, `lintian
/// 2.x`, `mock 6.x`, `rpmlint 2.x`, `ironmaint-policy-engine 0.1`.
/// Core sees them as opaque producer labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceProducer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<OperationId>,
}

impl EvidenceProducer {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
            execution_id: None,
        }
    }

    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    #[must_use]
    pub fn with_execution_id(mut self, id: OperationId) -> Self {
        self.execution_id = Some(id);
        self
    }
}

/// Kind of an evidence value (§26).
///
/// General categories only — individual tools are not encoded into
/// the enum. A tool like `lintian` or `rpmlint` is captured in the
/// [`EvidenceProducer::name`], not in the kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    SourceIntegrity,
    Build,
    PackageQa,
    FunctionalTest,
    UpgradeTest,
    Reproducibility,
    PolicyEvaluation,
    LicenseReview,
    IssueCorrelation,
    ReleaseAssembly,
    PublicationValidation,
    /// Escape hatch for kinds the core vocabulary doesn't enumerate.
    Other(String),
}

impl fmt::Display for EvidenceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl EvidenceKind {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::SourceIntegrity => "source_integrity",
            Self::Build => "build",
            Self::PackageQa => "package_qa",
            Self::FunctionalTest => "functional_test",
            Self::UpgradeTest => "upgrade_test",
            Self::Reproducibility => "reproducibility",
            Self::PolicyEvaluation => "policy_evaluation",
            Self::LicenseReview => "license_review",
            Self::IssueCorrelation => "issue_correlation",
            Self::ReleaseAssembly => "release_assembly",
            Self::PublicationValidation => "publication_validation",
            Self::Other(s) => s,
        }
    }
}

/// A single piece of evidence about a candidate (§22, §30).
///
/// `candidate` is the binding (§30): "Every evidence object is
/// candidate-bound by default. Phase 0A should deliberately **not**
/// implement cross-candidate evidence reuse." Cross-candidate reuse
/// is a later optimization; for 0A.3 every evidence's `candidate`
/// must match the gate it's intended to satisfy or
/// [`crate::gate::GateResult`] will refuse it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub id: EvidenceId,
    pub candidate: CandidateFingerprint,
    pub kind: EvidenceKind,
    pub status: EvidenceStatus,
    pub producer: EvidenceProducer,
    /// Wire-format schema version (§60). Always serializes as
    /// `SchemaVersion::V1`; on parse, a missing field defaults to `V1`.
    #[serde(default)]
    pub schema_version: SchemaVersion,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    pub scope: EvidenceScope,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "ironmaint_core::json_schema_impls::Rfc3339DateTime")]
    pub observed_at: OffsetDateTime,
    /// Free-form human notes. Capped at 4 KiB; not interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Evidence {
    /// Construct an [`Evidence`] with the minimum required fields.
    ///
    /// # Errors
    /// Returns `CoreError { kind: InvalidName, .. }` if `notes` exceeds
    /// [`NOTES_MAX`] bytes.
    pub fn new(
        candidate: CandidateFingerprint,
        kind: EvidenceKind,
        status: EvidenceStatus,
        producer: EvidenceProducer,
        scope: EvidenceScope,
        observed_at: OffsetDateTime,
    ) -> Self {
        Self {
            id: EvidenceId::new(),
            candidate,
            kind,
            status,
            producer,
            schema_version: SchemaVersion::default(),
            artifacts: Vec::new(),
            scope,
            observed_at,
            notes: None,
        }
    }

    #[must_use]
    pub fn with_artifact(mut self, artifact: ArtifactRef) -> Self {
        self.artifacts.push(artifact);
        self
    }

    /// Set the notes field, returning `Err(())` if too long.
    ///
    /// We surface this as a `Result<(), &'static str>` rather than
    /// `CoreError` because notes are user-supplied free-form text and
    /// callers may want to truncate-with-warning rather than fail.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Result<Self, &'static str> {
        let s = notes.into();
        if s.len() > NOTES_MAX {
            return Err("notes exceed 4096 bytes");
        }
        self.notes = Some(s);
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironmaint_core::JobId;
    use time::macros::datetime;

    fn fp() -> CandidateFingerprint {
        CandidateFingerprint::from_hex("a".repeat(64)).unwrap()
    }

    #[test]
    fn status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EvidenceStatus::InfrastructureError).unwrap(),
            r#""infrastructure_error""#
        );
        assert_eq!(
            serde_json::to_string(&EvidenceStatus::NotApplicable).unwrap(),
            r#""not_applicable""#
        );
    }

    #[test]
    fn status_satisfies_predicates() {
        assert!(EvidenceStatus::Pass.satisfies_pass_only());
        assert!(!EvidenceStatus::Fail.satisfies_pass_only());
        assert!(EvidenceStatus::Pass.satisfies_pass_or_not_applicable());
        assert!(EvidenceStatus::NotApplicable.satisfies_pass_or_not_applicable());
        assert!(!EvidenceStatus::Fail.satisfies_pass_or_not_applicable());
        assert!(!EvidenceStatus::Inconclusive.satisfies_pass_or_not_applicable());
        assert!(!EvidenceStatus::InfrastructureError.satisfies_pass_or_not_applicable());
    }

    #[test]
    fn producer_builder_sets_optionals() {
        let p = EvidenceProducer::new("sbuild")
            .with_version("0.85.5")
            .with_execution_id(OperationId::new());
        assert_eq!(p.name, "sbuild");
        assert_eq!(p.version.as_deref(), Some("0.85.5"));
        assert!(p.execution_id.is_some());
    }

    #[test]
    fn kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EvidenceKind::SourceIntegrity).unwrap(),
            r#""source_integrity""#
        );
    }

    #[test]
    fn kind_other_carries_string() {
        let k = EvidenceKind::Other("custom-check".to_string());
        let json = serde_json::to_string(&k).unwrap();
        let parsed: EvidenceKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, k);
    }

    #[test]
    fn evidence_minimum_construction() {
        let e = Evidence::new(
            fp(),
            EvidenceKind::Build,
            EvidenceStatus::Pass,
            EvidenceProducer::new("sbuild"),
            EvidenceScope::Job(JobId::new()),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        assert!(e.artifacts.is_empty());
        assert!(e.notes.is_none());
    }

    #[test]
    fn evidence_notes_rejects_oversize() {
        let e = Evidence::new(
            fp(),
            EvidenceKind::Build,
            EvidenceStatus::Pass,
            EvidenceProducer::new("sbuild"),
            EvidenceScope::Job(JobId::new()),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let too_long = "x".repeat(NOTES_MAX + 1);
        assert!(e.with_notes(too_long).is_err());
    }

    #[test]
    fn evidence_notes_accepts_at_limit() {
        let e = Evidence::new(
            fp(),
            EvidenceKind::Build,
            EvidenceStatus::Pass,
            EvidenceProducer::new("sbuild"),
            EvidenceScope::Job(JobId::new()),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let just_right = "x".repeat(NOTES_MAX);
        assert!(e.clone().with_notes(just_right).is_ok());
    }

    #[test]
    fn evidence_round_trips() {
        let e = Evidence::new(
            fp(),
            EvidenceKind::Build,
            EvidenceStatus::Pass,
            EvidenceProducer::new("sbuild").with_version("0.85"),
            EvidenceScope::Candidate(fp()),
            datetime!(2026-01-01 00:00:00 UTC),
        )
        .with_notes("all good")
        .unwrap();
        let json = serde_json::to_string(&e).unwrap();
        let parsed: Evidence = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, e);
    }

    #[test]
    fn evidence_serializes_rfc3339_observed_at() {
        let e = Evidence::new(
            fp(),
            EvidenceKind::Build,
            EvidenceStatus::Pass,
            EvidenceProducer::new("sbuild"),
            EvidenceScope::Job(JobId::new()),
            datetime!(2026-01-01 00:00:00 UTC),
        );
        let json = serde_json::to_string(&e).unwrap();
        // RFC3339 UTC: "2026-01-01T00:00:00Z"
        assert!(json.contains("\"2026-01-01T00:00:00Z\""), "got: {json}");
    }
}
