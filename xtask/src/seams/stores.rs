//! **S5 — both store backends expose the same surface.**
//!
//! ## What the compiler already gives us, and why this check is not that
//!
//! `IronMaintStore` is a marker trait with a blanket impl over ten
//! sub-traits. A type that does not implement all ten does not implement the
//! marker, and every call site is written against the marker. So *method
//! parity is already compiler-enforced* — `SqliteStore` cannot be missing a
//! `put_gate_result` and still satisfy the bound.
//!
//! The seam the compiler cannot see is **inherent** methods: the ones declared
//! in `impl MockStore { … }` rather than in a `impl CandidateStore for
//! MockStore`. Those are callable on the mock and absent from the real store.
//!
//! That is not a hypothetical. `MockStore` has three:
//!
//! - `read()` and `write()`, returning `RwLock` guards over mock-internal
//!   state. No real backend can have these, because no real backend has a
//!   process-local `RwLock` around its rows.
//! - `find_gate_result()`, the one the spec names: a test can call it, and
//!   production cannot, so a test written against it is a test that proves
//!   nothing about the system that ships.
//!
//! A test that compiles against the mock and could not compile against SQLite
//! is not caught by any existing check. That is the whole reason this one
//! exists.

use std::collections::BTreeSet;

use super::{CheckOutcome, CheckSummary, SeamViolation, SourceFile, path_tail, self_type_name};

/// The marker trait whose surface both backends must match.
const TRAIT: &str = "IronMaintStore";

/// `(backend, method, reason)` for an inherent method that exists on one
/// backend and not the other, and why the asymmetry is correct.
const ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "MockStore",
        "new",
        "Construction differs by design: the mock holds no files, so `new()` takes \
         nothing, while `SqliteStore::open` takes a config and a migrations dir. \
         Forcing a common constructor would mean a config the mock ignores.",
    ),
    (
        "MockStore",
        "read",
        "Mock-internal. Returns an `RwLockReadGuard` over in-process state; no real \
         backend has a process-local lock around its rows and cannot have one. \
         Private, so it is not callable from another crate.",
    ),
    (
        "MockStore",
        "write",
        "Mock-internal, as `read`. Returns an `RwLockWriteGuard` over in-process \
         state, which no real backend can offer; a real backend's writes go to \
         the database and are serialised there. Private, so it is not callable \
         from another crate.",
    ),
    (
        "MockStore",
        "find_gate_result",
        "**D-20: a real asymmetry, not a decision.** An inherent lookup the real \
         backend does not have. A test using it proves something about the mock \
         only. It is private today, which is why no test has yet been caught by \
         it; the check exists so that making it `pub` is a deliberate act.",
    ),
    (
        "SqliteStore",
        "open",
        "Construction, as `MockStore::new`. Takes a `SqliteStoreConfig` and applies \
         migrations; there is nothing for the mock to apply.",
    ),
    (
        "SqliteStore",
        "open_in_memory",
        "Construction, as `open`. A test affordance for SQLite, with no mock \
         counterpart because the mock is already in-memory.",
    ),
    (
        "SqliteStore",
        "config",
        "SQLite-only accessor returning the store's `SqliteStoreConfig`. The mock has \
         no configuration to return.",
    ),
];

pub struct StoresCheck;

/// The two backends whose surfaces must match.
const BACKENDS: [(&str, &str); 2] = [
    ("MockStore", "crates/ironmaint-store/src/mock.rs"),
    ("SqliteStore", "crates/ironmaint-store-sqlite/src/lib.rs"),
];

