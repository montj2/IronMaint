//! `CheckOutcome` — the domain result of running one check.
//!
//! `check.run` needs to hand an orchestrator the *durable* facts
//! about a check, not the executor's raw bytes. The 0B.6 stub
//! returned `exit_code`/`stdout`/`stderr` shaped output that nothing
//! persisted, and `SKILL.md` explicitly forbids the agent from
//! acting on raw tool output. What survives a restart is the
//! evidence row and the gate verdict, so those are what this type
//! carries.

use ironmaint_core::{CandidateFingerprint, CheckId, EvidenceId, GateId, JobId};
use ironmaint_evidence::{EvidenceStatus, GateStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What one check produced: the evidence it recorded and the gate
/// verdict that evidence contributed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckOutcome {
    /// The job the check belongs to.
    pub job_id: JobId,
    /// The check that was run.
    pub check_id: CheckId,
    /// The gate this check's evidence rolls up into.
    pub gate_id: GateId,
    /// The candidate the evidence is bound to. §30 freshness
    /// means this is always the *active* candidate at run time.
    pub candidate: CandidateFingerprint,
    /// The `<adapter-namespace>.<role>.<tool>` key that executed.
    pub tool_key: String,
    /// The evidence row this run wrote, or `None` if the check has
    /// never been run.
    ///
    /// `Option` rather than a sentinel status: `EvidenceStatus` has
    /// no "not evaluated" member, because *evidence* is always the
    /// result of something that actually ran. "Not run yet" is
    /// carried here, and the gate verdict below is
    /// `GateStatus::NotEvaluated` in that case. Keeping the two
    /// distinct is what stops an orchestrator reading a pending
    /// gate as a failed one.
    pub evidence_id: Option<EvidenceId>,
    /// The evidence's own status — the tool's verdict, before gate
    /// aggregation folded it together with sibling checks.
    pub evidence_status: Option<EvidenceStatus>,
    /// The gate verdict after aggregation.
    pub gate_status: GateStatus,
    /// Whether the executor bounded stdout/stderr (PHASE-0B.md
    /// §15 — a truncated PASS is still a Pass).
    pub truncated: bool,
}
