//! `ironmaint-state` — the workflow state machine.
//!
//! Owns [`TransitionEngine`], [`StateTransitioned`] / [`JobEvent`],
//! the static [`TRANSITION_RULES`] table, and the blocker / decision
//! vocabulary. "Only crate allowed to determine whether a job moves
//! from state X to state Y" (PHASE-0A.md §4.4). No adapter may
//! bypass it; no agent may mutate [`ironmaint_core::JobState`]
//! directly.
//!
//! ## Phase 0A.3 status
//!
//! - [`TransitionEngine::evaluate`] is pure and synchronous; takes
//!   a [`TransitionContext`] and returns a [`TransitionDecision`].
//! - [`TransitionEngine::apply`] wraps an allowed decision in a
//!   [`StateTransitioned`] event. The caller persists the new
//!   projection (the engine never mutates the input).
//! - The static [`TRANSITION_RULES`] table covers the §20 forward
//!   graph. Exceptional targets (`HumanReviewRequired`,
//!   `InfrastructureBlocked`, `Cancelled`) and resume transitions
//!   out of exceptional states are matched in-engine; the
//!   [`crate::transition::ResumeRecord`] parameter gates resume.
//! - §72 scenario coverage lives in `tests/engine_scenarios.rs`.

#![forbid(unsafe_code)]
// Tests in this crate legitimately `.unwrap()` / `.expect()` on
// values whose constructors we've already validated. Allow the
// workspace lint exception for `cfg(test)` only.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::map_err_ignore,
    )
)]

pub mod engine;
pub mod event;
pub mod transition;

pub use engine::{SYNTHETIC_RULE_INDEX, TRANSITION_RULES, TransitionEngine};
pub use event::{JobEvent, StateTransitioned};
pub use transition::{
    AdapterRequirement, ResumeRecord, Transition, TransitionApplyError, TransitionBlocker,
    TransitionContext, TransitionDecision, TransitionRequest, TransitionRequirements,
    TransitionRule,
};
