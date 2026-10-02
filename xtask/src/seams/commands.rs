//! **S1 — every declared `RuntimeCommand` has a production caller.**
//!
//! The assertion is *construction*, not mention. `RuntimeCommand` is
//! dispatched by `RuntimeService::handle_command`, whose twelve match arms each
//! spell a variant name followed by `{` — so a text search for `VariantName {`
//! finds all twelve and reports the seam as closed when it is wide open. The
//! difference is a single `=>`, and the difference is the check.
//!
//! ## The allowlist
//!
//! A variant may be excused, with a reason naming a spec section or a design
//! fact. Reasons here are not filler. Two of the seven are defects recorded in
//! `doc/DEBT.md` (D-20) rather than decisions, and their reasons say so; a
//! reader who disagrees with a verdict can act on the entry, whereas a reader
//! looking at "reserved" has nothing to push back against.
//!
//! Note what the allowlist costs. S8 — "constructible through a public surface,
//! not just a test" — only applies to variants S1 *clears*, so excusing a
//! variant here removes it from S8's sight as well. A command that nothing
//! constructs is by definition not reachable through a registered tool, which
//! makes the two checks agree rather than disagree. That is why the
//! allowlist's reasons are written to survive being read by someone who has
//! not read this file.

use std::collections::BTreeMap;

use syn::visit::Visit;

use super::{CheckSummary, SeamViolation, SourceFile, find_declaring_file};

/// The enum whose variants this check requires a production construction for.
const ENUM: &str = "RuntimeCommand";

/// `(variant, reason)` pairs for variants that are legitimately never
/// constructed by production code.
///
/// A blank or hedged reason is a review rejection: the point of the table is
/// that a reader can check the claim, and "reserved for later" cannot be
/// checked.
const ALLOWLIST: &[(&str, &str)] = &[
    (
        "MaterializeChecks",
        "Reached by a direct call to `RuntimeService::handle_materialize_checks` \
         from `handle_capture_candidate` (service.rs:1622), which is the only \
         flow that materialises checks — capture derives the adapter plan in \
         the same operation. The command variant is a parallel declaration of \
         the same inputs, not the production path.",
    ),
    (
        "SetActiveCandidate",
        "Reached by a direct call to `handle_set_active_candidate` from \
         `handle_capture_candidate` (service.rs:1712), so activation happens as \
         part of capture. D-16's `JobEvent::SetActiveCandidate` exists to make \
         that binding durable; dispatching it as a separate command would let \
         an agent activate a candidate the capture flow never bound.",
    ),
    (
        "RecordCheckEvidence",
        "A decision, recorded at the call site rather than here. The recording          does happen in production — `handle_run_check` writes the gate result          itself (service.rs:2417) — but it does so inline, and the reason is          written down where the code is: \"There is deliberately no MCP tool          for this: §53 forbids `ironmaint_obligation_set_pass`\". The command          is the interface; the flow calls the handler directly. A settable          command here would be the forbidden tool with an extra hop.",
    ),
    (
        "RecordObligationOutcome",
        "As `RecordCheckEvidence`, and by the same reasoning:          `handle_run_check` derives the obligation verdict inline via          `derive_obligation_verdicts` (service.rs:2431), and the comment there          states the consequence of not doing so — \"Without this step a          mandatory obligation could only ever be `NotEvaluated` over the tool          surface, and a job could never reach `ReadyForApproval`\". So the          capability the variant exists to express is live; only the command          wrapper is unconstructed.",
    ),
    (
        "RequestApproval",
        "A correct refusal, deliberately unreachable as a write (§0B 0B.10 C1          and the command's own doc): 0B ships no approval principal, no          delivery channel and no durable `ApprovalStore`, so implementing the          write would record a decision nobody made. **D-20 is the command's          doc, not the refusal.** The doc claims `next_actions` advertises          `RequestApproval` and that an agent following it calls this and reads          the refusal. `ReadyForApproval` emits `allowed: vec![]` and          `requires_human: ApproveRelease` (service.rs:2706), and          `RequestApproval` is not an `AllowedAction` at all — so the doc          describes a design that no longer exists.",
    ),
    (
        "Reconcile",
        "The MCP dispatcher calls `RuntimeService::reconcile(job_id)` directly \
         (mcp/src/dispatch.rs:214) rather than dispatching the command, so the \
         operation is reachable and the command variant is redundant. §41 \
         defines `reconcile` per-call; there is no background loop to call it.",
    ),
    (
        "EnterHumanReview",
        "Deliberate, and said so twice: \"escalation is an orchestration \
         decision, and an agent that could raise it against itself could also \
         strand itself\" (mcp/tests/dispatcher.rs:204), and \"0A §21 gives it \
         no entry point, so the runtime cannot yet produce this projection\" \
         (next_actions.rs). **D-20 is the consequence, not the decision:** the \
         only `ResumeRecorded` writer in the workspace is this handler, so no \
         production path writes the record `attach_resume_action` needs — which \
         makes the registered `job.resume` tool unable to succeed.",
    ),
];

