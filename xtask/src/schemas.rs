//! `cargo xtask verify-schemas` — regenerate JSON schemas and diff against
//! committed snapshots in `doc/schemas/`.
//!
//! Phase 0A.1 ships a skeleton: this module emits a placeholder report so
//! the subcommand is wired and the `--write` flag is plumbed. The real
//! schemars-driven implementation lands in 0A.6 alongside the JSON schema
//! generator.

use std::{error::Error, fmt::Write as _};

#[derive(Debug)]
pub struct Report {
    pub wrote: bool,
    pub committed: Vec<String>,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        writeln!(s, "Schemas verify report")?;
        writeln!(s, "======================")?;
        writeln!(
            s,
            "Phase 0A.1 skeleton — schemars integration lands in 0A.6 (PHASE-0A.md §61)."
        )?;
        writeln!(
            s,
            "Mode: {}",
            if self.wrote { "write" } else { "verify (diff)" }
        )?;
        if self.committed.is_empty() {
            writeln!(s, "No committed schemas found under doc/schemas/.")?;
        } else {
            writeln!(s, "Committed snapshots: {}", self.committed.len())?;
            for name in &self.committed {
                writeln!(s, "  - {name}")?;
            }
        }
        write!(f, "{s}")
    }
}

pub fn run(write: bool) -> Result<Report, Box<dyn Error>> {
    // 0A.1: enumerate committed snapshots only. The generator itself is
    // introduced in 0A.6 once every public durable type carries
    // `#[derive(JsonSchema)]`.
    let committed = scan_committed();
    Ok(Report {
        wrote: write,
        committed,
    })
}

fn scan_committed() -> Vec<String> {
    let Ok(root) = std::env::var("CARGO_MANIFEST_DIR") else {
        return Vec::new();
    };
    let schemas_dir = std::path::PathBuf::from(root).join("../doc/schemas");
    let Ok(read) = std::fs::read_dir(&schemas_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}
