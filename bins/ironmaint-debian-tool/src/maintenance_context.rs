//! `debian.inspect.maintenance_context` — 1C.3 real check.
//!
//! PHASE-1.md §13 / §19 / §26. The 1C.3 check is the
//! *maintenance-context* view of the same Debian source
//! tree the 1C.2 check inventorys. The 1C.2 report is
//! the *source-tree inventory* (what is on disk); the
//! 1C.3 report is the *maintenance context* (how is the
//! package being maintained? — patch stack, watch
//! configuration, gbp branch settings, upstream
//! relationship metadata).
//!
//! The wire format follows the 1C.2 precedent:
//!
//! - Read `ironmaint.candidate_input.v3` from stdin
//!   (one document, EOF-terminated, deserialised into
//!   the `CandidateInput` struct).
//! - Write the versioned JSON report
//!   (`DebianMaintenanceContextV1`) to stdout (one
//!   document, EOF-terminated).
//!
//! The 1C.3 RED commit's `run()` returns the *minimal*
//! struct (verdict + provenance + diagnostics only); the
//! 1C.3 GREEN commit's `run()` returns the comprehensive
//! struct with the §26 fields populated. The §97
//! schema-snapshot tool (`xtask verify-schemas`) catches
//! GREEN-side drift: `cargo run -p xtask --
//! verify-schemas --write` regenerates
//! `schemas/DebianMaintenanceContextV1.json` against the
//! new struct.

use std::path::PathBuf;

use serde::Deserialize;

use crate::parsers::{control, copyright};
use crate::report::{
    DebianMaintenanceContextV1, Diagnostic, MAINTENANCE_CONTEXT_REPORT_KIND,
    MAINTENANCE_CONTEXT_SCHEMA_VERSION, Provenance, UpstreamRelationship, Verdict,
};

/// The runtime's `ironmaint.candidate_input.v3` payload
/// (PHASE-1.md §7). 1C.3 reuses 1C.2's v3 shape — no
/// schema bump is needed because the new fields are
/// optional and 1C.2's `serde(default)` attributes
/// already accept absent fields.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct CandidateInput {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub job_id: String,
    #[serde(default)]
    pub check_id: String,
    pub package: Package,
    #[serde(default)]
    pub workspace_path: Option<PathBuf>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub tree: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Package {
    pub name: String,
    pub version: String,
}

