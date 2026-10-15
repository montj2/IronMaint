//! `workspace.apply_patch` and `workspace.stat` tools.

use ironmaint_core::JobId;
use ironmaint_workspace::WorkspaceRevision;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyPatchInput {
    pub job_id: JobId,
    pub patch: String,
    pub expected_revision: WorkspaceRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyPatchOutput {
    pub new_revision: WorkspaceRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatInput {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatOutput {
    pub revision: WorkspaceRevision,
}

// PHASE-1.1 PR 1.1.G (source import, P2 #3).
//
// `workspace.apply_patch` mutates an already-initialised
// working tree; before 1.1.G, the only path that could
// populate the tree with *external* content was
// `candidate.capture`, which inspects whatever is already
// on disk. The agent that wanted to bring a new package's
// upstream in therefore had to either shell out from the
// host (which defeats the `PrivilegedOperation` model
// for everything else) or stage a tarball by hand. 1.1.G
// closes the gap with a controlled `source.import` tool:
// the runtime takes a URL, gates it against a configured
// allowlist, and `git clone`s it into the workspace. The
// next `candidate.capture` operates on the cloned tree.
//
// The allowlist lives in `RuntimeConfig` (so an operator
// can set it via `--source-import-allowlist` or
// `IRONMAINT_SOURCE_IMPORT_ALLOWLIST`) and defaults to
// empty — a closed-by-default allowlist is the only safe
// initial state for a tool that can pull arbitrary code
// from arbitrary remotes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceImportInput {
    pub job_id: JobId,
    /// The remote URL to clone. Must match an entry in
    /// the runtime's `source_import_allowlist` (prefix
    /// match — see the dispatcher's note for the
    /// semantics). 1.1.G only supports `file://` URLs in
    /// practice; HTTPS / SSH URLs are reachable through
    /// the same code path but the §97 gate does not test
    /// them (the test fixture is a local bare repo).
    pub url: String,
    /// The ref to check out (branch, tag, or commit).
    /// Defaults to the remote's default branch when
    /// `None`.
    #[serde(default)]
    pub ref_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceImportOutput {
    /// The handle directory the runtime created or
    /// reused for `job_id`. The caller uses this to
    /// drive `candidate.capture` next.
    pub workspace_handle: String,
}
