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
fn schema_tool_names_eight() {
    use ironmaint_mcp::schema::tool_names;
    let names = tool_names();
    assert_eq!(names.len(), 8);
    assert!(names.iter().any(|n| n.0 == "job.create"));
    assert!(names.iter().any(|n| n.0 == "job.next_actions"));
}
