//! `cargo xtask verify-architecture` — assert the Phase 0A dependency graph.
//!
//! Phase 0A.1 ships a skeleton: this module walks every `Cargo.toml` in the
//! workspace, lists each crate's direct workspace dependencies, and reports
//! what it sees. It does NOT yet enforce forbidden edges; that lands in 0A.5
//! after all crates have their real dependencies declared. See
//! `doc/phases/PHASE-0A.md` §71 for the full forbidden-edge list.

use std::{
    collections::BTreeMap,
    error::Error,
    path::{Path, PathBuf},
};

use toml_edit::DocumentMut;

const WORKSPACE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";

type CrateDeps = BTreeMap<String, BTreeMap<String, Vec<String>>>;

#[derive(Debug)]
pub struct Report {
    pub crates: CrateDeps,
    pub violations: Vec<String>,
}

impl Report {
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
            if deps.is_empty() {
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
            writeln!(
                f,
                "\nNo violations (Phase 0A.1 skeleton — enforcement lands in 0A.5)."
            )?;
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
    Ok(Report {
        crates,
        violations: Vec::new(),
    })
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    // xtask itself lives at <workspace>/xtask/, so its CARGO_MANIFEST_DIR is
    // <workspace>/xtask. Step up one level to find the workspace root.
    let manifest = std::env::var(WORKSPACE_ROOT_ENV)
        .map_err(|e| format!("{WORKSPACE_ROOT_ENV} not set: {e}"))?;
    let path = PathBuf::from(manifest);
    let parent = path
        .parent()
        .ok_or_else(|| format!("manifest path {} has no parent", path.display()))?;
    Ok(parent.to_path_buf())
}

fn scan_workspace(root: &Path) -> Result<CrateDeps, Box<dyn Error>> {
    let mut out = BTreeMap::new();
    for manifest_rel in WORKSPACE_MANIFESTS {
        let manifest = root.join(manifest_rel);
        if !manifest.exists() {
            continue;
        }
        let crate_name = read_package_name(&manifest)?;
        let deps = read_workspace_deps(&manifest)?;
        out.insert(crate_name, deps);
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
        .ok_or_else(|| format!("{}: missing [package].name", manifest.display()))?
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
    let mut names: Vec<String> = table
        .iter()
        .map(|(name, _)| name.to_owned())
        .filter(|name| is_workspace_crate(name))
        .collect();
    names.sort();
    names
}

fn is_workspace_crate(name: &str) -> bool {
    WORKSPACE_CRATES.contains(&name)
}

const WORKSPACE_MANIFESTS: &[&str] = &[
    "xtask/Cargo.toml",
    "crates/ironmaint-core/Cargo.toml",
    "crates/ironmaint-evidence/Cargo.toml",
    "crates/ironmaint-policy/Cargo.toml",
    "crates/ironmaint-state/Cargo.toml",
    "crates/ironmaint-adapter-api/Cargo.toml",
    "crates/ironmaint-testkit/Cargo.toml",
    "adapters/debian-stub/Cargo.toml",
    "adapters/fedora-stub/Cargo.toml",
];

const WORKSPACE_CRATES: &[&str] = &[
    "ironmaint-core",
    "ironmaint-evidence",
    "ironmaint-policy",
    "ironmaint-state",
    "ironmaint-adapter-api",
    "ironmaint-testkit",
    "debian-stub",
    "fedora-stub",
];
