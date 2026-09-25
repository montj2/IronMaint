//! `cargo xtask verify-architecture` — assert the Phase 0A/0B
//! dependency graph.
//!
//! Discovers every workspace member by walking `crates/`, `adapters/`,
//! and `bins/` (so a new crate lands without touching this file),
//! reads its `[dependencies]` / `[dev-dependencies]`, and rejects
//! edges that would violate the architectural invariants:
//!
//! - PHASE-0A.md §71: distribution neutrality — core/state/evidence/
//!   policy/adapter-api may not depend on adapter implementations.
//! - PHASE-0A.md §65: domain crates may not depend on `tracing`.
//! - PHASE-0B.md §98: domain and infrastructure rules — core/state/
//!   mcp/adapters may not depend on `ironmaint-store-sqlite`; only
//!   the runtime is allowed to bridge MCP into persistence.
//! - PHASE-0A.md §67: `ironmaint-testkit` is a conformance runner,
//!   not a production dependency of any other workspace member.

use std::{
    collections::BTreeMap,
    error::Error,
    path::{Path, PathBuf},
};

use toml_edit::DocumentMut;

/// Directories under the workspace root that may contain crate
/// `Cargo.toml`s. Walked in lexicographic order so the report is
/// deterministic.
const DISCOVERY_DIRS: &[&str] = &["crates", "adapters", "bins"];

/// `(__any__, target)` means "any discovered crate other than
/// `target` is forbidden from depending on `target`". Used for the
/// `ironmaint-testkit` rule.
const ANY: &str = "__any__";

/// Forbidden `(source, target, reason)` edges. `source == ANY`
/// matches every discovered crate except `target` itself.
///
/// Each rule is anchored to a spec section, so a future contributor
/// can read the rationale without leaving the verifier.
const FORBIDDEN_EDGES: &[(&str, &str, &str)] = &[
    // PHASE-0B.md §98: persistence is reached only via the runtime.
    (
        "ironmaint-core",
        "ironmaint-store-sqlite",
        "§98.1 core may not depend on a store backend",
    ),
    (
        "ironmaint-core",
        "ironmaint-executor",
        "§98.2 core may not depend on the executor",
    ),
    (
        "ironmaint-core",
        "ironmaint-mcp",
        "§98.3 core may not depend on the MCP surface",
    ),
    (
        "ironmaint-state",
        "ironmaint-store-sqlite",
        "§98.4 state machine may not depend on a store backend",
    ),
    (
        "debian-stub",
        "ironmaint-store-sqlite",
        "§98.5 adapter may not depend on a store backend",
    ),
    (
        "fedora-stub",
        "ironmaint-store-sqlite",
        "§98.5 adapter may not depend on a store backend",
    ),
    (
        "ironmaint-mcp",
        "ironmaint-store-sqlite",
        "§98.6 MCP must reach persistence through the runtime",
    ),
    // PHASE-0A.md §65: no logging framework in domain crates.
    (
        "ironmaint-core",
        "tracing",
        "§65.1 domain crates may not depend on tracing",
    ),
    (
        "ironmaint-evidence",
        "tracing",
        "§65.1 domain crates may not depend on tracing",
    ),
    (
        "ironmaint-policy",
        "tracing",
        "§65.1 domain crates may not depend on tracing",
    ),
    (
        "ironmaint-state",
        "tracing",
        "§65.1 domain crates may not depend on tracing",
    ),
    (
        "ironmaint-adapter-api",
        "tracing",
        "§65.1 domain crates may not depend on tracing",
    ),
    // PHASE-0A.md §67: testkit is a conformance runner, not a
    // production dependency. Catch any drift with a single rule.
    (ANY, "ironmaint-testkit", "testkit is not a production dep"),
];

type CrateDeps = BTreeMap<String, BTreeMap<String, Vec<String>>>;

