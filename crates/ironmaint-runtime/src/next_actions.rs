//! `JobNextActions` (PHASE-0B.md §42).
//!
//! Materialises the *next allowed moves* for the orchestrator: the
//! [`AllowedAction`]s it can take right now, the [`HumanAction`]
//! it must route to a person, and the [`ActionBlocker`]s that
//! explain why anything not listed is off-limits.
//!
//! # Why there are three fields
//!
//! The field is called `allowed` and its documented contract is "the
//! `AllowedAction`s it can take *right now*". That was false for
//! three of the seven variants it used to contain:
//! `RecordObligationOutcome`, `RequestApproval` and
//! `AuthorizeOperation` were emitted with no MCP tool behind them,
//! and `RuntimeCommand::RequestApproval` is not merely tool-less but
//! actively refused. An agent following the field's contract would
//! call a tool that does not exist.
//!
//! The human cases now live in [`HumanAction`], on their own field,
//! rather than in [`ActionBlocker`]. They are not blockers in the
//! sense that word means here — a blocker explains why a move is
//! unavailable, and these moves are available, just not to an
//! agent. Filing them as blockers is what produced
//! `SKILL.md`'s own warning that there is no `missing_approval`
//! blocker and an agent should not "wait for a blocker that will
//! never arrive".

use ironmaint_core::{CheckId, JobId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A move the orchestrator can perform right now, by calling a tool.
///
/// Every variant has exactly one MCP tool behind it, and
/// `ironmaint_mcp::tool_for_action` maps each to its name. The match
/// there is exhaustive with no wildcard arm, so adding a variant
/// without a tool is a compile error rather than a silent lie — the
/// property this enum exists to have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AllowedAction {
    CaptureCandidate,
    RunCheck {
        check_id: CheckId,
    },
    ApplyPatch,
    /// Return a job in an exceptional state to the state named by
    /// its recorded resume state (0A §21).
    ///
    /// Emitted only when such a record actually exists. The pure
    /// projection cannot know that, so the runtime attaches it
    /// from the query handler after reading the log — advertising a
    /// move that would be refused would make `allowed` a lie, and
    /// this field's documented contract is "the `AllowedAction`s it
    /// can take *right now*".
    ///
    /// Being listed is not an instruction to take it. `allowed` says
    /// what a tool can perform; whether the caller *should* is a
    /// separate question, and `SKILL.md` tells an agent not to: the
    /// recorded target is the escalator's decision, so resuming is an
    /// operator's move. The two answers are deliberately different
    /// fields — `requires_human: ReviewEscalation` carries the second.
    ///
    /// **No production job reaches this.** `job.resume` is a registered
    /// tool and it works, but the `ResumeRecord` it reads is written
    /// only by `RuntimeCommand::EnterHumanReview`, and nothing in
    /// production constructs that command — the sole construction site
    /// is `crates/ironmaint-mcp/tests/dispatcher.rs`. So the record
    /// never exists, the handler refuses with `InvalidInput`, and the
    /// tool cannot succeed on any job a real agent can reach. Only
    /// tests, which dispatch `EnterHumanReview` directly, can get here.
    ///
    /// This is the one honest thing to say about the two states at
    /// once: the *shape* above is right, and there is no production
    /// path that exercises it. Who may escalate a job, and on what
    /// evidence, is an open architecture question — D-20, and §17
    /// question 8 of `doc/PHASE-0B-COMPLETION.md`. When that is
    /// answered, this doc's first paragraph is what a caller should
    /// read.
    ResumeJob,
}

/// A move that is available but belongs to a person, not an agent.
///
/// 0B ships no tool for any of these, and for two of them shipping
/// one is forbidden: §4.10/§26/§99 stop the privileged producers
/// (`RecordApprovalDecision`, `AuthorizeOperation`) from existing in
/// this phase at all. Reporting them as `allowed` would advertise a
/// capability the phase deliberately withholds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HumanAction {
    /// A mandatory policy obligation must be evaluated and
    /// satisfied before the job can advance.
    SatisfyObligation {
        /// The obligation's `requirement` string, when one is
        /// known. `None` at the pre-capture states, where no
        /// adapter has derived an obligation for the job yet.
        reference: Option<String>,
    },
    /// The job is complete and awaits explicit human approval
    /// (0A §35: the agent cannot grant its own approval).
    ApproveRelease,
    /// A privileged operation awaits authorization. No tool can
    /// perform this in 0B, and §4.10/§26/§99 forbid adding one.
    AuthorizePublication,
    /// The job was escalated to `HumanReviewRequired` (0A §21) and
    /// is waiting on a person. `job.resume` — which returns the job
    /// to the state recorded at escalation — is a *human-side* move
    /// in 0B: `SKILL.md` tells the agent to stop rather than take
    /// it, because the recorded state is the escalator's decision
    /// and the agent has no basis to overrule it.
    ///
    /// When a job is genuinely in this state it also lists
    /// `AllowedAction::ResumeJob`, because a tool for it exists. This
    /// variant is the reason that listing is not an instruction:
    /// `allowed` answers "what is performable", `requires_human`
    /// answers "whose move is it".
    ///
    /// **No production job reaches this either, and for the same
    /// reason** — see `AllowedAction::ResumeJob`. The projection
    /// produces this variant for `HumanReviewRequired` because the
    /// state exists in the machine, but nothing enters that state in
    /// production, so the pairing described above is exercised only by
    /// tests. Whether "nothing enters it" is correct is an open
    /// architecture question, not a fact this enum can settle.
    ReviewEscalation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionBlocker {
    /// Job is already in a terminal state; no further moves.
    Terminal,
    /// A mandatory gate's evidence is still `NotEvaluated`.
    ///
    /// `tool_key` is `None` when the job has no materialised check
    /// for the pending gate, so there is no tool to name. It used
    /// to be a hardcoded `"synthetic.build.validate"`, which put a
    /// fixture string into a domain projection: a real job with no
    /// adapter behind it was told to run a tool that does not
    /// exist. The honest answer when nothing is materialised is to
    /// say so, and the handler fills this in from the store when a
    /// check does exist.
    GatePending { tool_key: Option<String> },
    /// A mandatory gate's evidence is `Fail` and no exception
    /// is on file.
    GateFailed { tool_key: String },
    /// An obligation with `RequiresReview` is unsatisfied.
    ObligationPending { reference: String },
    /// The active candidate has not been captured yet.
    NoActiveCandidate,
    /// The state machine refused the transition; details are
    /// in the message.
    StateMachineBlocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobNextActions {
    pub job_id: JobId,
    /// Moves an orchestrator can take right now, each backed by an
    /// MCP tool. Never contains an action a tool cannot perform.
    pub allowed: Vec<AllowedAction>,
    /// The move that belongs to a person, if the job is parked on
    /// one. `None` at every state that is not waiting on a human.
    pub requires_human: Option<HumanAction>,
    /// Why the moves not listed in `allowed` are unavailable.
    pub blockers: Vec<ActionBlocker>,
}

impl JobNextActions {
    #[must_use]
    pub fn empty(job_id: JobId) -> Self {
        Self {
            job_id,
            allowed: vec![],
            requires_human: None,
            blockers: vec![ActionBlocker::Terminal],
        }
    }
}