pub struct CommandsCheck;

/// What one check found: its counts for the report, and its failures.
pub struct CheckOutcome {
    pub summary: CheckSummary,
    pub violations: Vec<SeamViolation>,
}

impl CheckOutcome {
    pub fn empty() -> Self {
        Self {
            summary: CheckSummary::default(),
            violations: Vec::new(),
        }
    }
}

impl CommandsCheck {
    pub fn run(files: &[SourceFile]) -> CheckOutcome {
        let Some(decl) = find_declaring_file(files, ENUM) else {
            return CheckOutcome::empty();
        };
        let variants: Vec<String> = super::enum_variants(&decl.syn, ENUM)
            .iter()
            .map(ToString::to_string)
            .collect();

        let mut constructed: BTreeMap<String, usize> = BTreeMap::new();
        for file in files {
            if file.is_test {
                continue;
            }
            for variant in &variants {
                let n = Self::constructions_in(&file.syn, ENUM, variant);
                *constructed.entry(variant.clone()).or_default() += n;
            }
        }

        let mut violations = Vec::new();
        let mut allowlisted = 0usize;
        for variant in &variants {
            if constructed.get(variant).copied().unwrap_or(0) > 0 {
                continue;
            }
            match ALLOWLIST.iter().find(|(v, _)| v == variant) {
                Some(_) => allowlisted += 1,
                None => violations.push(SeamViolation {
                    check: "S1",
                    subject: format!("{ENUM}::{variant}"),
                    detail: "no production code constructs this variant; nothing can \
                             send it. Either wire a caller or add a reasoned entry to \
                             ALLOWLIST in xtask/src/seams/commands.rs."
                        .to_owned(),
                }),
            }
        }

        CheckOutcome {
            summary: CheckSummary {
                checked: variants.len(),
                allowlisted,
            },
            violations,
        }
    }

    /// How many `Expr` in `file` construct `Enum::Variant`.
    ///
    /// `enum_path` may be empty, which means "a bare `Variant`", so a crate
    /// that stops qualifying its paths does not silently stop being checked.
    pub fn constructions_in(file: &syn::File, enum_path: &str, variant: &str) -> usize {
        let mut finder = ConstructionFinder {
            enum_path,
            variant,
            found: 0,
        };
        finder.visit_file(file);
        finder.found
    }
}

/// Counts construction *expressions* of one enum variant.
///
/// The distinction the whole check rests on: a `match` arm and a struct
/// literal are both `Path { … }` to syn, separated only by the `=>` that
/// follows. A `Pat` is never an expression, so visiting `Expr` and not `Pat`
/// is the structural form of the rule — no token matching, no heuristics.
struct ConstructionFinder<'a> {
    enum_path: &'a str,
    variant: &'a str,
    found: usize,
}

impl<'ast> Visit<'ast> for ConstructionFinder<'_> {
    fn visit_expr_struct(&mut self, node: &syn::ExprStruct) {
        if node.attrs.iter().any(super::is_cfg_test) {
            return;
        }
        if node
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == self.variant)
            && self.qualifies(node)
        {
            self.found += 1;
        }
        syn::visit::visit_expr_struct(self, node);
    }
}

