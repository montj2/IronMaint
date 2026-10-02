//! **S4 — a replay enumerates every `JobEvent`, and the seed decision is
//! decided in exactly one place.**
//!
//! ## Why the match arms are not the interesting half
//!
//! `ProjectionApply::apply` matches on `JobEvent`, so a new variant is a
//! compile error until it is handled. That half is free. It is not free if
//! someone adds a `_ => …` catch-all, which compiles today and silently
//! re-introduces exactly the defect this check exists to catch: a variant
//! that is *accepted* and *ignored*. So the check counts arms and rejects a
//! wildcard.
//!
//! ## What "a replay" means, and why it is not "any match in the tree"
//!
//! Two scoping decisions, both forced by code that is correct and that a
//! broader rule reports as a defect:
//!
//! - **`load_resume_record`** (`service.rs:966`) is a `find_map` over the
//!   event log with `_ => None`, scanning for one specific variant. Its
//!   wildcard is the correct code. A `match` on `JobEvent` is a *dispatcher*
//!   only when something dispatches an event, so the coverage rule applies to
//!   replay bodies and not to every `match` in the workspace.
//! - **`SqliteStore::rebuild_projection`** is a one-line forward into
//!   `ops::projections::rebuild`, and *that* is where the seed is chosen.
//!   Judging the method alone reports a second copy of a decision the code
//!   makes exactly once, which is the D-14 shape appearing as a false
//!   positive. So the seed rule is about the delegation *chain*: a body that
//!   is nothing but a call forwards its verdict to whatever it calls.
//!
//! The first version of this check made both mistakes and reported seven
//! violations against correct code. Scoping a check to "the code this rule is
//! actually about" is the work; finding the rule was the easy half.
//!
//! ## Why the seeds are the interesting half
//!
//! D-14 was the fix: two store backends each held their own copy of the
//! "which event seeds a projection" decision, and only one was ever exercised.
//! The rule this check encodes is therefore about *duplication*, not coverage:
//!
//! > the seed decision is made in exactly one place, and every replay
//! > implementation asks that place rather than re-deriving it.
//!
//! A replay that re-derives the decision is not wrong today. It is wrong the
//! first time the decision changes, and nothing will say so.

use std::collections::{BTreeMap, BTreeSet};

use syn::visit::Visit;

use super::{CheckOutcome, CheckSummary, SeamViolation, SourceFile, enum_variants};

/// The event enum every replay implementation must handle.
const ENUM: &str = "JobEvent";

/// The one function allowed to decide which event seeds a projection.
const SEED: &str = "seed_projection";

/// The trait method every replay implementation must route through it.
const REPLAY: &str = "rebuild_projection";

/// The trait whose `apply` is the one place a replay dispatches an event.
const APPLY_TRAIT: &str = "ProjectionApply";

pub struct EventsCheck;

