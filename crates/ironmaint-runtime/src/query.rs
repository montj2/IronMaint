//! Runtime queries (read-only).

use ironmaint_core::{ArtifactId, CandidateFingerprint, CheckId, EvidenceId, JobId, OperationId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeQuery {
    GetJob {
        job_id: JobId,
    },
    GetProjection {
        job_id: JobId,
    },
    ListNextActions {
        job_id: JobId,
    },
    /// Read back what a check produced: the evidence row it wrote
    /// and the gate verdict that evidence contributed to.
    GetCheckOutcome {
        check_id: CheckId,
    },
    /// Read a `PrivilegedOperation` by id.
    ///
    /// Routed through the runtime rather than a store handle the
    /// MCP layer holds, per PHASE-0B.md §98.6: the MCP surface
    /// reaches persistence through the runtime, never around it.
    GetOperation {
        operation_id: OperationId,
    },
    /// The release candidate for a job's *active* source candidate.
    ///
    /// Addressed by job rather than by id because a `JobId` is what
    /// a caller has: the id is minted by
    /// [`crate::RuntimeCommand::CreateReleaseCandidate`], so a
    /// caller that arrived first cannot know it. Since that command
    /// is idempotent on (job, fingerprint) there is at most one
    /// answer, and asking for it is how a caller finds out whether
    /// one has been assembled at all.
    GetReleaseCandidate {
        job_id: JobId,
    },
    /// PHASE-1.md §31 — the `evidence.list` MCP tool. List
    /// `Evidence` rows for a job, optionally narrowed to one
    /// candidate fingerprint. The job_id is required so a
    /// caller can never enumerate evidence across jobs
    /// without explicitly naming them; the optional
    /// fingerprint is a second scope check, not a way to
    /// widen.
    ListEvidence {
        job_id: JobId,
        candidate_fingerprint: Option<CandidateFingerprint>,
    },
    /// PHASE-1.md §31 — the `evidence.get` MCP tool. Read a
    /// single `Evidence` row by its `EvidenceId`. The
    /// runtime resolves the id and returns the row verbatim.
    GetEvidence {
        evidence_id: EvidenceId,
    },
    /// PHASE-1.md §31 — the `evidence.artifact.read` MCP
    /// tool. Read the bytes of a `Report` artifact bound to
    /// a specific `Evidence` row. The runtime looks the
    /// `ArtifactRef` up on the row, validates that the
    /// caller named a real `ArtifactId` and not an
    /// arbitrary digest, and returns the on-disk bytes
    /// through the artifact store. The MCP layer receives
    /// the bytes as base64 in a JSON object alongside the
    /// artifact's declared `media_type` (PHASE-0B.md §15:
    /// artifact refs carry the type, not just the digest).
    ReadEvidenceArtifact {
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
    },
}