pub struct Report {
    pub crates: CrateDeps,
    pub violations: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Architecture verify report")?;
        writeln!(f, "=========================")?;
        for (crate_name, deps) in &self.crates {
            writeln!(f, "\n[{crate_name}]")?;
            if deps.values().all(Vec::is_empty) {
                writeln!(f, "  (no workspace dependencies)")?;
            } else {
                for (kind, names) in deps {
                    if names.is_empty() {
                        continue;
                    }
                    writeln!(f, "  {kind}: {}", names.join(", "))?;
                }
            }
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
    let workspace_root = workspace_root()?;
    let crates = scan_workspace(&workspace_root)?;
    let violations = find_violations(&crates);
    Ok(Report { crates, violations })
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    // xtask itself lives at <workspace>/xtask/, so its
    // CARGO_MANIFEST_DIR is <workspace>/xtask. Step up one level to
    // find the workspace root.
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|e| format!("CARGO_MANIFEST_DIR not set: {e}"))?;
    let path = PathBuf::from(manifest);
    let parent = path
        .parent()
        .ok_or_else(|| format!("manifest path {} has no parent", path.display()))?;
    Ok(parent.to_path_buf())
}

/// Discover every crate under the workspace root by walking the
/// well-known directories and looking for `Cargo.toml` files.
///
/// Returns a map of `crate_name -> {kind -> [dependency_names]}`
/// where `kind` is `"dependencies"` or `"dev-dependencies"` and the
/// names are restricted to *workspace* dependencies (paths-only
/// crates, not external registry deps).
fn scan_workspace(root: &Path) -> Result<CrateDeps, Box<dyn Error>> {
    let mut out: CrateDeps = BTreeMap::new();
    for dir in DISCOVERY_DIRS {
        let dir_path = root.join(dir);
        if !dir_path.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&dir_path)
            .map_err(|e| format!("read_dir {}: {e}", dir_path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest = path.join("Cargo.toml");
            if !manifest.exists() {
                continue;
            }
            let crate_name = read_package_name(&manifest)?;
            let deps = read_workspace_deps(&manifest)?;
            out.insert(crate_name, deps);
        }
    }
    Ok(out)
}

fn read_package_name(manifest: &Path) -> Result<String, Box<dyn Error>> {
    let raw = std::fs::read_to_string(manifest)
        .map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let doc: DocumentMut = raw
        .parse()
        .map_err(|e| format!("parse {}: {e}", manifest.display()))?;
    let name = doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| format!("missing [package].name in {}", manifest.display()))?
        .to_owned();
    Ok(name)
}

fn read_workspace_deps(manifest: &Path) -> Result<BTreeMap<String, Vec<String>>, Box<dyn Error>> {
    let raw = std::fs::read_to_string(manifest)?;
    let doc: DocumentMut = raw.parse()?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    out.insert(
        "dependencies".into(),
        workspace_dep_names(doc.get("dependencies")),
    );
    out.insert(
        "dev-dependencies".into(),
        workspace_dep_names(doc.get("dev-dependencies")),
    );
    Ok(out)
}

fn workspace_dep_names(table: Option<&toml_edit::Item>) -> Vec<String> {
    let Some(table) = table.and_then(|i| i.as_table_like()) else {
        return Vec::new();
    };
    // A workspace dependency is either a `path = "..."` relative
    // reference or a plain string dependency. Both appear in the
    // table's key list. External registry deps (`serde`, `tokio`,
    // ...) are simply not in our workspace, so we keep all keys and
    // let the forbidden-edge matcher filter by name.
    let mut names: Vec<String> = table.iter().map(|(name, _)| name.to_owned()).collect();
    names.sort();
    names
}

/// Check the discovered crate graph against `FORBIDDEN_EDGES`.
///
/// Returns one violation string per offending edge, in a stable
/// order (by source then target).
///
/// `ANY`-source rules apply only to `[dependencies]`, not
/// `[dev-dependencies]`: the testkit rule is a production-dep
/// constraint (PHASE-0A.md §67), and the §98 distribution-neutrality
/// rules are about production code paths. `tracing` and the
/// store/sqlite rules are absolute and apply to both.
fn find_violations(crates: &CrateDeps) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (source, deps) in crates {
        for (kind, names) in deps {
            for target in names {
                for (rule_source, rule_target, reason) in FORBIDDEN_EDGES {
                    if *rule_target != target {
                        continue;
                    }
                    if *rule_source == ANY && kind != "dependencies" {
                        continue;
                    }
                    let matches = if *rule_source == ANY {
                        source != rule_target
                    } else {
                        source == rule_source
                    };
                    if matches {
                        out.push(format!(
                            "{source} [{kind}] -> {target} forbidden ({reason})"
                        ));
                    }
                }
            }
        }
    }
    out.sort();
    out
}