impl EventsCheck {
    pub fn run(files: &[SourceFile]) -> CheckOutcome {
        let Some(decl) = super::find_declaring_file(files, ENUM) else {
            return CheckOutcome {
                summary: CheckSummary::default(),
                violations: vec![SeamViolation {
                    check: "S4",
                    subject: ENUM.to_owned(),
                    detail: "no file declares this enum, so the check cannot run".to_owned(),
                }],
            };
        };
        let variants: BTreeSet<String> = enum_variants(&decl.syn, ENUM)
            .iter()
            .map(ToString::to_string)
            .collect();

        let mut violations = Vec::new();
        let mut checked = 0usize;

        // The bodies that make up a replay, in two groups: the trait method
        // that dispatches an event, and the implementation bodies reached
        // from `rebuild_projection` by delegation.
        let bodies = ReplayBodies::collect(files);

        // 1. Every variant has an explicit arm wherever a replay dispatches
        //    `JobEvent`.
        for body in bodies.all() {
            for (owner, matched) in EventsCheck::matchers_in(&body.loc, body.block) {
                for v in &variants {
                    checked += 1;
                    if !matched.handled.contains(v) {
                        violations.push(SeamViolation {
                            check: "S4",
                            subject: format!("{ENUM}::{v} at {owner}"),
                            detail: "no explicit arm in a replay. A wildcard would \
                                      compile and would silently drop the event, which \
                                      is the D-14 defect."
                                .to_owned(),
                        });
                    }
                }
                if matched.wildcard {
                    checked += 1;
                    violations.push(SeamViolation {
                        check: "S4",
                        subject: format!("a wildcard arm in {owner}"),
                        detail: format!(
                            "a replay dispatches `{ENUM}` with `_ =>`, so a new variant \
                             compiles without being handled. Replace it with explicit arms."
                        ),
                    });
                }
            }
        }

        // 2. The seed decision is made exactly once.
        let seeds = Self::definitions(files, SEED);
        if seeds.len() != 1 {
            violations.push(SeamViolation {
                check: "S4",
                subject: format!("`{SEED}` is defined {} times", seeds.len()),
                detail: format!(
                    "the seed decision must be made in exactly one place. Found at: \
                     {}. D-14 was two copies of this decision, only one of which \
                     was ever exercised.",
                    seeds
                        .iter()
                        .map(|(l, _)| l.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }

        // 3. Every replay reaches the seed, wherever in the chain it sits.
        //
        //    The verdict is about the *chain*, not the method: a
        //    `rebuild_projection` that is a one-line delegation into an
        //    internal helper has not re-derived the decision, and reporting it
        //    as having done so would be reporting a false seam against code
        //    that routes through the seed exactly as the D-14 fix intends.
        for chain in &bodies.chains {
            checked += 1;
            if !chain.reaches_seed {
                violations.push(SeamViolation {
                    check: "S4",
                    subject: format!("{REPLAY} at {}", chain.entry),
                    detail: format!(
                        "neither this method nor anything it delegates to calls \
                         `{SEED}` (chain: {}). A replay that decides for itself which \
                         event seeds a projection is a second copy of the decision, \
                         and only the copy that is tested is ever right.",
                        chain.path.join(" -> ")
                    ),
                });
            }
        }

        let mut allowlisted = 0usize;
        if seeds.len() == 1 {
            allowlisted += 1;
        }
        allowlisted += usize::from(bodies.chains.iter().all(|c| c.reaches_seed));

        CheckOutcome {
            summary: CheckSummary {
                checked,
                allowlisted,
            },
            violations,
        }
    }

    /// Every `match` on `JobEvent` inside one block, with the variants it names
    /// explicitly and whether it also has a wildcard.
    pub fn matchers_in(loc: &str, block: &syn::Block) -> Vec<(String, Matched)> {
        let mut out = Vec::new();
        let mut v = MatcherVisitor {
            loc: loc.to_owned(),
            out: &mut out,
        };
        v.visit_block(block);
        out
    }

    /// `(file:line, calls_seed_projection)` for every `fn rebuild_projection`.
    pub fn definitions(files: &[SourceFile], name: &str) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        for file in files {
            if file.is_test {
                continue;
            }
            let mut v = DefinitionVisitor {
                name,
                file,
                out: &mut out,
            };
            v.visit_file(&file.syn);
        }
        out
    }
}

/// One function body in the workspace, with a name to be called by.
struct Body<'a> {
    loc: String,
    block: &'a syn::Block,
    /// Whether this is the `apply` method of a `ProjectionApply` impl.
    is_apply: bool,
}

/// One `rebuild_projection` implementation, followed through its delegations.
///
/// Bodies are arena indices, not references: the arena is owned by
/// `ReplayBodies`, so holding borrows into it here would be a self-reference.
struct Chain {
    entry: String,
    /// The bodies that make up the replay, entry first.
    bodies: Vec<usize>,
    path: Vec<String>,
    reaches_seed: bool,
}

/// The set of bodies a replay is made of, and one chain per implementation.
struct ReplayBodies<'a> {
    arena: Vec<Body<'a>>,
    /// Every body a replay dispatches an event in: the `apply` method of each
    /// `ProjectionApply` impl, plus every chain.
    all: Vec<usize>,
    chains: Vec<Chain>,
}

/// A delegation hop is only followed when the body is *nothing but* the call.
///
/// A method that does real work and then returns another function's result
/// has its own decisions in it, and following the tail would excuse them.
fn delegates_to(block: &syn::Block) -> Option<String> {
    if block.stmts.len() != 1 {
        return None;
    }
    let syn::Stmt::Expr(expr, None) = &block.stmts[0] else {
        return None;
    };
    peel(expr).callee()
}

/// Strip the wrappers a forwarder is wrapped in — `.await`, `&`, `( … )` —
/// and name the function it ultimately calls.
fn peel(expr: &syn::Expr) -> Peeled<'_> {
    match expr {
        syn::Expr::Await(a) => peel(&a.base),
        syn::Expr::Group(g) => peel(&g.expr),
        syn::Expr::Paren(p) => peel(&p.expr),
        syn::Expr::Reference(r) => peel(&r.expr),
        // A method call is a forwarder in either of two shapes, and the two
        // name different functions:
        //
        //   `self.rebuild(x)`        — the method on the receiver
        //   `ops::projections::rebuild(x).await` — a free function named by a
        //                                    multi-segment path, reached as a
        //                                    method call because of `.await`
        //
        // Reading the receiver's tail for the first shape yields `self`, and
        // following "self" is following nothing.
        syn::Expr::MethodCall(m) => match &*m.receiver {
            syn::Expr::Path(p) if p.path.segments.len() == 1 => Peeled::Call(Some(&m.method)),
            syn::Expr::Path(p) => Peeled::Call(last_ident(&p.path)),
            _ => Peeled::NotACall,
        },
        syn::Expr::Call(c) => match &*c.func {
            syn::Expr::Path(p) => Peeled::Call(last_ident(&p.path)),
            _ => Peeled::NotACall,
        },
        _ => Peeled::NotACall,
    }
}

