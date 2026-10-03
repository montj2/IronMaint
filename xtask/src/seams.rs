//! `cargo xtask verify-seams` — the static seam checks.
//!
//! A *seam* is a boundary where one layer declares an interface — a type, a
//! function name, a command variant, a tool name, a method set — and a
//! separate layer is supposed to fulfil it, and where the two can drift so
//! that a thing is *modelled* and nothing *produces* it. That is the defect
//! class 0B.10 turned up fourteen times, and it is invisible to the checks the
//! project already runs: a fully-covered line of code is fully covered whether
//! or not anything can ever call it.
//!
//! Four static checks live here, one per spec item in
//! `doc/SEAM-VERIFICATION.md` §2:
//!
//! | Check | Assertion |
//! |---|---|
//! | **S1** | every `RuntimeCommand` variant is *constructed* outside tests |
//! | **S2** | every tool name `tool_for_action` returns is registered |
//! | **S4** | every `JobEvent` variant is handled by every replay implementation, and seed selection is decided in exactly one place |
//! | **S5** | both store backends implement the same `IronMaintStore` method set |
//!
//! S3, S8 and S9 are behavioural and live in `ironmaint-testkit`; S6 is
//! deferred; S7 already exists at
//! `crates/ironmaint-testkit/tests/replay_roundtrip.rs` and is inherited here
//! rather than reimplemented.
//!
//! ## Two rules this file is built around
//!
//! **Construction is not mention.** A variant can appear in a match arm, a doc
//! comment, a schema snapshot, and a test, and be callable by nobody. S1
//! therefore counts *expression* paths only. The `=>` that separates a pattern
//! from a body is the whole difference, and it is exactly what a grep for
//! `VariantName {` gets wrong: every arm of `handle_command` looks like a
//! construction to a text search, and there are twelve of them.
//!
//! **An allowlist is a claim, and a blank claim is not one.** Each entry names
//! a spec section or a design fact. The report prints the count, so an
//! allowlist that grows is visible in the §97 output rather than only in a
//! diff. D-14's lesson is that a check which has never rejected anything is an
//! assumption wearing a green tick, and a check whose failures are all
//! suppressed is the same thing with more steps.
//!
//! ## A fourth thing, which is not a rule about seams
//!
//! The first run of this file reported six violations, and **five of the six
//! were bugs in the check** — it flagged constructions I had verified by hand
//! at `dispatch.rs:138, 229, 304, 352, 486`. The cause was one line: the
//! qualification test compared the enum's tail segment against the
//! *construction path's* tail segment, which is the variant, so every
//! construction failed to qualify.
//!
//! The test that would have caught it — `a_fully_qualified_path_counts_as_construction`
//! — had never been executed. The module compiled, `cargo run -p xtask --
//! verify-seams` produced output, and the check reported violations with a
//! confident format. Nobody had run `cargo test -p xtask`, and it turns out
//! twelve of its tests failed.
//!
//! So: **run the new check's own tests before believing its first output.** A
//! check that has never been run against inputs whose answers are known is
//! indistinguishable from a check that works, right up until the moment it
//! matters. `cargo test` also does not run clippy lints, so the §97 gate has to
//! be walked in full the first time rather than assumed from the parts of it
//! that were already green.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use syn::spanned::Spanned;

use crate::seams::{
    actions::ActionsCheck,
    commands::{CheckOutcome, CommandsCheck},
    events::EventsCheck,
    reachability::ReachabilityCheck,
    stores::StoresCheck,
};

mod actions;
mod commands;
mod events;
mod reachability;
mod stores;

/// Directories under the workspace root whose `.rs` files the source-walking
/// checks read. Mirrors `architecture.rs`'s discovery set so a new crate lands
/// without touching this file.
const SOURCE_DIRS: &[&str] = &["crates", "adapters", "bins"];

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// One failed assertion, attributed to the check that made it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SeamViolation {
    pub check: &'static str,
    pub subject: String,
    pub detail: String,
}

impl fmt::Display for SeamViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} — {}", self.check, self.subject, self.detail)
    }
}

/// The result of running every static check, plus the context a reader needs to
/// judge the run: what was inspected, and what was allowed.
#[derive(Debug, Default)]
pub struct Report {
    pub violations: Vec<SeamViolation>,
    /// Check name → (subjects inspected, subjects excused by the allowlist).
    pub summary: BTreeMap<&'static str, CheckSummary>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CheckSummary {
    pub checked: usize,
    pub allowlisted: usize,
}

