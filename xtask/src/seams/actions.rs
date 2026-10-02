//! **S2 — every allowed action names a tool that is actually registered.**
//!
//! `tool_for_action` is a total `match` over `AllowedAction`, so a *missing*
//! arm is a compile error and the project gets that half for free. What it
//! cannot catch is a wrong string: the arms return `&'static str` literals,
//! and `"check.run"` versus `"checks.run"` compiles identically and fails only
//! when an agent follows `next_actions` and gets nothing back.
//!
//! So the two halves come from different places on purpose:
//!
//! - the **mapping** is read from source, because that is where the literals
//!   are, and a value-based check would have to construct an `AllowedAction`
//!   per variant to read back a constant;
//! - the **registered set** comes from `generate_all_schemas()`, which is what
//!   the daemon actually serves. Asking the live API rather than a second
//!   source of truth is the difference between this check and the drift it is
//!   meant to catch.
//!
//! The resulting asymmetry is deliberate: a tool *implemented but not
//! registered* is caught here; a tool *registered but not implemented* is not,
//! because that failure mode is a dispatch arm — already `verify-architecture`
//! and `verify-mcp-schemas` territory. Conflating them would make this check
//! fail for reasons its author did not intend, and a check that fails for
//! reasons its author did not intend gets disabled.

use std::collections::BTreeSet;

use syn::spanned::Spanned;

use super::{CheckOutcome, CheckSummary, SeamViolation, SourceFile, is_test_path};

/// The function that maps an action to a tool name.
const MAPPER: &str = "tool_for_action";

/// One `AllowedAction` → tool-name mapping, as written in the source.
///
/// Named for what it is rather than for what it will become: every arm is
/// collected here, and whether the daemon can honour it is decided later
/// against the live registry. A name that said "unregistered" would mean the
/// opposite of what the type holds.
pub struct Mapping {
    pub action: String,
    pub tool: String,
    pub loc: String,
}

pub struct ActionsCheck;

impl ActionsCheck {
    /// The literal each arm of `tool_for_action` returns, read from source.
    ///
    /// Split from [`ActionsCheck::run`] so the extraction is testable without a
    /// workspace on disk — the workspace-walking is plumbing, and the parsing
    /// is the part with edge cases.
    pub fn mappings_in(files: &[SourceFile]) -> Vec<Mapping> {
        let mut found = Vec::new();
        for file in files {
            if is_test_path(&file.path) {
                continue;
            }
            for item in &file.syn.items {
                let syn::Item::Fn(f) = item else { continue };
                if f.sig.ident != MAPPER {
                    continue;
                }
                let Some(syn::Expr::Match(m)) = super::tail_expr(&f.block) else {
                    continue;
                };
                for arm in &m.arms {
                    // Three arm shapes name an action, and they are three
                    // different `syn` types: a unit variant is `Pat::Path`,
                    // a braced variant is `Pat::Struct`, a tuple variant is
                    // `Pat::TupleStruct`. Matching only one of them is how a
                    // check ends up reporting "0 checked" and reading as a
                    // pass.
                    let path = match &arm.pat {
                        syn::Pat::Path(p) => &p.path,
                        syn::Pat::Struct(p) => &p.path,
                        syn::Pat::TupleStruct(p) => &p.path,
                        _ => continue,
                    };
                    // `_ => "fallback"` names no action, so there is nothing
                    // to check.
                    if path.is_ident("_") {
                        continue;
                    }
                    let Some(action) = path.segments.last() else {
                        continue;
                    };
                    let loc = format!("{}:{}", file.rel, arm.pat.span().start().line);
                    // An arm that is not a bare string literal is reported as
                    // such rather than skipped. The dangerous failure here is
                    // silence: an arm that stops being a literal must not
                    // quietly leave the checked set.
                    found.push(Mapping {
                        action: action.ident.to_string(),
                        tool: literal_str(&arm.body)
                            .unwrap_or_else(|| "<not a string literal>".to_owned()),
                        loc,
                    });
                }
            }
        }
        found
    }
}