enum Peeled<'a> {
    Call(Option<&'a syn::Ident>),
    NotACall,
}

impl Peeled<'_> {
    fn callee(self) -> Option<String> {
        match self {
            Self::Call(Some(i)) => Some(i.to_string()),
            Self::Call(None) | Self::NotACall => None,
        }
    }
}

fn last_ident(path: &syn::Path) -> Option<&syn::Ident> {
    path.segments.last().map(|s| &s.ident)
}

/// The last segment of a call's callee, if the call names a function at all.
///
/// Used for the seed: `e.seed_projection()` is a method call, so the method
/// name is the reference, not the receiver.
fn seed_referenced(block: &syn::Block) -> bool {
    let mut finder = SeedCallFinder { called: false };
    finder.visit_block(block);
    finder.called
}

impl<'a> ReplayBodies<'a> {
    /// Index every non-test function body once, so that a body reached by
    /// name and a body reached by traversal are the same entry and
    /// deduplication is an integer comparison rather than a pointer one.
    fn index(files: &'a [SourceFile]) -> (Vec<Body<'a>>, BTreeMap<String, Vec<usize>>) {
        let mut bodies: Vec<Body<'a>> = Vec::new();
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for file in files {
            if file.is_test {
                continue;
            }
            for item in &file.syn.items {
                match item {
                    syn::Item::Fn(f) => {
                        by_name
                            .entry(f.sig.ident.to_string())
                            .or_default()
                            .push(bodies.len());
                        bodies.push(Body {
                            loc: file.locate(f),
                            block: &f.block,
                            is_apply: false,
                        });
                    }
                    syn::Item::Impl(i) => {
                        let is_apply = i
                            .trait_
                            .as_ref()
                            .is_some_and(|(_, p, _)| super::path_tail(p) == APPLY_TRAIT);
                        for m in &i.items {
                            let syn::ImplItem::Fn(f) = m else { continue };
                            by_name
                                .entry(f.sig.ident.to_string())
                                .or_default()
                                .push(bodies.len());
                            bodies.push(Body {
                                loc: file.locate(f),
                                block: &f.block,
                                is_apply,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        (bodies, by_name)
    }

    fn collect(files: &'a [SourceFile]) -> Self {
        let (arena, by_name) = Self::index(files);

        // The dispatch point: every `impl … ProjectionApply { fn apply }`.
        // These are in scope for the coverage rule whether or not a
        // `rebuild_projection` is found — `apply` is where an event is
        // dispatched, and a workspace with no replay entry point still has a
        // dispatcher to get wrong.
        let mut all: Vec<usize> = arena
            .iter()
            .enumerate()
            .filter(|(_, b)| b.is_apply)
            .map(|(i, _)| i)
            .collect();

        // One chain per `rebuild_projection`, followed through delegations.
        let mut chains = Vec::new();
        if let Some(entries) = by_name.get(REPLAY) {
            for i in entries {
                chains.push(Self::walk(*i, &arena, &by_name));
            }
        }
        for chain in &chains {
            for b in &chain.bodies {
                if !all.contains(b) {
                    all.push(*b);
                }
            }
        }

        Self { arena, all, chains }
    }

    fn walk(start: usize, arena: &[Body<'a>], by_name: &BTreeMap<String, Vec<usize>>) -> Chain {
        let mut bodies = vec![start];
        let mut path = vec![arena[start].loc.clone()];
        let mut reaches_seed = seed_referenced(arena[start].block);
        // A short bound rather than no bound: a chain longer than this is not
        // a forwarder chain, it is a call graph, and walking that would make
        // both the check's cost and its claims unbounded.
        let mut hops = 0;
        while !reaches_seed && hops < MAX_HOPS {
            hops += 1;
            let Some(next_name) = delegates_to(arena[bodies[bodies.len() - 1]].block) else {
                break;
            };
            let Some(i) = by_name.get(&next_name).and_then(|v| v.first()) else {
                break;
            };
            let next = *i;
            if bodies.contains(&next) {
                break;
            }
            bodies.push(next);
            path.push(arena[next].loc.clone());
            reaches_seed = seed_referenced(arena[next].block);
        }
        Chain {
            entry: arena[start].loc.clone(),
            bodies,
            path,
            reaches_seed,
        }
    }

    /// The bodies a replay dispatches events in, deduplicated.
    pub fn all(&self) -> impl Iterator<Item = &Body<'a>> {
        self.all.iter().map(move |i| &self.arena[*i])
    }
}

/// The bound on how far a delegation chain is followed.
const MAX_HOPS: usize = 4;

/// What one `match` on `JobEvent` covers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Matched {
    pub handled: BTreeSet<String>,
    pub wildcard: bool,
}

struct MatcherVisitor<'a> {
    loc: String,
    out: &'a mut Vec<(String, Matched)>,
}

impl<'ast> Visit<'ast> for MatcherVisitor<'_> {
    fn visit_expr_match(&mut self, node: &syn::ExprMatch) {
        // A `JobEvent` match is identified by its **arms**, not by its
        // scrutinee. The scrutinee is nearly always a local binding —
        // `match e {`, `match &event {`, `match self {` — and none of those
        // name the enum, so a check that insists on finding `JobEvent` in the
        // scrutinee sees zero matchers in a codebase where nothing is wrong.
        // An arm that names `JobEvent::…` is a `JobEvent` match under any
        // reading, and looking there also picks up `Self::…` in an
        // `impl JobEvent` block for free.
        let mut m = Matched::default();
        for arm in &node.arms {
            match &arm.pat {
                // `_` is `Pat::Wild`. Watching only `Pat::Path` misses every
                // catch-all in the language, which is precisely the arm this
                // check exists to notice.
                syn::Pat::Wild(_) => m.wildcard = true,
                syn::Pat::Path(p) => {
                    if p.path.is_ident("_") {
                        m.wildcard = true;
                    }
                    if let Some(id) = variant_under(&p.path, ENUM) {
                        m.handled.insert(id.to_string());
                    }
                }
                syn::Pat::Struct(p) => {
                    if let Some(id) = variant_under(&p.path, ENUM) {
                        m.handled.insert(id.to_string());
                    }
                }
                syn::Pat::TupleStruct(p) => {
                    if let Some(id) = variant_under(&p.path, ENUM) {
                        m.handled.insert(id.to_string());
                    }
                }
                _ => {}
            }
        }
        if !m.handled.is_empty() {
            self.out.push((self.loc.clone(), m));
        }
        syn::visit::visit_expr_match(self, node);
    }
}

struct DefinitionVisitor<'a> {
    name: &'a str,
    file: &'a SourceFile,
    out: &'a mut Vec<(String, bool)>,
}

impl<'ast> Visit<'ast> for DefinitionVisitor<'_> {
    /// A free function named `self.name`.
    fn visit_item_fn(&mut self, node: &syn::ItemFn) {
        if node.sig.ident == self.name {
            self.record(&node.block, node.sig.ident.span());
        }
        syn::visit::visit_item_fn(self, node);
    }

    /// A method named `self.name`.
    ///
    /// `seed_projection` is `impl JobEvent { pub fn seed_projection }`, not a
    /// free function. A visitor that only looked at `ItemFn` would report
    /// "defined 0 times" — the same shape as the seed decision being made
    /// nowhere at all, which is a worse thing to hear than it is to read.
    fn visit_impl_item_fn(&mut self, node: &syn::ImplItemFn) {
        if node.sig.ident == self.name {
            self.record(&node.block, node.sig.ident.span());
        }
        syn::visit::visit_impl_item_fn(self, node);
    }

    /// A trait method's declaration. Counted so that a default body added to
    /// the trait is visible to the check rather than invisible to it.
    fn visit_trait_item_fn(&mut self, node: &syn::TraitItemFn) {
        if let Some(block) = &node.default
            && node.sig.ident == self.name
        {
            self.record(block, node.sig.ident.span());
        }

        syn::visit::visit_trait_item_fn(self, node);
    }
}

impl DefinitionVisitor<'_> {
    fn record(&mut self, block: &syn::Block, span: proc_macro2::Span) {
        let mut finder = SeedCallFinder { called: false };
        finder.visit_block(block);
        self.out.push((self.file.locate_span(span), finder.called));
    }
}

struct SeedCallFinder {
    called: bool,
}

impl<'ast> Visit<'ast> for SeedCallFinder {
    fn visit_expr_call(&mut self, node: &syn::ExprCall) {
        if let syn::Expr::Path(p) = &*node.func
            && p.path.segments.last().is_some_and(|s| s.ident == SEED)
        {
            self.called = true;
        }
        syn::visit::visit_expr_call(self, node);
    }

