//! **S6 — production adapters do not advertise Phase 2 capabilities.**
//!
//! ## What the compiler already gives us, and why this check is not that
//!
//! The `DistributionAdapter` trait's accessor methods (`policy()`,
//! `build()`, `issues()`, `release()`, `inspection()`) return
//! `Option<&dyn ...Capability>`. The shared conformance suite in
//! `crates/ironmaint-testkit/src/conformance.rs:97-127`
//! (`assert_descriptor_is_valid`) walks every advertised
//! `AdapterCapability` and asserts the corresponding method returns
//! `Some`.
//!
//! What the compiler does not give us is the **inverse**: a production
//! adapter could advertise a Phase 2 capability, return `None` from
//! the corresponding accessor, and the conformance suite would
//! reject it — but at the *unit-test* layer, not at the seam. The
//! seam check is what makes the property structural: a future PR
//! that adds a `BuildPlanning` to the production adapter's
//! descriptor without also adding a real `BuildCapability` impl
//! fails this check before it ever reaches the conformance suite.
//!
//! ## What this check does
//!
//! For every `adapters/*/src/descriptor.rs` under the workspace:
//!
//!  1. Collect every `AdapterCapability::Foo` token mentioned in
//!     the file (both `caps.insert(AdapterCapability::Foo)` and
//!     `for c in [..., AdapterCapability::Foo, ...]` patterns).
//!  2. Locate the `implementation_name` string literal in the
//!     `AdapterDescriptor` literal expression.
//!  3. If `implementation_name` does NOT end in `"-stub"`, walk
//!     the advertised set and report a violation for each
//!     `BuildPlanning | PackageQaPlanning | FunctionalTestPlanning
//!     | UpgradeTestPlanning | ReproducibilityPlanning | IssueWrite
//!     | ReleaseMetadata | PublicationPlanning` token.
//!
//! Stubs are allowlisted: their `implementation_name` ends in
//! `"-stub"` by convention (`adapters/debian-stub`'s
//! `implementation_name = "debian-stub"`, ditto for Fedora). A
//! future adapter whose name accidentally ends in `"-stub"` will
//! be allowlisted by this check — but a future adapter named
//! `something-with-stub-in-the-middle` will not, because the
//! match is `ends_with` not `contains`. The trade-off is
//! deliberate: convention over correctness-of-strings.
//!
//! ## What this check is NOT
//!
//! - **Not** a runtime check. The check reads the AST; the
//!   conformance suite reads the live `AdapterDescriptor`. Both
//!   reads come from the same `build_descriptor()` function, so
//!   a divergence is a bug in the check, not in the adapter.
//! - **Not** an `UpstreamDiscovery` check. `UpstreamDiscovery`
//!   is a Phase 2 capability by section number, but it is NOT
//!   in §11's "Do not advertise yet" list for Debian. The
//!   Fedora stub already advertises it. S6 does not regress
//!   that.
//! - **Not** a `SourceInspection` check. The production
//!   Debian adapter advertises `SourceInspection` in 1B.3;
//!   S6 must not flag that.

use std::collections::BTreeSet;
use std::path::Path;

use super::{CheckOutcome, CheckSummary, SeamViolation, SourceFile};

/// The Phase 2 capabilities a production (non-stub) adapter
/// must NOT advertise. Names match the `AdapterCapability`
/// enum-variant spellings exactly.
const PHASE_2_CAPABILITIES: &[&str] = &[
    "BuildPlanning",
    "PackageQaPlanning",
    "FunctionalTestPlanning",
    "UpgradeTestPlanning",
    "ReproducibilityPlanning",
    "IssueWrite",
    "ReleaseMetadata",
    "PublicationPlanning",
];

/// Substring convention: a `DistributionAdapter` whose
/// `implementation_name` ends in `"-stub"` is a test fixture
/// and is allowlisted. Stubs simulate the full Phase 2 surface
/// for the conformance suite.
const STUB_NAME_SUFFIX: &str = "-stub";

/// The descriptor files the check walks. The shape is fixed by
/// the workspace layout (`adapters/*/src/descriptor.rs`).
fn descriptor_files(files: &[SourceFile]) -> impl Iterator<Item = &SourceFile> {
    files.iter().filter(|f| is_descriptor_file(&f.rel))
}

/// `path::Path::ends_with` is the wrong tool here (it works on
/// whole segments, not arbitrary suffixes). Compare the
/// `OsStr`s directly.
fn is_descriptor_file(rel: &str) -> bool {
    let p = Path::new(rel);
    p.starts_with("adapters/") && p.ends_with("src/descriptor.rs")
}

/// S6: the seam check.
pub struct AdaptersCheck;