impl StoresCheck {
    pub fn run(files: &[SourceFile]) -> CheckOutcome {
        let mut violations = Vec::new();
        let mut allowlisted = 0usize;
        let mut checked = 0usize;

        // (a) The marker is not vacuous: every supertrait it names is
        // implemented by both backends. A typo in the bound would leave the
        // blanket impl applying to nothing, and every call site would still
        // compile against the marker.
        let supertraits = Self::supertraits(files);
        if supertraits.is_empty() {
            violations.push(SeamViolation {
                check: "S5",
                subject: TRAIT.to_owned(),
                detail: "no supertraits found; the marker trait is either empty or \
                         unparseable, and the check cannot run"
                    .to_owned(),
            });
        }
        for (name, path) in BACKENDS {
            for sup in &supertraits {
                checked += 1;
                if !Self::implements(files, path, name, sup) {
                    violations.push(SeamViolation {
                        check: "S5",
                        subject: format!("{name} does not implement {sup}"),
                        detail: format!(
                            "`{TRAIT}` names {sup} as a supertrait, but {path} has no \
                             `impl {sup} for {name}`. The blanket impl therefore does \
                             not apply to this backend."
                        ),
                    });
                }
            }
        }

        // (b) Inherent surfaces match. This is the half the compiler does not
        // cover.
        let mock = Self::inherent(files, BACKENDS[0].1, BACKENDS[0].0);
        let sqlite = Self::inherent(files, BACKENDS[1].1, BACKENDS[1].0);
        for (owner, mine, theirs) in [
            (BACKENDS[0].0, &mock, &sqlite),
            (BACKENDS[1].0, &sqlite, &mock),
        ] {
            for method in mine.difference(theirs) {
                checked += 1;
                match ALLOWLIST
                    .iter()
                    .find(|(b, m, _)| *b == owner && *m == method)
                {
                    Some((_, _, reason)) => {
                        allowlisted += 1;
                        // Surfaced rather than silently swallowed: an
                        // allowlist that grows invisibly is how a check stops
                        // meaning anything.
                        eprintln!("  note: {owner}::{method} — {reason}");
                    }
                    None => violations.push(SeamViolation {
                        check: "S5",
                        subject: format!("{owner}::{method}"),
                        detail: "an inherent method the other backend does not have. A \
                                 test can call this against the mock and cannot call \
                                 anything like it in production. Either give the other \
                                 backend the same method, or add a reasoned entry to \
                                 ALLOWLIST in xtask/src/seams/stores.rs."
                            .to_owned(),
                    }),
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

    /// The sub-trait names in `IronMaintStore`'s supertrait bound.
    pub fn supertraits(files: &[SourceFile]) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for file in files {
            for item in &file.syn.items {
                let syn::Item::Trait(t) = item else { continue };
                if t.ident != TRAIT {
                    continue;
                }
                for bound in &t.supertraits {
                    // Lifetimes and `?Sized` carry no method set and
                    // contribute nothing to compare.
                    if let syn::TypeParamBound::Trait(tb) = bound
                        && let Some(last) = tb.path.segments.last()
                    {
                        out.insert(last.ident.to_string());
                    }
                }
            }
        }
        out
    }

    /// Inherent (non-trait) methods declared on `ty` in `path`.
    pub fn inherent(files: &[SourceFile], path: &str, ty: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for file in files {
            if file.rel != path {
                continue;
            }
            for item in &file.syn.items {
                let syn::Item::Impl(i) = item else { continue };
                if i.trait_.is_some() || self_type_name(&i.self_ty).is_none_or(|n| n != ty) {
                    continue;
                }
                for m in &i.items {
                    if let syn::ImplItem::Fn(f) = m {
                        out.insert(f.sig.ident.to_string());
                    }
                }
            }
        }
        out
    }

    /// Whether `path` contains `impl <trait> for <ty>`.
    pub fn implements(files: &[SourceFile], path: &str, ty: &str, trait_name: &str) -> bool {
        files.iter().any(|file| {
            file.rel == path
                && file.syn.items.iter().any(|item| {
                    let syn::Item::Impl(i) = item else {
                        return false;
                    };
                    i.trait_
                        .as_ref()
                        .is_some_and(|(_, p, _)| path_tail(p) == trait_name)
                        && self_type_name(&i.self_ty).is_some_and(|n| n == ty)
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(rel: &str, src: &str) -> SourceFile {
        SourceFile {
            rel: rel.to_owned(),
            path: std::path::PathBuf::from(rel),
            syn: syn::parse_file(src).expect("parse"),
            is_test: false,
        }
    }

    const MOCK: &str = "crates/ironmaint-store/src/mock.rs";
    const SQLITE: &str = "crates/ironmaint-store-sqlite/src/lib.rs";

    #[test]
    fn supertraits_are_read_off_the_bound() {
        let f = vec![file(
            "crates/ironmaint-store/src/lib.rs",
            "pub trait IronMaintStore: EventStore + ProjectionStore + CandidateStore {}",
        )];
        let s = StoresCheck::supertraits(&f);
        assert_eq!(s.len(), 3);
        assert!(s.contains("EventStore"));
    }

    #[test]
    fn an_empty_marker_yields_no_supertraits() {
        let f = vec![file(
            "crates/ironmaint-store/src/lib.rs",
            "pub trait IronMaintStore {}",
        )];
        assert!(StoresCheck::supertraits(&f).is_empty());
    }

    #[test]
    fn inherent_and_trait_methods_are_counted_separately() {
        // A `find_gate_result` that is a trait method is parity; the same name
        // inherent is a seam. Conflating them would make the check silent on
        // the case the spec actually names.
        let f = vec![file(
            MOCK,
            "struct MockStore; impl MockStore { fn a(&self) {} }",
        )];
        assert!(StoresCheck::inherent(&f, MOCK, "MockStore").contains("a"));

        let f = vec![file(
            MOCK,
            "struct MockStore; trait T { fn a(&self); } impl T for MockStore { fn a(&self) {} }",
        )];
        assert!(!StoresCheck::inherent(&f, MOCK, "MockStore").contains("a"));
    }

    #[test]
    fn an_inherent_method_present_on_one_backend_only_is_the_seam() {
        let files = vec![
            file(
                MOCK,
                "struct MockStore; impl MockStore { fn only_here(&self) {} }",
            ),
            file(SQLITE, "struct SqliteStore; impl SqliteStore {}"),
        ];
        let mock = StoresCheck::inherent(&files, MOCK, "MockStore");
        let sqlite = StoresCheck::inherent(&files, SQLITE, "SqliteStore");
        assert!(mock.difference(&sqlite).any(|m| m == "only_here"));
        assert_eq!(sqlite.difference(&mock).count(), 0);
    }

    #[test]
    fn trait_implementation_is_detected_by_type_and_trait_name() {
        let f = vec![file(
            MOCK,
            "struct MockStore; trait EventStore { fn a(&self); } \
             impl EventStore for MockStore { fn a(&self) {} }",
        )];
        assert!(StoresCheck::implements(&f, MOCK, "MockStore", "EventStore"));
        assert!(!StoresCheck::implements(
            &f,
            MOCK,
            "SqliteStore",
            "EventStore"
        ));
        assert!(!StoresCheck::implements(&f, MOCK, "MockStore", "GateStore"));
    }

    #[test]
    fn an_allowlist_entry_names_a_method_that_really_is_asymmetric() {
        // A reason for a method that was renamed, or made symmetric, is a
        // reason for nothing — and it would keep the allowlist looking
        // justified while the list it justifies shrank.
        let files = vec![
            file(
                MOCK,
                "struct MockStore; impl MockStore { fn new()->Self{Self} \
                 fn read(&self){} fn write(&self){} fn find_gate_result(&self){} }",
            ),
            file(SQLITE, "struct SqliteStore;"),
        ];
        let mock = StoresCheck::inherent(&files, MOCK, "MockStore");
        let sqlite = StoresCheck::inherent(&files, SQLITE, "SqliteStore");
        for (backend, method, reason) in ALLOWLIST {
            if *backend != "MockStore" {
                continue;
            }
            assert!(
                mock.difference(&sqlite).any(|m| m == method),
                "allowlist excuses {backend}::{method}, which is no longer asymmetric"
            );
            assert!(reason.len() > 40, "{backend}::{method} has no reason");
        }
    }
}