    /// `e.seed_projection()` is a **method call**, not a path call.
    ///
    /// `seed_projection` is `impl JobEvent { pub fn seed_projection }`, so
    /// every use of it that reads naturally is `some_event.seed_projection()`
    /// and reaches `visit_expr_method_call` rather than `visit_expr_call`. A
    /// finder that watches only the latter would report every replay as
    /// re-deriving the decision, and the D-14 fix as still in place.
    fn visit_expr_method_call(&mut self, node: &syn::ExprMethodCall) {
        if node.method == SEED {
            self.called = true;
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

/// The variant a pattern names, when `path` is `enum_name::Variant` (with any
/// number of leading qualifiers).
///
/// The enum is a segment to look for rather than the tail to compare: in
/// `JobEvent::CheckPassed` the tail is the variant.
fn variant_under<'p>(path: &'p syn::Path, enum_name: &str) -> Option<&'p syn::Ident> {
    let last = path.segments.last()?;
    let under = path
        .segments
        .iter()
        .take(path.segments.len().saturating_sub(1))
        .any(|s| s.ident == enum_name);
    under.then_some(&last.ident)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).expect("parse")
    }

    fn file(source: &str) -> Vec<SourceFile> {
        vec![src("crates/ironmaint-state/src/apply.rs", parse(source))]
    }