/// Run the 1C.3 maintenance-context check.
///
/// The 1C.3 RED commit's `run()` returns the *minimal*
/// struct (verdict + provenance + diagnostics only). The
/// §26 comprehensive fields (patches, tests, watch, gbp,
/// maintainer scripts, upstream_relationship) are
/// reserved for the 1C.3 GREEN commit's
/// `run()` — the parser composition there re-uses the
/// 1C.2 helpers (`read_patches`, `read_tests_control`,
/// `read_watch`, `read_gbp_config`, `read_maintainer_scripts`)
/// and adds the new `parsers::copyright` /
/// `parsers::control` helpers for the
/// `UpstreamRelationship` block.
pub fn run(input: &CandidateInput) -> DebianMaintenanceContextV1 {
    let workspace_path = input.workspace_path.clone();
    let (verdict, diagnostics) = match workspace_path.as_ref() {
        None => (
            Verdict::InfrastructureError,
            vec![Diagnostic {
                code: "E_WORKSPACE_PATH_MISSING".to_string(),
                message: "candidate input did not include workspace_path".to_string(),
                path: None,
            }],
        ),
        Some(p) if !p.is_dir() => (
            Verdict::InfrastructureError,
            vec![Diagnostic {
                code: "E_WORKSPACE_NOT_DIR".to_string(),
                message: format!("workspace_path {} is not a directory", p.display()),
                path: Some(p.clone()),
            }],
        ),
        Some(p) => {
            let debian_dir = p.join("debian");
            if !debian_dir.is_dir() {
                (
                    Verdict::Fail,
                    vec![Diagnostic {
                        code: "E_DEBIAN_DIR_MISSING".to_string(),
                        message: format!("{} does not exist", debian_dir.display()),
                        path: Some(debian_dir),
                    }],
                )
            } else {
                (Verdict::Pass, Vec::new())
            }
        }
    };

    // The 1C.3 RED commit's `run()` ignores the §26
    // comprehensive fields; the 1C.3 GREEN commit's
    // `run()` calls the parser composition here. The
    // helpers are imported in GREEN; the imports below
    // ensure the `parsers` module is reachable from this
    // path so the GREEN commit's body is a single
    // edit to the `run()` body.
    let _upstream = copyright::read_upstream_metadata(&debian_dir_path(&workspace_path));
    let _vcs = control::read_vcs_metadata(&debian_dir_path(&workspace_path));

    // Pre-build the upstream-relationship block from the
    // hermetic-fixture's known fields. The GREEN commit
    // replaces this placeholder with the real merge of
    // `copyright::UpstreamMetadata` and `control::VcsMetadata`.
    let upstream_relationship = UpstreamRelationship::default();
    let _ = upstream_relationship;

    DebianMaintenanceContextV1 {
        schema_version: MAINTENANCE_CONTEXT_SCHEMA_VERSION,
        report_kind: MAINTENANCE_CONTEXT_REPORT_KIND.to_string(),
        provenance: Provenance {
            candidate_fingerprint: input.fingerprint.clone().unwrap_or_default(),
            candidate_commit: input.commit.clone().unwrap_or_default(),
            candidate_tree: input.tree.clone().unwrap_or_default(),
            inspector_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        verdict,
        diagnostics,
    }
}

fn debian_dir_path(workspace_path: &Option<PathBuf>) -> std::path::PathBuf {
    match workspace_path {
        Some(p) => p.join("debian"),
        None => std::path::PathBuf::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_input() -> CandidateInput {
        CandidateInput {
            schema: "ironmaint.candidate_input.v3".to_string(),
            job_id: "0192a000-0000-0000-0000-000000000001".to_string(),
            check_id: "0192a000-0000-0000-0000-000000000002".to_string(),
            package: Package {
                name: "example".to_string(),
                version: "0+ironmaint".to_string(),
            },
            workspace_path: Some(fixture_workspace()),
            fingerprint: Some("blake3:test".to_string()),
            commit: Some("a".repeat(40)),
            tree: Some("b".repeat(40)),
        }
    }

    fn fixture_workspace() -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        workspace_root.join("containers/fixtures/debian/example-1.0")
    }

    /// The §17 pass path: the good fixture's
    /// `debian/` exists, so the minimal struct
    /// emits a `Verdict::Pass` verdict. The 1C.3
    /// GREEN commit's `run()` also populates the
    /// §26 comprehensive fields.
    #[test]
    fn good_fixture_yields_a_pass_verdict() {
        let report = run(&good_input());
        assert_eq!(
            report.verdict,
            Verdict::Pass,
            "the well-formed hermetic fixture's minimal struct verdict: {report:?}"
        );
    }

    /// The 1C.3 schema is the *comprehensive* §26
    /// report. In the 1C.3 GREEN commit, the struct
    /// carries `patches`, `tests`, `watch`, `gbp`,
    /// `maintainer_scripts`, `upstream_relationship`.
    /// In the 1C.3 RED commit, those fields are
    /// absent (the struct is the minimal shape);
    /// reading the wire JSON shows them as missing.
    /// The assertion is on the JSON shape so the
    /// test compiles in both states — in RED the
    /// asserted fields are absent (so the assertion
    /// fails), in GREEN the fields are present and
    /// non-empty (so the assertion passes).
    #[test]
    fn comprehensive_maintenance_context_is_populated() {
        let report = run(&good_input());
        let json = serde_json::to_value(&report).expect("serialise report");
        let obj = json.as_object().expect("report is an object");

        let patches = obj
            .get("patches")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("patches must be present and an array: {json}"));
        assert!(
            !patches.is_empty(),
            "the fixture's `debian/patches/series` references one patch; \
             patches must be non-empty: {json}"
        );

        let tests = obj
            .get("tests")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("tests must be present and an array: {json}"));
        assert!(
            !tests.is_empty(),
            "the fixture's `debian/tests/control` declares one autopkgtest; \
             tests must be non-empty: {json}"
        );

        let watch = obj
            .get("watch")
            .unwrap_or_else(|| panic!("watch must be present: {json}"));
        assert!(
            watch.is_object(),
            "the fixture's `debian/watch` parses as a v4 watch object: {json}"
        );

        let gbp = obj
            .get("gbp")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("gbp must be present and an object: {json}"));
        assert_eq!(
            gbp.get("gbp_conf_present").and_then(|v| v.as_bool()),
            Some(true),
            "the 1C.3 fixture's `debian/gbp.conf` is present: {json}"
        );

        let maintainer_scripts = obj
            .get("maintainer_scripts")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("maintainer_scripts must be present and an object: {json}"));
        assert!(
            maintainer_scripts
                .get("postinst")
                .and_then(|v| v.as_object())
                .and_then(|o| o.get("executable"))
                .and_then(|v| v.as_bool())
                == Some(true),
            "the 1C.3 fixture's `debian/postinst` is executable: {json}"
        );

        let upstream_relationship = obj
            .get("upstream_relationship")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| {
                panic!("upstream_relationship must be present and an object: {json}")
            });
        assert!(
            upstream_relationship
                .get("upstream_name")
                .and_then(|v| v.as_str())
                == Some("example"),
            "the fixture's `debian/copyright` declares `Upstream-Name: example`: {json}"
        );
    }

    /// The §17 fail path: a workspace that is not
    /// a Debian source tree yields `Verdict::Fail`
    /// and the `E_DEBIAN_DIR_MISSING` diagnostic.
    #[test]
    fn missing_debian_directory_yields_fail_verdict() {
        let mut input = good_input();
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        input.workspace_path = Some(manifest.join("src"));
        let report = run(&input);
        assert_eq!(report.verdict, Verdict::Fail);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "E_DEBIAN_DIR_MISSING")
        );
    }

    /// The §17 `infrastructure_error` path: no
    /// `workspace_path` field. The tool's
    /// `E_WORKSPACE_PATH_MISSING` diagnostic is in
    /// the report; the normalizer maps the verdict
    /// to `Unclassifiable` and the runtime surfaces
    /// it as `ExecutorError::InfrastructureFailed`.
    #[test]
    fn missing_workspace_path_yields_infrastructure_error() {
        let mut input = good_input();
        input.workspace_path = None;
        let report = run(&input);
        assert_eq!(report.verdict, Verdict::InfrastructureError);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "E_WORKSPACE_PATH_MISSING")
        );
    }

    /// The §17 `infrastructure_error` path:
    /// `workspace_path` is a file, not a directory.
    /// The tool's `E_WORKSPACE_NOT_DIR` diagnostic
    /// is in the report.
    #[test]
    fn workspace_path_pointing_at_a_file_yields_infrastructure_error() {
        let mut input = good_input();
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // `Cargo.toml` is a file, not a directory.
        input.workspace_path = Some(manifest.join("Cargo.toml"));
        let report = run(&input);
        assert_eq!(report.verdict, Verdict::InfrastructureError);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "E_WORKSPACE_NOT_DIR")
        );
    }
}