impl Report {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Seam verify report")?;
        writeln!(f, "===================")?;
        for (name, s) in &self.summary {
            writeln!(
                f,
                "  {name:<14} {} checked, {} allowlisted",
                s.checked, s.allowlisted
            )?;
        }
        if self.violations.is_empty() {
            writeln!(f, "\nNo violations.")?;
        } else {
            writeln!(f, "\nViolations ({}):", self.violations.len())?;
            for v in &self.violations {
                writeln!(f, "  - {v}")?;
            }
        }
        Ok(())
    }
}

pub fn run() -> Result<Report, Box<dyn Error>> {
    let root = workspace_root()?;
    let files = collect_source_files(&root)?;
    let mut report = Report::default();

    // One walk, five checks. S2 and S8 read the same sources as the rest; only
    // part of each comes from the tree, and that part is the part that needs
    // reading.
    let outcomes = [
        ("S1", CommandsCheck::run(&files)),
        ("S2", ActionsCheck::run(&files)),
        ("S4", EventsCheck::run(&files)),
        ("S5", StoresCheck::run(&files)),
        ("S8", ReachabilityCheck::run(&files)),
    ];
    for (name, outcome) in outcomes {
        report.summary.insert(name, outcome.summary);
        report.violations.extend(outcome.violations);
    }
    // Sorted so the failure list does not depend on which check ran first.
    report.violations.sort();
    Ok(report)
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    // xtask lives at <workspace>/xtask, so CARGO_MANIFEST_DIR's parent is the
    // root. Derived from the manifest rather than from `current_exe` for the
    // same reason `migrations.rs` does it: `CARGO_TARGET_DIR` may point
    // outside the workspace entirely (the container sets it to a named
    // volume), and a `current_exe` walk then leaves the workspace.
    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").map_err(|e| format!("CARGO_MANIFEST_DIR: {e}"))?,
    );
    let root = manifest
        .parent()
        .ok_or("verify-seams: CARGO_MANIFEST_DIR has no parent")?
        .to_path_buf();
    if !root.join("Cargo.toml").is_file() {
        return Err(format!(
            "verify-seams: {} is not a workspace root (no Cargo.toml)",
            root.display()
        )
        .into());
    }
    Ok(root)
}

// ---------------------------------------------------------------------------
// Source collection
// ---------------------------------------------------------------------------

/// A parsed Rust source file: its path relative to the workspace root, its
/// `syn` tree, and whether it is a test the production-crawl should ignore.
#[derive(Debug)]
pub struct SourceFile {
    pub rel: String,
    pub path: PathBuf,
    pub syn: syn::File,
    pub is_test: bool,
}

impl SourceFile {
    /// A one-line locator for a report, so a failure names the place rather
    /// than the file.
    #[must_use]
    pub fn locate(&self, node: &dyn Spanned) -> String {
        self.locate_span(node.span())
    }

    /// `file:line` for a span, for the nodes `Spanned` cannot reach — an
    /// `Ident` inside an `ItemFn` and an `ImplItemFn` is the same function
    /// under two node types, and the check wants to name where it is.
    pub fn locate_span(&self, span: proc_macro2::Span) -> String {
        format!("{}:{}", self.rel, span.start().line)
    }
}

/// Whether a path is test code, by the two conventions the workspace uses:
/// a `tests/` directory, or a `#[cfg(test)]` module. Both are excluded from the
/// "does production construct this?" question, because a test constructing a
/// command is the defect this check exists to catch, not evidence against it.
#[must_use]
pub fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_string_lossy() == "tests")
}

/// Every `.rs` file under the source directories, parsed, with test modules
/// stripped.
///
/// Walked in sorted order so both the report and any failure list are
/// deterministic — a verifier whose output order depends on directory iteration
/// is one whose failures get skimmed.
pub fn collect_source_files(root: &Path) -> Result<Vec<SourceFile>, Box<dyn Error>> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for dir in SOURCE_DIRS {
        let dir_path = root.join(dir);
        if !dir_path.is_dir() {
            continue;
        }
        walk_rs(&dir_path, &mut paths)?;
    }
    paths.sort();

    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut syn =
            syn::parse_file(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // Strip *before* deciding `is_test`, so that a file's production
        // items are the only ones any check can see. Leaving a `#[cfg(test)]
        // mod tests` in the tree is the single most likely way for one of
        // these checks to produce a confident false pass: a helper of the
        // right name inside the test module is a *copy*, and a check that
        // reads the copy is checking nothing.
        let is_test = is_test_path(&path);
        strip_cfg_test(&mut syn);
        out.push(SourceFile {
            rel,
            path,
            syn,
            is_test,
        });
    }
    Ok(out)
}