impl AdaptersCheck {
    pub fn run(files: &[SourceFile]) -> CheckOutcome {
        let mut violations = Vec::new();
        let mut checked = 0usize;
        let mut allowlisted = 0usize;

        for file in descriptor_files(files) {
            let advertised = advertised_capabilities(&file.syn);
            let implementation_name = match implementation_name(&file.syn) {
                Some(name) => name,
                None => {
                    violations.push(SeamViolation {
                        check: "S6",
                        subject: file.rel.clone(),
                        detail: "could not locate `implementation_name: \"...\"` in the \
                                 AdapterDescriptor literal; the check cannot run against this file"
                            .to_owned(),
                    });
                    continue;
                }
            };
            let is_stub = implementation_name.ends_with(STUB_NAME_SUFFIX);

            for cap in PHASE_2_CAPABILITIES {
                checked += 1;
                if advertised.contains(*cap) {
                    if is_stub {
                        allowlisted += 1;
                    } else {
                        violations.push(SeamViolation {
                            check: "S6",
                            subject: format!("{implementation_name} advertises {cap}"),
                            detail: format!(
                                "`{implementation_name}` (`{rel}`) advertises \
                                 `AdapterCapability::{cap}`, a Phase 2 capability that \
                                 §11's \"Do not advertise yet\" list forbids for \
                                 production adapters. Either drop the capability from \
                                 the descriptor or move `{implementation_name}` to a \
                                 `-stub`-suffixed test fixture.",
                                implementation_name = implementation_name,
                                rel = file.rel,
                                cap = cap,
                            ),
                        });
                    }
                }
            }
        }

        CheckOutcome {
            summary: CheckSummary {
                checked,
                allowlisted,
            },
            violations,
        }
    }
}

/// Every `AdapterCapability::Foo` token mentioned in the file.
/// Both `caps.insert(AdapterCapability::Foo)` and `for c in
/// [..., AdapterCapability::Foo, ...]` patterns surface here.
pub fn advertised_capabilities(file: &syn::File) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for item in &file.items {
        collect_from_item(item, &mut out);
    }
    out
}

