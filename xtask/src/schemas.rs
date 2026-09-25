//! `cargo xtask verify-schemas` — regenerate JSON schemas and diff against
//! committed snapshots in top-level `schemas/` (PHASE-0A.md §61).
//!
//! The generator is in-process: every wire type carries
//! `#[derive(JsonSchema)]` (PHASE-0A.md §61); xtask calls
//! `schemars::schema_for!(T)` for each spec-listed type and diffs the
//! resulting JSON against the committed snapshot at `schemas/<Name>.json`.
//!
//! Exit code is `FAILURE` when any mismatch exists; the developer runs
//! `cargo xtask verify-schemas --write` to accept the new schemas and
//! commit the resulting JSON verbatim.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MismatchKind {
    Added,
    Removed,
    Changed,
}

impl std::fmt::Display for MismatchKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added => write!(f, "added"),
            Self::Removed => write!(f, "removed"),
            Self::Changed => write!(f, "changed"),
        }
    }
}

#[derive(Debug)]
pub struct Mismatch {
    pub filename: String,
    pub kind: MismatchKind,
    pub detail: String,
}

#[derive(Debug)]
pub struct Report {
    pub generated: BTreeMap<String, String>,
    pub committed: Vec<String>,
    pub mismatches: Vec<Mismatch>,
    pub wrote: bool,
}

impl Report {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        writeln!(s, "Schemas verify report")?;
        writeln!(s, "======================")?;
        writeln!(
            s,
            "Mode: {}",
            if self.wrote { "write" } else { "verify (diff)" }
        )?;
        writeln!(s, "Generated: {} type(s)", self.generated.len())?;
        if self.wrote {
            writeln!(s, "Wrote {} snapshot(s) to schemas/.", self.generated.len())?;
        } else {
            writeln!(s, "Committed: {} snapshot(s)", self.committed.len())?;
            if self.mismatches.is_empty() {
                writeln!(s, "No drift.")?;
            } else {
                writeln!(s, "Mismatches: {}", self.mismatches.len())?;
                for m in &self.mismatches {
                    writeln!(s, "  - {} [{}] {}", m.filename, m.kind, m.detail)?;
                }
            }
        }
        write!(f, "{s}")
    }
}

pub fn run(write: bool) -> Result<Report, Box<dyn Error>> {
    let generated = generate_all()?;
    if write {
        write_snapshots(&generated)?;
        Ok(Report {
            generated,
            committed: Vec::new(),
            mismatches: Vec::new(),
            wrote: true,
        })
    } else {
        let committed = scan_committed();
        let mismatches = diff(&generated, &committed);
        Ok(Report {
            generated,
            committed: committed.keys().cloned().collect(),
            mismatches,
            wrote: false,
        })
    }
}

fn generate_all() -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut out = BTreeMap::new();
    out.insert(
        "Evidence.json".into(),
        serde_json::to_string_pretty(&schemars::schema_for!(ironmaint_evidence::Evidence))?,
    );
    out.insert(
        "Obligation.json".into(),
        serde_json::to_string_pretty(&schemars::schema_for!(ironmaint_policy::Obligation))?,
    );
    out.insert(
        "ReleaseCandidate.json".into(),
        serde_json::to_string_pretty(&schemars::schema_for!(ironmaint_policy::ReleaseCandidate))?,
    );
    out.insert(
        "PublicationPlan.json".into(),
        serde_json::to_string_pretty(&schemars::schema_for!(ironmaint_policy::PublicationPlan))?,
    );
    out.insert(
        "JobEvent.json".into(),
        serde_json::to_string_pretty(&schemars::schema_for!(ironmaint_state::JobEvent))?,
    );
    out.insert(
        "StateTransitioned.json".into(),
        serde_json::to_string_pretty(&schemars::schema_for!(ironmaint_state::StateTransitioned))?,
    );
    Ok(out)
}

fn schemas_dir() -> Result<PathBuf, Box<dyn Error>> {
    let root = std::env::var("CARGO_MANIFEST_DIR")?;
    let dir = PathBuf::from(root).join("../schemas");
    Ok(dir)
}

fn scan_committed() -> BTreeMap<String, String> {
    let Ok(dir) = schemas_dir() else {
        return BTreeMap::new();
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        out.insert(name, content);
    }
    out
}

fn diff(
    generated: &BTreeMap<String, String>,
    committed: &BTreeMap<String, String>,
) -> Vec<Mismatch> {
    let mut mismatches = Vec::new();
    for (name, generated_str) in generated {
        match committed.get(name) {
            None => mismatches.push(Mismatch {
                filename: name.clone(),
                kind: MismatchKind::Added,
                detail: "schema generated but no committed snapshot".into(),
            }),
            Some(stored) if stored.as_str() != generated_str.as_str() => {
                mismatches.push(Mismatch {
                    filename: name.clone(),
                    kind: MismatchKind::Changed,
                    detail: format!(
                        "expected {} bytes, got {} bytes",
                        stored.len(),
                        generated_str.len()
                    ),
                });
            }
            Some(_) => {}
        }
    }
    for name in committed.keys() {
        if !generated.contains_key(name) {
            mismatches.push(Mismatch {
                filename: name.clone(),
                kind: MismatchKind::Removed,
                detail: "committed snapshot has no generator".into(),
            });
        }
    }
    // Sort for stable display
    mismatches.sort_by(|a, b| a.filename.cmp(&b.filename));
    mismatches
}

fn write_snapshots(generated: &BTreeMap<String, String>) -> Result<(), Box<dyn Error>> {
    let dir = schemas_dir()?;
    std::fs::create_dir_all(&dir)?;
    for (name, content) in generated {
        std::fs::write(dir.join(name), content)?;
    }
    Ok(())
}

// Helper used by tests and main; not strictly required for the
// subcommand but mirrors the architecture.rs public surface.
#[allow(dead_code)]
pub fn schemas_path() -> Result<PathBuf, Box<dyn Error>> {
    let dir = schemas_dir()?;
    Ok(Path::new(&dir).canonicalize()?)
}