    /// The body of the first free function in `source`, which is all these
    /// tests ever need.
    fn block_of(file: &syn::File) -> syn::Block {
        file.items
            .iter()
            .find_map(|i| match i {
                syn::Item::Fn(f) => Some(*f.block.clone()),
                _ => None,
            })
            .expect("a free function")
    }

    fn src(rel: &str, syn: syn::File) -> SourceFile {
        SourceFile {
            rel: rel.to_owned(),
            path: std::path::PathBuf::from(rel),
            syn,
            is_test: false,
        }
    }

    #[test]
    fn a_match_naming_every_variant_is_fully_covered() {
        let f = parse(
            "fn f(e: &JobEvent) { match e {\
                 JobEvent::A => 1, JobEvent::B => 2, JobEvent::C => 3, } }",
        );
        let m = &EventsCheck::matchers_in("t", &block_of(&f))[0].1;
        assert_eq!(m.handled.len(), 3);
        assert!(!m.wildcard);
    }

    #[test]
    fn a_wildcard_arm_is_reported_and_the_missing_variant_with_it() {
        // The compile-time half of S4 is the match being exhaustive; a `_`
        // arm removes it silently. Both the wildcard and the uncovered
        // variant must appear, because either one alone is diagnosable and
        // both together say what happened.
        let f = parse("fn f(e: &JobEvent) { match e { JobEvent::A => 1, _ => 2 } }");
        let m = &EventsCheck::matchers_in("t", &block_of(&f))[0].1;
        assert!(m.wildcard);
        assert!(!m.handled.contains("C"));
    }

    #[test]
    fn a_match_on_something_else_is_not_a_matcher() {
        let f = parse("fn f(e: &Other) { match e { Other::A => 1 } }");
        assert!(EventsCheck::matchers_in("t", &block_of(&f)).is_empty());
    }

    #[test]
    fn one_seed_definition_is_one_site() {
        let f = file("fn seed_projection() {}");
        assert_eq!(EventsCheck::definitions(&f, SEED).len(), 1);
    }