impl ConstructionFinder<'_> {
    /// Whether `node`'s path names *this* enum rather than some other type's
    /// same-named variant.
    ///
    /// `RuntimeCommand::CreateJob` puts the enum in `segments[0]` and the
    /// variant in the last position, so the enum is a segment to *look for*,
    /// not the tail to *compare*. A bare `CreateJob { … }` — one segment, no
    /// qualifier — is counted too, on the grounds that the variants are
    /// distinctive enough that a collision would need a second type with a
    /// variant of the same name, and because the failure modes are not
    /// symmetric: a missed construction is a false alarm, while a spurious
    /// one is a seam that has gone quiet.
    fn qualifies(&self, node: &syn::ExprStruct) -> bool {
        let Some(enum_tail) = self.enum_path.rsplit("::").next() else {
            return true;
        };
        if enum_tail.is_empty() {
            return true;
        }
        node.path.segments.iter().any(|s| s.ident == enum_tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).expect("parse")
    }

    #[test]
    fn a_match_arm_is_not_a_construction() {
        // The single most important case in this module. Every arm of
        // `handle_command` looks like a construction to a text search.
        let f = parse(
            "enum RuntimeCommand { CreateJob { a: u8 }, Reconcile { b: u8 } }\
             fn f(c: RuntimeCommand) { match c {\
                 RuntimeCommand::CreateJob { a } => {},\n\
                 RuntimeCommand::Reconcile { b } => {},\n\
             } }",
        );
        assert_eq!(
            CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob"),
            0
        );
        assert_eq!(
            CommandsCheck::constructions_in(&f, "RuntimeCommand", "Reconcile"),
            0
        );
    }

    #[test]
    fn a_struct_literal_inside_a_function_is_a_construction() {
        let f = parse(
            "enum RuntimeCommand { CreateJob { a: u8 } }\
             fn f() { let _ = RuntimeCommand::CreateJob { a: 1 }; }",
        );
        assert_eq!(
            CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob"),
            1
        );
    }

    #[test]
    fn a_construction_nested_in_a_call_is_still_counted() {
        let f = parse(
            "enum RuntimeCommand { CreateJob { a: u8 } }\
             fn f() { g(RuntimeCommand::CreateJob { a: 1 }); }",
        );
        assert_eq!(
            CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob"),
            1
        );
    }

    #[test]
    fn a_different_variants_construction_does_not_count() {
        let f = parse(
            "enum RuntimeCommand { CreateJob { a: u8 }, Reconcile { b: u8 } }\
             fn f() { let _ = RuntimeCommand::Reconcile { b: 1 }; }",
        );
        assert_eq!(
            CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob"),
            0
        );
    }

    #[test]
    fn a_fully_qualified_path_counts_as_construction() {
        let f = parse(
            "enum RuntimeCommand { CreateJob { a: u8 } }\
             fn f() { let _ = ironmaint_runtime::RuntimeCommand::CreateJob { a: 1 }; }",
        );
        assert_eq!(
            CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob"),
            1
        );
    }

    #[test]
    fn a_pattern_in_a_let_is_not_a_construction() {
        let f = parse(
            "enum RuntimeCommand { CreateJob { a: u8 } }\
             fn f(c: RuntimeCommand) { if let RuntimeCommand::CreateJob { a } = c { } }",
        );
        assert_eq!(
            CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob"),
            0
        );
    }

    #[test]
    fn every_allowlist_entry_names_a_variant_that_exists() {
        // A reason for a variant that was renamed is a reason for nothing.
        let f = parse(
            "enum RuntimeCommand { CreateJob { a: u8 }, CaptureCandidate { b: u8 }, \
             MaterializeChecks { c: u8 }, SetActiveCandidate { d: u8 }, \
             RecordCheckEvidence { e: u8 }, RecordObligationOutcome { f: u8 }, \
             RunCheck { g: u8 }, RequestApproval { h: u8 }, Reconcile { i: u8 }, \
             EnterHumanReview { j: u8 }, ResumeJob { k: u8 }, \
             CreateReleaseCandidate { l: u8 } }",
        );
        let declared: Vec<String> = super::super::enum_variants(&f, ENUM)
            .iter()
            .map(ToString::to_string)
            .collect();
        for (variant, reason) in ALLOWLIST {
            assert!(
                declared.iter().any(|d| d == variant),
                "allowlist names `{variant}`, which is not a {ENUM} variant"
            );
            assert!(
                reason.len() > 40,
                "allowlist entry `{variant}` has no reason worth reading"
            );
        }
    }
}