fn collect_from_item(item: &syn::Item, out: &mut BTreeSet<String>) {
    match item {
        syn::Item::Fn(f) => {
            for stmt in &f.block.stmts {
                collect_from_stmt(stmt, out);
            }
        }
        syn::Item::Const(c) => {
            collect_from_expr(&c.expr, out);
        }
        syn::Item::Static(s) => {
            collect_from_expr(&s.expr, out);
        }
        syn::Item::Impl(i) => {
            for item in &i.items {
                if let syn::ImplItem::Fn(method) = item {
                    for stmt in &method.block.stmts {
                        collect_from_stmt(stmt, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_from_stmt(stmt: &syn::Stmt, out: &mut BTreeSet<String>) {
    match stmt {
        syn::Stmt::Local(local) => {
            if let Some(init) = &local.init {
                collect_from_expr(&init.expr, out);
            }
        }
        syn::Stmt::Expr(expr, _) => {
            collect_from_expr(expr, out);
        }
        syn::Stmt::Item(item) => {
            collect_from_item(item, out);
        }
        // `Stmt::Macro` is `let Some(mac) = ...;` style; the
        // existing descriptor files don't use this. If a future
        // descriptor does, the macro call's tokens are visible
        // via `mac.tokens` but require token-tree parsing, which
        // is out of scope for S6.
        _ => {}
    }
}

fn collect_from_expr(expr: &syn::Expr, out: &mut BTreeSet<String>) {
    match expr {
        syn::Expr::Call(c) => {
            for arg in &c.args {
                collect_from_expr(arg, out);
            }
            collect_from_expr(&c.func, out);
        }
        syn::Expr::Array(a) => {
            for el in &a.elems {
                collect_from_expr(el, out);
            }
        }
        syn::Expr::ForLoop(f) => {
            collect_from_expr(&f.expr, out);
        }
        syn::Expr::MethodCall(m) => {
            collect_from_expr(&m.receiver, out);
            for arg in &m.args {
                collect_from_expr(arg, out);
            }
        }
        syn::Expr::Path(p) => {
            collect_capability_path(&p.path, out);
        }
        syn::Expr::Struct(s) => {
            for field in &s.fields {
                collect_from_expr(&field.expr, out);
            }
        }
        _ => {}
    }
}

fn collect_capability_path(p: &syn::Path, out: &mut BTreeSet<String>) {
    if p.segments.len() == 2
        && p.segments[0].ident == "AdapterCapability"
        && let Some(last) = p.segments.last()
    {
        out.insert(last.ident.to_string());
    }
}

/// The string literal assigned to `implementation_name` in the
/// `AdapterDescriptor { ... }` struct expression. Returns
/// `None` if the file has no such expression.
pub fn implementation_name(file: &syn::File) -> Option<String> {
    for item in &file.items {
        if let Some(name) = implementation_name_in_item(item) {
            return Some(name);
        }
    }
    None
}

fn implementation_name_in_item(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Fn(f) => implementation_name_in_stmts(&f.block.stmts),
        syn::Item::Impl(i) => {
            for ii in &i.items {
                if let syn::ImplItem::Fn(m) = ii
                    && let Some(name) = implementation_name_in_stmts(&m.block.stmts)
                {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

fn implementation_name_in_stmts(stmts: &[syn::Stmt]) -> Option<String> {
    for stmt in stmts {
        if let syn::Stmt::Expr(syn::Expr::Struct(s), _) = stmt {
            for field in &s.fields {
                if let syn::Member::Named(ident) = &field.member
                    && ident == "implementation_name"
                {
                    if let syn::Expr::Lit(elit) = &field.expr
                        && let syn::Lit::Str(lit) = &elit.lit
                    {
                        return Some(lit.value());
                    }
                    // `implementation_name: "x".into()` is the
                    // common idiom. Walk the inner `MethodCall`'s
                    // receiver (the string literal) to get the
                    // value.
                    if let syn::Expr::MethodCall(m) = &field.expr
                        && let syn::Expr::Lit(elit) = &*m.receiver
                        && let syn::Lit::Str(lit) = &elit.lit
                    {
                        return Some(lit.value());
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_file;

    fn parse(src: &str) -> syn::File {
        parse_file(src).expect("valid Rust")
    }

    #[test]
    fn a_production_adapter_advertising_a_phase_2_capability_violates() {
        let src = r#"
            fn build_descriptor() -> AdapterDescriptor {
                let mut caps = AdapterCapabilities::new();
                caps.insert(AdapterCapability::SourceInspection);
                caps.insert(AdapterCapability::BuildPlanning);
                AdapterDescriptor {
                    family: DistributionFamily::new("debian").unwrap(),
                    implementation_name: "debian".into(),
                    capabilities: caps,
                }
            }
        "#;
        let f = parse(src);
        let outcome = AdaptersCheck::run(&[SourceFile {
            rel: "adapters/debian/src/descriptor.rs".into(),
            path: std::path::PathBuf::from("adapters/debian/src/descriptor.rs"),
            syn: f,
            is_test: false,
        }]);
        assert_eq!(outcome.violations.len(), 1);
        assert!(outcome.violations[0].detail.contains("BuildPlanning"));
        assert!(outcome.violations[0].detail.contains("debian"));
    }

    #[test]
    fn a_stub_adapter_advertising_a_phase_2_capability_is_allowlisted() {
        let src = r#"
            fn build_descriptor() -> AdapterDescriptor {
                let mut caps = AdapterCapabilities::new();
                caps.insert(AdapterCapability::BuildPlanning);
                AdapterDescriptor {
                    family: DistributionFamily::new("debian").unwrap(),
                    implementation_name: "debian-stub".into(),
                    capabilities: caps,
                }
            }
        "#;
        let f = parse(src);
        let outcome = AdaptersCheck::run(&[SourceFile {
            rel: "adapters/debian-stub/src/descriptor.rs".into(),
            path: std::path::PathBuf::from("adapters/debian-stub/src/descriptor.rs"),
            syn: f,
            is_test: false,
        }]);
        assert!(outcome.violations.is_empty());
        assert!(outcome.summary.allowlisted >= 1);
    }

    #[test]
    fn a_production_adapter_advertising_only_phase1_capabilities_passes() {
        let src = r#"
            fn build_descriptor() -> AdapterDescriptor {
                let mut caps = AdapterCapabilities::new();
                caps.insert(AdapterCapability::SourceInspection);
                caps.insert(AdapterCapability::VersionComparison);
                caps.insert(AdapterCapability::IssueRead);
                AdapterDescriptor {
                    family: DistributionFamily::new("debian").unwrap(),
                    implementation_name: "debian".into(),
                    capabilities: caps,
                }
            }
        "#;
        let f = parse(src);
        let outcome = AdaptersCheck::run(&[SourceFile {
            rel: "adapters/debian/src/descriptor.rs".into(),
            path: std::path::PathBuf::from("adapters/debian/src/descriptor.rs"),
            syn: f,
            is_test: false,
        }]);
        assert!(outcome.violations.is_empty());
    }

    #[test]
    fn stub_array_pattern_is_also_recognised() {
        // The existing debian-stub uses a `for c in [...]` pattern.
        // The walker must recognise `AdapterCapability::Foo` in
        // array literals as well as `caps.insert` arguments.
        let src = r#"
            fn build_descriptor() -> AdapterDescriptor {
                let mut caps = AdapterCapabilities::new();
                for c in [
                    AdapterCapability::BuildPlanning,
                    AdapterCapability::IssueRead,
                ] {
                    caps.insert(c);
                }
                AdapterDescriptor {
                    family: DistributionFamily::new("debian").unwrap(),
                    implementation_name: "debian-stub".into(),
                    capabilities: caps,
                }
            }
        "#;
        let f = parse(src);
        let advertised = advertised_capabilities(&f);
        assert!(advertised.contains("BuildPlanning"));
        assert!(advertised.contains("IssueRead"));
    }
}