/// Remove every `#[cfg(test)]` item from the tree, at any depth.
pub fn strip_cfg_test(file: &mut syn::File) {
    file.items.retain(|item| !item_is_cfg_test(item));
    let mut stripper = CfgTestStripper;
    syn::visit_mut::visit_file_mut(&mut stripper, file);
}

fn item_is_cfg_test(item: &syn::Item) -> bool {
    match item {
        syn::Item::Mod(m) => m.attrs.iter().any(is_cfg_test),
        syn::Item::Fn(f) => f.attrs.iter().any(is_cfg_test),
        syn::Item::Impl(i) => i.attrs.iter().any(is_cfg_test),
        syn::Item::Struct(s) => s.attrs.iter().any(is_cfg_test),
        syn::Item::Enum(e) => e.attrs.iter().any(is_cfg_test),
        syn::Item::Const(c) => c.attrs.iter().any(is_cfg_test),
        syn::Item::Static(s) => s.attrs.iter().any(is_cfg_test),
        syn::Item::Macro(m) => m.attrs.iter().any(is_cfg_test),
        _ => false,
    }
}

struct CfgTestStripper;

impl syn::visit_mut::VisitMut for CfgTestStripper {
    fn visit_item_mut(&mut self, node: &mut syn::Item) {
        if item_is_cfg_test(node) {
            *node = syn::Item::Verbatim(syn::parse_quote! {});
            return;
        }
        syn::visit_mut::visit_item_mut(self, node);
    }
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk_rs(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// A block's trailing expression, if it has one. `match` is an expression, so
/// a function whose value is a `match` ends in one.
#[must_use]
pub fn tail_expr(block: &syn::Block) -> Option<&syn::Expr> {
    match block.stmts.last()? {
        syn::Stmt::Expr(e, None) => Some(e),
        _ => None,
    }
}

#[must_use]
pub fn is_cfg_test(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("cfg") {
        return false;
    }
    // `#[cfg(test)]` and `#[cfg(all(test, unix))]` both mean "test", and
    // matching the attribute path alone would miss the second. The `cfg`
    // meta is a path or a list of conditions in every form that occurs here;
    // both stringify through `Display`, so this needs no `quote` dependency
    // and stays correct for a form this code has not seen.
    match &attr.meta {
        syn::Meta::Path(p) => p.is_ident("test"),
        syn::Meta::List(l) => l.tokens.to_string().replace(' ', "").contains("test"),
        syn::Meta::NameValue(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Shared AST helpers
// ---------------------------------------------------------------------------

/// The last path segment of a `Path`.
///
/// A `Path` that reaches these helpers came from a trait position in an
/// `impl`, or from a type position — neither can be empty, so this is an
/// `expect` rather than an `Option`.
#[must_use]
pub fn path_tail(path: &syn::Path) -> &syn::Ident {
    &path
        .segments
        .last()
        .expect("a trait or type path is never empty")
        .ident
}

/// The name of a type in an `impl` header or a `Type` position.
///
/// `self_ty` is a `Box<Type>`, and the shapes that reach these helpers are a
/// bare path (`MockStore`) or a reference to one. Anything more elaborate is
/// not one of the types this file is looking for and yields `None`, so a
/// caller compares rather than assumes.
#[must_use]
pub fn self_type_name(ty: &syn::Type) -> Option<&syn::Ident> {
    match ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| &s.ident),
        syn::Type::Reference(r) => self_type_name(&r.elem),
        syn::Type::Group(g) => self_type_name(&g.elem),
        syn::Type::Paren(p) => self_type_name(&p.elem),
        _ => None,
    }
}

/// Visit every `EnumVariant` declared by a named enum in one file, yielding the
/// variant name.
pub fn enum_variants<'a>(file: &'a syn::File, enum_name: &str) -> Vec<&'a syn::Ident> {
    let mut out = Vec::new();
    for item in &file.items {
        let syn::Item::Enum(e) = item else {
            continue;
        };
        if e.ident != enum_name {
            continue;
        }
        for v in &e.variants {
            out.push(&v.ident);
        }
    }
    out
}

/// Locate the single file that declares `enum <name>`, by scanning for the
/// declaration rather than hardcoding a path — the check should survive the
/// enum moving.
pub fn find_declaring_file<'a>(files: &'a [SourceFile], enum_name: &str) -> Option<&'a SourceFile> {
    files
        .iter()
        .find(|f| !enum_variants(&f.syn, enum_name).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).expect("parse")
    }

    #[test]
    fn a_path_with_multiple_components_takes_its_tail() {
        let f = parse("fn f() { let _ = RuntimeCommand::CreateJob { x: 1 }; }");
        let names = CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob");
        assert_eq!(names, 1, "the fully-qualified path must still be found");
    }

    #[test]
    fn a_bare_variant_name_is_found_too() {
        // `use RuntimeCommand::CreateJob;` then `CreateJob { .. }` is legal
        // and must count, or the check would pass on a codebase that stopped
        // qualifying its paths.
        let f = parse("fn f() { let _ = CreateJob { x: 1 }; }");
        let names = CommandsCheck::constructions_in(&f, "", "CreateJob");
        assert_eq!(names, 1);
    }

    #[test]
    fn enum_variants_are_listed_in_declaration_order() {
        let f = parse("enum E { Alpha, Beta, Gamma }");
        let names: Vec<String> = enum_variants(&f, "E")
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(names, ["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn a_missing_enum_yields_nothing_rather_than_erroring() {
        let f = parse("struct NotAnEnum;");
        assert!(enum_variants(&f, "E").is_empty());
        assert!(find_declaring_file(&[], "E").is_none());
    }

    #[test]
    fn a_cfg_test_module_is_stripped_before_any_check_sees_it() {
        // The false pass this prevents: a test module containing a helper
        // named like the thing under test. S1 would count the construction,
        // the seam would read as closed, and the check would be checking its
        // own test.
        let mut f = parse(
            "pub fn production() { let _ = RuntimeCommand::CreateJob { a: 1 }; }\
             #[cfg(test)]\
             mod tests {\
                 pub fn production() { let _ = RuntimeCommand::CreateJob { a: 2 }; }\
             }",
        );
        strip_cfg_test(&mut f);
        let n = CommandsCheck::constructions_in(&f, "RuntimeCommand", "CreateJob");
        assert_eq!(n, 1, "only the production construction may remain");
    }

    #[test]
    fn cfg_test_stripping_reaches_inside_a_nested_module() {
        // Not only the top level: `mod outer { #[cfg(test)] mod t { … } }`
        // has to lose it too, and a depth-1 stripper would leave the copy in
        // place while reporting success.
        let mut f = parse(
            "mod outer { pub fn helper() {} #[cfg(test)] mod t { \
                 pub fn helper() {} } }",
        );
        strip_cfg_test(&mut f);
        assert_eq!(fn_defs(&f, "helper"), 1);
    }

    #[test]
    fn cfg_all_test_is_recognised() {
        // `#[cfg(all(test, unix))]` is the same intent. Matching the
        // attribute path alone would miss it and leave a test copy in the
        // tree.
        let mut f =
            parse("pub fn helper() {} #[cfg(all(test, unix))] mod t { pub fn helper() {} }");
        strip_cfg_test(&mut f);
        assert_eq!(fn_defs(&f, "helper"), 1);
    }

    #[test]
    fn an_unrelated_cfg_attribute_is_not_stripped() {
        // `#[cfg(feature = "x")]` compiles into production builds. Stripping
        // it would hide real code from the check, which fails in the other
        // direction and is just as wrong.
        let mut f = parse("#[cfg(feature = \"serde\")] pub fn helper() {}");
        strip_cfg_test(&mut f);
        assert_eq!(fn_defs(&f, "helper"), 1);
    }

    /// How many free functions named `name` survive in the tree.
    ///
    /// The strip tests measure *definitions*, so they count definitions. An
    /// earlier version of these tests counted them through
    /// `constructions_in`, which visits `ExprStruct` and therefore cannot see
    /// a function item at all — the three tests passed nothing and failed
    /// everything, which is how they were caught.
    fn fn_defs(file: &syn::File, name: &str) -> usize {
        use syn::visit::Visit;
        struct V<'a> {
            name: &'a str,
            n: usize,
        }
        impl<'ast> syn::visit::Visit<'ast> for V<'_> {
            fn visit_item_fn(&mut self, node: &syn::ItemFn) {
                if node.sig.ident == self.name {
                    self.n += 1;
                }
                syn::visit::visit_item_fn(self, node);
            }
        }
        let mut v = V { name, n: 0 };
        v.visit_file(file);
        v.n
    }
}
