//! **S8 — every advertised tool is dispatchable, and every dispatchable
//! tool is advertised.**
//!
//! S2 checks one direction: every `AllowedAction` names a registered tool. Its
//! own doc records that the other direction is deliberately *not* covered, and
//! gives the reason — "a tool *registered but not implemented* is a dispatch
//! arm", which it calls other territory. This is that territory.
//!
//! ## Why the gap is worth closing anyway
//!
//! `dispatch` matches on a `&str`:
//!
//! ```text
//! match tool.0.as_str() {
//!     "candidate.capture" => dispatch_capture(&runtime, input).await,
//!     // ...
//!     unknown => Err(McpError::Other(format!("unknown tool: {unknown}"))),
//! }
//! ```
//!
//! The final arm makes the match total, which is correct and necessary. It is
//! also what makes a *miss* compile. There is no error for `"checks.run"` where
//! `"check.run"` was meant: the daemon advertises `check.run`, has no arm for
//! it, and hands the agent `unknown tool: check.run` at call time.
//!
//! **What actually catches that today, stated precisely.** Every one of the
//! eleven tools is called by at least one test — checked while writing this,
//! and the check is why the claim below is narrower than the one this section
//! first made. So `cargo test --workspace --features integration` does fail on
//! a missing arm. That is luck with a cause, not a property: it holds because
//! the current test set happens to touch every tool, and nothing in the build
//! requires it to keep doing so. Add a tool and a schema but no arm, and the
//! only thing that fails is a test that happens to call it.
//!
//! So the argument for S8 is not "nothing catches this." It is that S8 catches
//! it in a check that needs no feature flag, no test to have been written, and
//! no call to have happened, and it says *which* tool and *which* arm rather
//! than surfacing `unknown tool` from a test that reads as being about
//! something else.
//!
//! ## The second half: constructions that no arm reaches
//!
//! A tool arm names a `dispatch_*` helper, and a `RuntimeCommand` is
//! constructed inside one. If a construction moves into a helper that no arm
//! calls — a refactor that leaves the old helper behind, or a new one added
//! speculatively — S1 still sees a production construction site and stays
//! green. The command is constructed, by code that never runs.
//!
//! This is S1's question asked in the other direction. S1 asks "is anything
//! outside the enum's own match arms constructing this variant?"; S8 asks "is
//! the thing that constructs it reachable from the tool surface?" A caller can
//! exist, be production, and be unreachable — which is D-14's shape, and the
//! reason a "there is a caller" answer is not sufficient on its own.
//!
//! ## What this does not claim
//!
//! Reachability here is *structural*: a construction inside a function the
//! dispatcher calls. It is not a call-graph analysis, and a helper called only
//! by another unreachable helper is still counted as reachable. S1 §1.1 already
//! records that a runtime tracer was considered and rejected for coverage of
//! *reached* code not being coverage of *reachable* code; this check has the
//! same limit and it is a limit worth stating rather than a defect. It closes
//! the arm-miss and the orphan-helper cases, which are the two that a rename
//! or a copy-paste actually produces.
//!
//! ## The orphan-helper assertion is about `pub` helpers, and only those
//!
//! The teeth-check established this rather than assuming it. Deleting an arm
//! whose helper is *private* never reaches S8: the workspace denies
//! `dead_code`, so an uncalled private function is a compile error
//! (`5f2076a`):
//!
//! ```text
//! error: function `dispatch_operation_get` is never used
//! note: requested on the command line with `-D dead-code`
//! ```
//!
//! Nothing is wrong with that — it is a stronger check than S8's. But it means
//! the lints already own the private case, and S8 is for the one they are blind
//! to: a `pub` fn in a `pub mod` gets no dead-code warning, so when it stops
//! being reachable nothing else in the build objects. That is the case worth
//! having a check for, and it is the only one this assertion can catch.

use std::collections::{BTreeMap, BTreeSet};

use syn::spanned::Spanned;
use syn::visit::Visit;

use super::{CheckOutcome, SeamViolation, SourceFile, is_test_path};

/// The dispatcher whose arms are the tool surface.
const DISPATCHER: &str = "dispatch";

/// The MCP crate's source file, relative to the workspace root.
const DISPATCH_FILE: &str = "crates/ironmaint-mcp/src/dispatch.rs";

