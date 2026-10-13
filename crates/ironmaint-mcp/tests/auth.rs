//! MCP auth tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ironmaint_mcp::{AuthToken, TokenValidator};

#[test]
fn token_validator_accepts_matching() {
    let v = TokenValidator::new(AuthToken::new("secret"));
    assert!(v.validate("secret").is_ok());
}

#[test]
fn token_validator_rejects_wrong_length() {
    let v = TokenValidator::new(AuthToken::new("secret"));
    assert!(v.validate("secre").is_err());
}

#[test]
fn token_validator_rejects_wrong_bytes() {
    let v = TokenValidator::new(AuthToken::new("secret"));
    assert!(v.validate("secreX").is_err());
}

#[test]
fn schema_tool_names_ten() {
    use ironmaint_mcp::schema::tool_names;
    let names = tool_names();
    // 1A.4 (PHASE-1.md §31) added `evidence.list`, `evidence.get`, and
    // `evidence.artifact.read`, taking the surface from 11 to 14. The
    // assertions that *specific* tool names are present stay the same;
    // the count is what changed.
    assert_eq!(names.len(), 14);
    assert!(names.iter().any(|n| n.0 == "job.create"));
    assert!(names.iter().any(|n| n.0 == "job.reconcile"));
    assert!(names.iter().any(|n| n.0 == "job.next_actions"));
    assert!(names.iter().any(|n| n.0 == "evidence.list"));
    assert!(names.iter().any(|n| n.0 == "evidence.get"));
    assert!(names.iter().any(|n| n.0 == "evidence.artifact.read"));
}