    #[test]
    fn two_seed_definitions_are_two_sites() {
        // This is D-14's shape, and the whole reason S4 exists.
        let f = file("fn seed_projection() {} fn seed_projection() {}");
        assert_eq!(EventsCheck::definitions(&f, SEED).len(), 2);
    }

    #[test]
    fn a_replay_that_calls_seed_projection_is_recognised() {
        let f = file("fn rebuild_projection() { let p = e.seed_projection()?; }");
        assert_eq!(
            EventsCheck::definitions(&f, REPLAY),
            vec![("crates/ironmaint-state/src/apply.rs:1".to_owned(), true)]
        );
    }

    #[test]
    fn a_replay_that_re_derives_the_decision_is_recognised_as_one_that_does_not() {
        let f = file("fn rebuild_projection() { let p = decide_myself(e); }");
        assert!(!EventsCheck::definitions(&f, REPLAY)[0].1);
    }

    #[test]
    fn a_search_for_one_variant_is_not_a_replay_dispatcher() {
        // `load_resume_record` in `service.rs` is this shape: a `find_map`
        // over the event log with `_ => None`, scanning for one specific
        // variant. The wildcard is the correct code, and a check that policed
        // every `match` on `JobEvent` anywhere would report it as a dropped
        // event — which is why coverage is scoped to replay bodies instead.
        let f = parse(
            "fn load(events: &[E]) -> Option<JobEvent> {\
                 events.iter().find_map(|e| match &e.event {\
                     JobEvent::ResumeRecorded(r) => Some(r.clone()),\
                     _ => None,\
                 }) }",
        );
        let m = &EventsCheck::matchers_in("service.rs:966", &block_of(&f))[0].1;
        assert!(m.wildcard);
        assert_eq!(m.handled.len(), 1);
    }

    #[test]
    fn a_delegating_replay_reaches_the_seed_through_its_delegate() {
        // The SQLite backend's `rebuild_projection` is a one-line forward into
        // an internal helper, and the helper is where the seed is chosen.
        // Judging the method alone reported a seam against the code the D-14
        // fix produced.
        let f = parse(
            "fn rebuild_projection(&self) { self.rebuild() }\
             fn rebuild(&self) { let p = e.seed_projection()?; Ok(p) }",
        );
        let files = [src("t", f)];
        let bodies = ReplayBodies::collect(&files);
        let chain = &bodies.chains[0];
        assert!(chain.reaches_seed, "{:?}", chain.path);
        assert_eq!(chain.bodies.len(), 2);
    }

    #[test]
    fn a_replay_that_re_derives_the_seed_is_caught_through_its_delegate() {
        let f = parse(
            "fn rebuild_projection(&self) { self.rebuild() }\
             fn rebuild(&self) { let p = decide_myself(e); Ok(p) }",
        );
        let files = [src("t", f)];
        let bodies = ReplayBodies::collect(&files);
        let chain = &bodies.chains[0];
        assert!(!chain.reaches_seed);
        assert_eq!(chain.bodies.len(), 2);
    }

    #[test]
    fn a_body_that_does_its_own_work_is_not_a_delegate() {
        // Returning another function's result is a forwarder. Computing
        // something and *then* returning it has its own decisions in it, and
        // following the tail would excuse them.
        let f = parse(
            "fn rebuild_projection(&self) { let x = prepare(); self.rebuild(x) }\
             fn rebuild(&self, x: X) { let p = decide_myself(x); Ok(p) }",
        );
        let files = [src("t", f)];
        let bodies = ReplayBodies::collect(&files);
        let chain = &bodies.chains[0];
        assert!(!chain.reaches_seed);
        assert_eq!(chain.bodies.len(), 1);
    }

    #[test]
    fn the_apply_of_a_projection_apply_impl_is_in_scope_with_no_replay_at_all() {
        // A workspace with a dispatcher but no `rebuild_projection` still has
        // the place a variant could go unhandled.
        let f = parse(
            "trait ProjectionApply { fn apply(&self) -> u8; }\
             impl ProjectionApply for P {\
                 fn apply(&self) -> u8 { match self { P::A => 1, _ => 2 } } }",
        );
        let files = [src("t", f)];
        let bodies = ReplayBodies::collect(&files);
        let all: Vec<_> = bodies.all().collect();
        assert_eq!(all.len(), 1);
        assert!(all[0].is_apply);
    }
}
