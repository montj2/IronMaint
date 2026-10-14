//! `debian/changelog` first-entry reader for the candidate capture path.
//!
//! `WorkspaceManager::capture_candidate` (PHASE-0B.md §6.7) used to
//! construct every candidate with the placeholder
//! `PackageVersion::new("0+ironmaint")`. The hermetic fixtures were
//! written to match that placeholder, so the version cross-check in
//! `debian.inspect.source_preparation` always passed; a real Debian
//! package with a changelog version such as `1.0.0-1` would have
//! failed the identity check. P1 finding #1 of
//! `doc/PHASE-1-FEEDBACK.md` calls this out: the candidate's
//! identity is supposed to be bound to the source tree, not to a
//! fixed sentinel.
//!
//! The fix has two halves. The runtime now plumbs an optional
//! `expected_version` through `candidate.capture`; when the caller
//! supplies one, the candidate uses it directly and the
//! source-preparation check records a version-mismatch diagnostic
//! if the changelog disagrees. When the caller does not supply
//! one, this module reads the first entry of `debian/changelog`
//! and returns its version. The `capture_candidate` body uses the
//! returned version (or falls back to the historical
//! `0+ironmaint` sentinel when no changelog is present, so
//! non-Debian workspaces and the existing workspace-level
//! integration tests behave the same as before).
//!
//! The parser is hand-rolled against the same grammar the
//! `bins/ironmaint-debian-tool` source_preparation check uses
//! (PHASE-1.md §16's "focused parser crates" preference). The
//! `debian/changelog` header is a degenerate RFC 822 line of the
//! form
//!
//! ```text
//! <source> (<version>) <distribution>; urgency=<urgency>
//! ```
//!
//! which is simple enough that a pure-Rust split-by-parentheses
//! parser is what every other Debian-aware Rust tool uses. The
//! shape of the parser is preserved from the 1C.1 source — the
//! only change is that it now lives in
//! `ironmaint-workspace` so the candidate capture path can
//! resolve the version before the runtime dispatches the
//! `source_preparation` check.

use std::path::Path;

use ironmaint_core::PackageVersion;

use crate::error::{WorkspaceError, WorkspaceErrorKind};

/// Read `<workspace_root>/debian/changelog` and return the
/// first entry's version.
///
/// - `Ok(Some(v))` — the changelog exists and the first entry
///   parses; `v` is the version the candidate should use.
/// - `Ok(None)` — the changelog is absent (non-Debian workspace
///   or a workspace that has not yet had a Debian source tree
///   staged). The caller should fall back to its placeholder
///   so non-Debian workspaces behave exactly as before.
/// - `Err(_)` — the changelog exists but cannot be read at all
///   (e.g. permission denied on the file). The caller should
///   surface the error to the agent rather than silently
///   substituting the placeholder, because a Debian workspace
///   that the runtime cannot read is an infrastructure error
///   the source-preparation check is the right place to
///   diagnose. The body of the file is not parsed; an
///   unparseable changelog is `Ok(None)` (the source-preparation
///   check will record the parse failure on its own report).
pub fn read_changelog_first_entry(
    workspace_root: &Path,
) -> Result<Option<PackageVersion>, WorkspaceError> {
    let changelog_path = workspace_root.join("debian").join("changelog");
    if !changelog_path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&changelog_path).map_err(|e| {
        WorkspaceError::new(WorkspaceErrorKind::Other(format!(
            "cannot read changelog at {}: {e}",
            changelog_path.display()
        )))
    })?;
    let version_str = match parse_changelog_first_entry(&text) {
        Some(v) => v,
        None => return Ok(None),
    };
    Ok(PackageVersion::new(version_str).ok())
}

/// Pure parser, exported for unit tests. Returns the
/// `<version>` token from the first `<source> (<version>) …`
/// header of a `debian/changelog` body, or `None` if the body
/// has no parseable first entry.
fn parse_changelog_first_entry(body: &str) -> Option<String> {
    let header = body.lines().find(|line| !line.trim().is_empty())?;
    let open = header.find('(')?;
    let close = header[open..].find(')')? + open;
    if close <= open + 1 {
        return None;
    }
    let source = header[..open].trim();
    let version = header[open + 1..close].trim();
    if source.is_empty() || version.is_empty() {
        return None;
    }
    Some(version.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_header() {
        let parsed = parse_changelog_first_entry(
            "example (1.0.0-1) unstable; urgency=medium\n\n  * Initial release.\n",
        )
        .expect("canonical header parses");
        assert_eq!(parsed, "1.0.0-1");
    }

    #[test]
    fn skips_leading_blank_lines() {
        let parsed = parse_changelog_first_entry("\n\nfoo (2.0.0) stable; urgency=low\n")
            .expect("leading blanks skipped");
        assert_eq!(parsed, "2.0.0");
    }

    #[test]
    fn missing_urgency_is_tolerated() {
        let parsed = parse_changelog_first_entry("bar (3.4.5-2) experimental\n").expect("parses");
        assert_eq!(parsed, "3.4.5-2");
    }

    #[test]
    fn unparseable_header_returns_none() {
        assert!(parse_changelog_first_entry("not a valid header\n").is_none());
        assert!(parse_changelog_first_entry("").is_none());
        assert!(parse_changelog_first_entry("\n\n").is_none());
    }
}