impl ActionsCheck {
    /// `files` is the walk every other check already made. S2 does not
    /// re-walk the tree: parsing 232 files twice to answer four questions is
    /// a cost paid on every §97 run, and the sources are the same files.
    pub fn run(files: &[SourceFile]) -> CheckOutcome {
        // Asked of the live MCP API rather than a second source of truth,
        // which is the difference between this check and the drift it exists
        // to catch.
        let registered: BTreeSet<String> = ironmaint_mcp::schema::generate_all_schemas()
            .keys()
            .map(|k| k.0.clone())
            .collect();
        let mappings = ActionsCheck::mappings_in(files);

        let mut violations = Vec::new();
        if registered.is_empty() {
            violations.push(SeamViolation {
                check: "S2",
                subject: "registered tool set".to_owned(),
                detail: "the daemon registers no tools at all, so no action->tool \
                         mapping is verifiable. That is a failure of the thing S2 \
                         measures, not of the measurement."
                    .to_owned(),
            });
        }

        for m in &mappings {
            if !registered.contains(&m.tool) {
                violations.push(SeamViolation {
                    check: "S2",
                    subject: format!("AllowedAction::{} -> \"{}\"", m.action, m.tool),
                    detail: format!(
                        "the daemon registers no tool named `{}` ({}). next_actions \
                         advertises this action, so an agent that follows it calls a \
                         tool that does not exist.",
                        m.tool, m.loc
                    ),
                });
            }
        }

        CheckOutcome {
            summary: CheckSummary {
                checked: mappings.len(),
                allowlisted: 0,
            },
            violations,
        }
    }
}

/// The value of an expression that is a bare string literal.
fn literal_str(expr: &syn::Expr) -> Option<String> {
    match strip_groups(expr) {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) => Some(s.value()),
        _ => None,
    }
}

/// Unwrap `( … )` and `{ … }` wrappers, which `rustfmt` adds freely and which
/// would otherwise hide the literal from the matcher.
fn strip_groups(mut expr: &syn::Expr) -> &syn::Expr {
    loop {
        expr = match expr {
            syn::Expr::Paren(p) => &p.expr,
            syn::Expr::Group(g) => &g.expr,
            _ => return expr,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(src: &str) -> Vec<SourceFile> {
        // Stripped the way `collect_source_files` strips, so these tests
        // exercise the pipeline the check actually runs and not a parse that
        // is one stage short of it.
        let mut syn = syn::parse_file(src).expect("parse");
        super::super::strip_cfg_test(&mut syn);
        vec![SourceFile {
            rel: "crates/ironmaint-mcp/src/schema.rs".to_owned(),
            path: std::path::PathBuf::from("schema.rs"),
            syn,
            is_test: false,
        }]
    }

    fn tools(m: &[Mapping]) -> Vec<(&str, &str)> {
        m.iter()
            .map(|u| (u.action.as_str(), u.tool.as_str()))
            .collect()
    }

    #[test]
    fn a_literal_per_arm_is_read_out() {
        let f = files(
            "pub fn tool_for_action(a: &A) -> &'static str { match a {\
                 A::One => \"one.tool\",\
                 A::Two => \"two.tool\",\
             } }",
        );
        assert_eq!(
            tools(&ActionsCheck::mappings_in(&f)),
            [("One", "one.tool"), ("Two", "two.tool")]
        );
    }

    #[test]
    fn a_parenthesised_literal_is_still_a_literal() {
        // `rustfmt` and an over-eager edit both introduce these, and a check
        // that stopped seeing the literal would go quiet rather than red.
        let f = files(
            "pub fn tool_for_action(a: &A) -> &'static str { match a {\
                 A::One => (\"one.tool\"),\
             } }",
        );
        assert_eq!(tools(&ActionsCheck::mappings_in(&f))[0].1, "one.tool");
    }

    #[test]
    fn a_non_literal_arm_is_reported_rather_than_skipped() {
        let f = files(
            "const N: &str = \"x\";\
             pub fn tool_for_action(a: &A) -> &'static str { match a {\
                 A::One => N,\
             } }",
        );
        assert_eq!(
            tools(&ActionsCheck::mappings_in(&f))[0].1,
            "<not a string literal>"
        );
    }

    #[test]
    fn a_wildcard_arm_is_not_a_mapping() {
        // `_ => "fallback"` names no action, so there is nothing to check.
        let f = files(
            "pub fn tool_for_action(a: &A) -> &'static str { match a {\
                 _ => \"fallback\",\
             } }",
        );
        assert!(ActionsCheck::mappings_in(&f).is_empty());
    }

    #[test]
    fn a_test_copy_of_the_mapper_is_not_the_mapper() {
        // The mapper is found by function name, not by directory, so a test
        // that grows a helper of the same name must not be able to inject a
        // mapping into the result. The copy here returns a tool the daemon
        // does not register, so reading it would raise a violation against
        // code that is never compiled into a binary.
        let f = files(
            "pub fn tool_for_action(a: &A) -> &'static str { match a {\
                 A::One => \"real.tool\",\
             } }\
             #[cfg(test)] mod tests {\
                 fn tool_for_action(a: &A) -> &'static str { match a {\
                     A::One => \"copy.tool\",\
                 } } }",
        );
        let m = ActionsCheck::mappings_in(&f);
        assert_eq!(tools(&m), [("One", "real.tool")]);
    }
}