/// One `"tool.name" => dispatch_thing(..)` arm, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arm {
    pub tool: String,
    /// The helper the arm calls, or `<not a call>` if it does not call one.
    pub target: String,
    pub loc: String,
}

/// The parsed shape of [`DISPATCHER`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dispatcher {
    pub arms: Vec<Arm>,
    /// Helper functions the arms call, e.g. `dispatch_capture`.
    pub reachable: BTreeSet<String>,
    /// Every `dispatch_*` function defined in the file, called or not.
    pub defined: BTreeSet<String>,
    /// Every `RuntimeCommand` construction, as (variant, enclosing function).
    pub constructions: Vec<(String, String)>,
    /// Whether the match has a catch-all arm. See the module doc: its absence
    /// is a compile-level change, its presence is why a typo is silent.
    pub has_wildcard: bool,
}

impl Dispatcher {
    /// Parse the dispatcher out of one file.
    ///
    /// A standalone function so the extraction is testable without a workspace
    /// on disk, matching how [`super::actions::ActionsCheck`] splits parsing
    /// from walking.
    #[must_use]
    pub fn parse(file: &syn::File, rel: &str) -> Self {
        let mut out = Self::default();

        for item in &file.items {
            let syn::Item::Fn(f) = item else { continue };
            let name = f.sig.ident.to_string();

            // `dispatch` is the entry point, not one of its own helpers, so
            // it is excluded here rather than being reported as an unarmed
            // helper by the check that consumes `defined`.
            if name.starts_with("dispatch") && name != DISPATCHER {
                out.defined.insert(name.clone());
            }

            if name == DISPATCHER
                && let Some(syn::Expr::Match(m)) = super::tail_expr(&f.block)
            {
                for arm in &m.arms {
                    match &arm.pat {
                        // `unknown => ..` and `_ => ..` are the catch-all.
                        //
                        // Both shapes, because syn parses a bare identifier
                        // as `Pat::Ident` and only a qualified path as
                        // `Pat::Path`. Accepting one and not the other
                        // reports a clean tree over a dispatcher that has a
                        // catch-all — the wrong way to be wrong, and it
                        // is exactly the mistake the first run of this
                        // check made.
                        syn::Pat::Ident(i) if i.ident == "unknown" || i.ident == "_" => {
                            out.has_wildcard = true;
                        }
                        syn::Pat::Path(p) if p.path.is_ident("unknown") || p.path.is_ident("_") => {
                            out.has_wildcard = true;
                        }
                        syn::Pat::Lit(syn::PatLit {
                            lit: syn::Lit::Str(s),
                            ..
                        }) => {
                            let loc = format!("{rel}:{}", arm.pat.span().start().line);
                            out.arms.push(Arm {
                                tool: s.value(),
                                target: called_helper(&arm.body)
                                    .unwrap_or_else(|| "<not a call>".to_owned()),
                                loc: loc.clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }

            // Any `RuntimeCommand::Variant { .. }` expression literal inside
            // this function, at any depth — the same tolerance S1 applies to
            // its own construction finder, so the two agree on what counts.
            let mut finder = CommandFinder {
                commands: Vec::new(),
            };
            finder.visit_block(&f.block);
            for variant in finder.commands {
                out.constructions.push((variant, name.clone()));
            }
        }

        out.reachable = out
            .arms
            .iter()
            .filter(|a| a.target != "<not a call>")
            .map(|a| a.target.clone())
            .collect();
        out
    }
}

/// The `dispatch_*` helper a match arm calls, if it calls exactly one.
fn called_helper(body: &syn::Expr) -> Option<String> {
    let mut finder = CallFinder { names: Vec::new() };
    finder.visit_expr(body);
    finder.names.into_iter().find(|n| n.starts_with("dispatch"))
}

/// Collects every called function name in an expression tree.
struct CallFinder {
    names: Vec<String>,
}

impl<'ast> Visit<'ast> for CallFinder {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*node.func
            && let Some(last) = p.path.segments.last()
        {
            self.names.push(last.ident.to_string());
        }
        syn::visit::visit_expr_call(self, node);
    }
}

/// Collects every `RuntimeCommand::Variant` struct-literal construction.
struct CommandFinder {
    commands: Vec<String>,
}

impl<'ast> Visit<'ast> for CommandFinder {
    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        if let Some(last) = node.path.segments.last() {
            let enum_name = node
                .path
                .segments
                .iter()
                .rev()
                .nth(1)
                .map_or(String::new(), |s| s.ident.to_string());
            if enum_name == "RuntimeCommand" {
                self.commands.push(last.ident.to_string());
            }
        }
        syn::visit::visit_expr_struct(self, node);
    }
}

pub struct ReachabilityCheck;

impl ReachabilityCheck {
    pub fn run(files: &[SourceFile]) -> CheckOutcome {
        let registered: BTreeSet<String> = ironmaint_mcp::schema::generate_all_schemas()
            .keys()
            .map(|k| k.0.clone())
            .collect();

        let Some(dispatcher) = parse_dispatcher(files) else {
            return CheckOutcome {
                summary: super::CheckSummary::default(),
                violations: vec![SeamViolation {
                    check: "S8",
                    subject: DISPATCH_FILE.to_owned(),
                    detail: format!(
                        "the MCP dispatcher could not be found or parsed. Without \
                         it there is no tool surface to check, and a check that \
                         silently inspects nothing reads as a pass. The file S8 is \
                         written against is `{DISPATCH_FILE}`; if it moved, this \
                         is where to say so rather than let the check go quiet."
                    ),
                }],
            };
        };

        let mut violations = Vec::new();

        if registered.is_empty() {
            violations.push(SeamViolation {
                check: "S8",
                subject: "registered tool set".to_owned(),
                detail: "the daemon advertises no tools at all, so the tool \
                         surface cannot be compared against its dispatcher. That \
                         is a failure of the thing S8 measures, not of the \
                         measurement."
                    .to_owned(),
            });
        }

        let armed: BTreeSet<String> = dispatcher.arms.iter().map(|a| a.tool.clone()).collect();

        // Advertised but undispatchable. The silent one.
        for tool in registered.difference(&armed) {
            violations.push(SeamViolation {
                check: "S8",
                subject: format!("advertised tool `{tool}` has no dispatch arm"),
                detail: format!(
                    "`{DISPATCHER}` has no arm for `{tool}`, so the daemon lists a \
                     tool it cannot serve. Because the match ends in a catch-all, \
                     this compiles, and the first sign of it is an `unknown tool: \
                     {tool}` error handed to whoever called it. Add the arm, or \
                     remove the tool from `generate_all_schemas()` and the schema \
                     snapshot."
                ),
            });
        }

        // Dispatchable but unadvertised.
        for arm in &dispatcher.arms {
            if !registered.contains(&arm.tool) {
                violations.push(SeamViolation {
                    check: "S8",
                    subject: format!("dispatch arm \"{}\" is not advertised", arm.tool),
                    detail: format!(
                        "`{}` dispatches `{}` for tool `{}`, but the daemon does \
                         not advertise that name, so no agent can reach it. \
                         (`{}` is the arm's location.)",
                        DISPATCHER, arm.target, arm.tool, arm.loc
                    ),
                });
            }
        }

        // An arm whose body is not a call to a helper S8 can name. Reported
        // rather than skipped, for S2's reason: silence is the dangerous case.
        for arm in &dispatcher.arms {
            if arm.target == "<not a call>" {
                violations.push(SeamViolation {
                    check: "S8",
                    subject: format!("dispatch arm \"{}\" is not a call", arm.tool),
                    detail: format!(
                        "the arm for `{}` at {} does not call a `dispatch_*` \
                         helper, so S8 cannot tell which functions it makes \
                         reachable. If the arm is genuinely inline, say so by \
                         keeping the call shape — the check is worth more than \
                         the tidiness of this one arm.",
                        arm.tool, arm.loc
                    ),
                });
            }
        }

        // A construction in a helper no arm calls.
        let mut unreachable: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (variant, func) in &dispatcher.constructions {
            if !dispatcher.reachable.contains(func) {
                unreachable.entry(func.as_str()).or_default().push(variant);
            }
        }
        for (func, variants) in &unreachable {
            violations.push(SeamViolation {
                check: "S8",
                subject: format!("{func} constructs a command no arm reaches"),
                detail: format!(
                    "`{func}` constructs {}, but no arm of `{DISPATCHER}` calls \
                     it, so the code that builds the command never runs. S1 passes \
                     this — a production construction site is all S1 asks for — and \
                     every test passes, because the tests call the helper directly. \
                     This is D-14's shape: the caller exists and leads nowhere.",
                    variants
                        .iter()
                        .map(|v| format!("`RuntimeCommand::{}`", v))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            });
        }

        // A helper defined and never armed at all. Weaker than the above — it
        // may construct nothing yet — so it is the same violation with an
        // empty list, which is the point: it gets named.
        for func in dispatcher.defined.difference(&dispatcher.reachable) {
            if !unreachable.contains_key(func.as_str()) {
                violations.push(SeamViolation {
                    check: "S8",
                    subject: format!("{func} is defined but no arm calls it"),
                    detail: format!(
                        "`{func}` is a dispatcher helper and no arm of \
                         `{DISPATCHER}` names it. If it is dead, delete it; if it \
                         is new and not wired up yet, this is where the wiring \
                         should have landed."
                    ),
                });
            }
        }

        if !dispatcher.has_wildcard {
            violations.push(SeamViolation {
                check: "S8",
                subject: format!("{DISPATCHER}'s match has no catch-all arm"),
                detail: "S8's reasoning is that the catch-all is what makes a \
                         missed tool silent. If it has been removed, a miss is a \
                         panic rather than an `unknown tool` error — which may be \
                         a deliberate choice, but it invalidates the premise the \
                         other S8 violations are argued from and needs saying."
                    .to_owned(),
            });
        }

        CheckOutcome {
            summary: super::CheckSummary {
                checked: dispatcher.arms.len(),
                allowlisted: 0,
            },
            violations,
        }
    }
}

/// Find and parse the dispatcher, skipping test paths.
fn parse_dispatcher(files: &[SourceFile]) -> Option<Dispatcher> {
    files
        .iter()
        .filter(|f| !is_test_path(&f.path))
        .find(|f| f.rel == DISPATCH_FILE)
        .map(|f| Dispatcher::parse(&f.syn, &f.rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Dispatcher {
        Dispatcher::parse(&syn::parse_str(src).expect("valid Rust"), "t.rs")
    }

    #[test]
    fn a_plain_dispatch_table_parses() {
        let d = parse(
            "pub async fn dispatch(r: R, t: &str) { match t { \
             \"job.create\" => dispatch_job_create(&r).await, \
             unknown => Err(unknown.to_string()), } } \
             async fn dispatch_job_create(r: &R) { \
             let _ = RuntimeCommand::CreateJob { a: 1 }; }",
        );
        assert_eq!(d.arms.len(), 1);
        assert_eq!(d.arms[0].tool, "job.create");
        assert_eq!(d.arms[0].target, "dispatch_job_create");
        assert!(d.has_wildcard);
        assert!(d.reachable.contains("dispatch_job_create"));
        assert_eq!(
            d.constructions,
            vec![("CreateJob".to_owned(), "dispatch_job_create".to_owned())]
        );
    }

    #[test]
    fn a_construction_in_an_unarmed_helper_is_still_found() {
        let d = parse(
            "pub async fn dispatch(r: R, t: &str) { match t { \
             \"job.create\" => dispatch_job_create(&r).await, \
             unknown => Err(unknown.to_string()), } } \
             async fn dispatch_job_create(r: &R) { \
             let _ = RuntimeCommand::CreateJob { a: 1 }; } \
             async fn dispatch_orphan(r: &R) { \
             let _ = RuntimeCommand::Reconcile { b: 2 }; }",
        );
        assert!(!d.reachable.contains("dispatch_orphan"));
        assert!(d.defined.contains("dispatch_orphan"));
        assert!(
            d.constructions
                .contains(&("Reconcile".to_owned(), "dispatch_orphan".to_owned())),
            "an orphan construction must still be recorded; skipping it is how \
             this check would pass on the defect it exists to catch"
        );
    }

    #[test]
    fn a_match_arm_matching_a_command_is_not_an_arm() {
        // The S1 lesson, applied here: a bare enum path is a pattern, not a
        // tool name, and treating it as one would invent tools.
        let d = parse(
            "pub async fn dispatch(c: RuntimeCommand) { match c { \
             RuntimeCommand::CreateJob { .. } => (), \
             unknown => (), } }",
        );
        assert!(d.arms.is_empty());
    }

    #[test]
    fn a_match_without_a_catch_all_is_reported() {
        let d = parse(
            "pub async fn dispatch(t: &str) { match t { \"a\" => dispatch_a(), } } \
             async fn dispatch_a() {}",
        );
        assert!(!d.has_wildcard);
    }
}
