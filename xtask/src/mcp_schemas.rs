//! MCP schema snapshot verifier.
//!
//! Mirrors the shape of `xtask/src/schemas.rs`: it scans
//! committed `schemas/mcp/*.json` and compares them against the
//! live output of `ironmaint_mcp::schema::generate_all_schemas`.
//!
//! `cargo xtask verify-mcp-schemas` is in force from commit 15
//! onward. Run with `--write` to regenerate the snapshots.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use ironmaint_mcp::schema::{McpToolName, generate_all_schemas};

#[derive(Debug)]
pub struct McpSchemaReport {
    pub committed: Vec<String>,
    pub generated: Vec<String>,
    pub mismatches: Vec<String>,
}

impl McpSchemaReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    let mut p = std::env::current_dir()?;
    // xtask/src/mcp_schemas.rs is two levels below the workspace root.
    if p.ends_with("xtask") {
        p.pop();
    }
    Ok(p)
}

pub fn run(write: bool) -> Result<McpSchemaReport, Box<dyn Error>> {
    let root = workspace_root()?;
    let snapshot_dir = root.join("schemas").join("mcp");
    let committed = scan_committed(&snapshot_dir)?;
    let generated = generate_all_schemas();
    let generated_names: Vec<String> = generated.keys().cloned().map(|n| n.0).collect();
    let mut report = McpSchemaReport {
        committed: committed.keys().cloned().collect(),
        generated: generated_names.clone(),
        mismatches: Vec::new(),
    };

    for (name, value) in &generated {
        let path = snapshot_dir.join(format!("{}.json", name.0));
        let on_disk = committed.get(&name.0);
        let on_disk_str = on_disk.map(String::as_str).unwrap_or("");
        let generated_str = serde_json::to_string_pretty(value)?;
        if on_disk_str != generated_str {
            report.mismatches.push(format!("{}: drift", name.0));
            if write {
                fs::write(&path, &generated_str)?;
            }
        }
    }

    for name in &report.committed {
        if !generated_names.contains(name) {
            report.mismatches.push(format!("{name}: orphaned snapshot"));
            if write {
                fs::remove_file(snapshot_dir.join(format!("{name}.json")))?;
            }
        }
    }

    Ok(report)
}

fn scan_committed(dir: &Path) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut out = BTreeMap::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("bad file stem")?
            .to_string();
        let body = fs::read_to_string(&path)?;
        out.insert(name, body);
    }
    Ok(out)
}

#[allow(dead_code)]
fn _ensure_used() {
    let _ = McpToolName::from;
}
