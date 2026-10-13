//! `evidence.list`, `evidence.get`, `evidence.artifact.read`.
//!
//! PHASE-1.md §31 — read-only evidence access. The runtime
//! is the only writer of `Evidence` rows; these tools are
//! the read side, addressed through the runtime (PHASE-0B
//! §98.6) so the MCP layer never holds a store handle.
//!
//! All three tools are deliberately *read-only*. The
//! schema is the durable form: the tool returns the same
//! `Evidence` shape a future maintainer-facing UI would
//! see, and the `evidence.artifact.read` tool returns the
//! bytes verbatim so a client can render a structured
//! report. The bytes are base64 in the JSON output;
//! `application/json` is the only media type the runtime
//! ever stores, and the tool carries it through
//! unchanged.
//!
//! `evidence.artifact.read` is the §31 exit-checkpoint
//! from 1A.3: an MCP client that ran `check.run` now has
//! an `evidence_id` and an `artifact_id`, and the tool
//! gives it the report bytes.

use ironmaint_core::{ArtifactId, CandidateFingerprint, EvidenceId, JobId};
use ironmaint_evidence::Evidence;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `evidence.list` input.
///
/// `candidate_fingerprint` is optional: when present, the
/// tool narrows the result to evidence bound to that
/// candidate. When absent, the tool returns every evidence
/// row for the job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListInput {
    pub job_id: JobId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_fingerprint: Option<CandidateFingerprint>,
}

/// `evidence.list` output: the durable list of evidence
/// rows. Newest-first, per `EvidenceStore::list_evidence_for_job`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListOutput {
    pub rows: Vec<Evidence>,
}

/// `evidence.get` input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetInput {
    pub evidence_id: EvidenceId,
}

/// `evidence.get` output: the durable evidence row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetOutput {
    pub evidence: Evidence,
}

/// `evidence.artifact.read` input.
///
/// The `artifact_id` is the `ArtifactRef::id` of one of
/// the artifacts bound to the named `evidence_id`. The
/// runtime refuses to serve bytes for an `artifact_id`
/// the named evidence row does not reference, so the
/// tool exposes the bytes of *that evidence row's*
/// artifacts, not an arbitrary digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadArtifactInput {
    pub evidence_id: EvidenceId,
    pub artifact_id: ArtifactId,
}

/// `evidence.artifact.read` output: the bytes of one
/// artifact plus the `media_type` the artifact was stored
/// with.
///
/// `bytes_base64` is the artifact payload encoded as
/// standard base64 (RFC 4648, no URL-safe alphabet,
/// no line wrapping). Clients decoding into a structured
/// report (the only kind the runtime currently stores)
/// should treat the result as a UTF-8 JSON document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadArtifactOutput {
    /// The artifact's content-addressed identity,
    /// round-tripped so a client can confirm the on-the-wire
    /// bytes are the bytes the runtime stored.
    pub digest: ironmaint_core::Digest,
    /// The `media_type` declared on the `ArtifactRef` at
    /// write time (PHASE-0B.md §15). `None` if the producer
    /// did not declare one — the tool passes the absence
    /// through rather than guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// The artifact bytes, base64-encoded.
    pub bytes_base64: String,
}
