//! `source.import` URL allowlist (PHASE-2 PR 2.0-E).
//!
//! Replaces the PHASE-1.1 PR 1.1.G `Vec<String>` `starts_with`
//! allowlist with a parsed policy. A `SourceImportPolicy` matches
//! a `url::Url` by exact scheme, case-insensitive host, exact
//! (or default) port, and an optional path prefix. The path
//! prefix is anchored at the start of the URL's path component
//! and the trailing-slash on a prefix is significant — a
//! policy of `https://github.com/debian/` parses to
//! `path_prefix = "/debian/"` and matches `/debian/foo` but
//! NOT `/debian-mirror/foo`; a policy of
//! `https://github.com/debian` (no trailing slash) parses to
//! `path_prefix = "/debian"` and matches both.
//!
//! ## Distribution neutrality
//!
//! This type lives in `ironmaint-core` because it is a pure
//! value type: it carries no Debian / Fedora / sbuild
//! semantics, and `url::Url` is the only runtime dep.
//! `crates/ironmaint-mcp` re-imports the type via the
//! `ironmaint-core` re-export and the dispatcher's allowlist
//! check is the only place that exercises `matches`.
//!
//! ## Why a parsed type, not a `String::starts_with`
//!
//! A `String` prefix allowlist silently accepts any substring
//! of the configured value. A policy of `https://github.com/debian/`
//! matched `https://github.com/debian-mirror/foo` under the
//! old implementation, which is a phishing-shaped false
//! negative. The parsed policy requires the operator to opt in
//! to sub-paths explicitly (the optional `path_prefix` field).
//!
//! ## Repeated `source.import` semantics
//!
//! A repeated `source.import` for the same `job_id` re-clones
//! the remote. This is the documented behaviour (PHASE-2 PR
//! 2.0-E user decision). The policy does not encode the
//! re-clone decision; that lives in
//! `WorkspaceManager::import_git_remote`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::CoreError;

/// A parsed `source.import` URL allowlist entry.
///
/// The four fields are matched against the candidate URL as
/// follows:
///
/// - `scheme` — exact match (case-insensitive, per `url::Url`).
/// - `host` — exact match (case-insensitive). A `file://`
///   policy with `host = ""` matches any host-less `file://`
///   URL.
/// - `port` — exact match. A `None` policy-port matches the
///   URL's default for its scheme (per
///   `url::Url::default_port(scheme)`); a `None` URL-port
///   matches the policy-port when the policy-port is the
///   default for the policy's scheme. Both `None` matches
///   both `None`.
/// - `path_prefix` — anchored prefix on `url::Url::path()`.
///   `None` means "any path is fine". A trailing slash in
///   the prefix is significant: `Some("/debian/")` matches
///   `/debian/foo` but not `/debian-mirror/foo`;
///   `Some("/debian")` matches both.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct SourceImportPolicy {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path_prefix: Option<String>,
}

/// Errors produced by [`SourceImportPolicy::parse`]. Each variant
/// carries the offending input so the dispatcher's diagnostic
/// names exactly what was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceImportPolicyError {
    /// The string is empty.
    Empty,
    /// The string does not parse as a URL.
    InvalidUrl { input: String, why: String },
    /// The URL has no scheme (a relative URL is not a policy).
    MissingScheme { input: String },
    /// The URL has no host and the scheme is not `file`.
    /// `file://` policies with no host are supported (the
    /// local filesystem path is the "host").
    MissingHost { input: String },
}

impl std::fmt::Display for SourceImportPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("source import policy string is empty"),
            Self::InvalidUrl { input, why } => {
                write!(
                    f,
                    "source import policy `{input}` is not a valid URL: {why}"
                )
            }
            Self::MissingScheme { input } => {
                write!(
                    f,
                    "source import policy `{input}` has no scheme (a relative URL is not a policy)"
                )
            }
            Self::MissingHost { input } => {
                write!(f, "source import policy `{input}` has no host")
            }
        }
    }
}

impl std::error::Error for SourceImportPolicyError {}

impl Default for SourceImportPolicy {
    /// The default policy is `https://` with no host — a deny-
    /// all sentinel. A `Vec<SourceImportPolicy>` of
    /// `[Default::default()]` rejects every `source.import`
    /// call.
    fn default() -> Self {
        Self {
            scheme: "https".into(),
            host: String::new(),
            port: None,
            path_prefix: None,
        }
    }
}

impl std::fmt::Display for SourceImportPolicy {
    /// Re-emit the policy in `scheme://host[:port][/path]`
    /// form so it round-trips through [`Self::parse`]. The
    /// result is the canonical string a daemon would have
    /// written to its config file.
    ///
    /// Trailing-slash on `path_prefix` is preserved: `Display`
    /// emits `path_prefix` verbatim, and `parse` reads it
    /// verbatim. The matcher's directory-vs-broader semantics
    /// survive the round trip.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.scheme)?;
        f.write_str("://")?;
        if !(self.scheme == "file" && self.host.is_empty()) {
            f.write_str(&self.host)?;
        }
        if let Some(p) = self.port {
            write!(f, ":{p}")?;
        }
        if let Some(prefix) = &self.path_prefix {
            f.write_str(prefix)?;
        } else if self.scheme == "file" && self.host.is_empty() {
            // A host-less `file://` policy without a path is
            // ambiguous when re-parsed (the URL grammar requires
            // a path). The empty path is encoded as `/` so
            // round-tripping through `Display` → `parse`
            // reproduces a host-less `file://` policy with
            // `path_prefix = None`.
            f.write_str("/")?;
        }
        Ok(())
    }
}

impl SourceImportPolicy {
    /// Parse a `scheme://host[:port][/path-prefix]` string into
    /// a `SourceImportPolicy`.
    ///
    /// The trailing-slash on the path is preserved (a
    /// disambiguator for the matcher's "directory prefix" vs
    /// "broader prefix" semantics); leading and trailing
    /// whitespace is stripped.
    ///
    /// # Errors
    /// - `Empty` if the input is empty or whitespace-only.
    /// - `InvalidUrl` if the input is not parseable as a URL.
    /// - `MissingScheme` if the parsed URL has no scheme
    ///   (a relative URL).
    /// - `MissingHost` if the parsed URL has no host and the
    ///   scheme is not `file`.
    pub fn parse(s: &str) -> Result<Self, SourceImportPolicyError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(SourceImportPolicyError::Empty);
        }
        let url = Url::parse(trimmed).map_err(|e| SourceImportPolicyError::InvalidUrl {
            input: trimmed.to_string(),
            why: e.to_string(),
        })?;
        let scheme = url.scheme();
        if scheme.is_empty() {
            return Err(SourceImportPolicyError::MissingScheme {
                input: trimmed.to_string(),
            });
        }
        // `file://` is the one scheme the daemon legitimately
        // configures with no host (the local filesystem path
        // is the "host"). Treat a host-less `file://` as a
        // special case: host = "" matches any host-less
        // `file://` URL the caller supplies.
        let host = match url.host_str() {
            Some(h) => h.to_string(),
            None if scheme == "file" => String::new(),
            None => {
                return Err(SourceImportPolicyError::MissingHost {
                    input: trimmed.to_string(),
                });
            }
        };
        let port = url.port();
        // The path is part of the policy. The matcher treats
        // `None` as "any path is fine", so an empty or
        // single-slash path is `None`. A non-trivial path is
        // `Some(path)` with the trailing slash preserved —
        // the matcher uses the trailing slash to disambiguate
        // "directory prefix" from "broader prefix".
        let path_prefix = {
            let p = url.path();
            if p.is_empty() || p == "/" {
                None
            } else {
                Some(p.to_string())
            }
        };
        Ok(Self {
            scheme: scheme.to_string(),
            host,
            port,
            path_prefix,
        })
    }

    /// Decide whether this policy allows `url`.
    ///
    /// The matcher is:
    ///
    /// - scheme — exact, case-insensitive (per `url::Url`).
    /// - host — exact, case-insensitive. A `file://` policy
    ///   with `host = ""` matches any host-less `file://` URL.
    /// - port — `None` policy port is interpreted as the
    ///   policy's scheme's default port. `None` URL port is
    ///   the URL's effective port (i.e. its explicit port, or
    ///   the default for its scheme). Both must agree.
    /// - path — when `path_prefix` is `None`, any path is
    ///   fine. When `Some(prefix)`, the URL's path must start
    ///   with the prefix literally (the trailing slash is
    ///   significant).
    #[must_use]
    pub fn matches(&self, url: &Url) -> bool {
        if !self.scheme.eq_ignore_ascii_case(url.scheme()) {
            return false;
        }
        // Host comparison is case-insensitive. `url::Url::host_str`
        // returns the host in lowercase form (per WHATWG URL
        // parsing) when the input is ASCII, so a policy
        // authored in any case will round-trip.
        let url_host = url.host_str().unwrap_or("");
        if self.host.is_empty() && self.scheme == "file" {
            // host-less file:// policies only match host-less
            // file:// URLs.
            if !url_host.is_empty() {
                return false;
            }
        } else if !self.host.eq_ignore_ascii_case(url_host) {
            return false;
        }
        // Port: compute the effective port on each side and
        // compare. `None` policy port is the default for the
        // policy's scheme. `None` URL port is the default for
        // the URL's scheme (the schemes are already known
        // equal). Both `None` means the schemes have no
        // known default — match.
        let policy_eff = self.port.or_else(|| default_port(&self.scheme));
        let url_eff = url.port_or_known_default();
        match (policy_eff, url_eff) {
            (None, None) => {}
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
        // Path: `None` matches any path. `Some(prefix)` is a
        // literal anchored prefix.
        match &self.path_prefix {
            None => true,
            Some(prefix) => url.path().starts_with(prefix.as_str()),
        }
    }
}

impl From<SourceImportPolicyError> for CoreError {
    fn from(e: SourceImportPolicyError) -> Self {
        CoreError::invalid_url(e.to_string())
    }
}

/// Default IANA-registered port for a scheme. Returns `None`
/// for schemes without a registered default (e.g. `file`,
/// `git`, custom protocols).
///
/// The list covers the schemes a `source.import` policy
/// legitimately sees: `http`, `https`, `ftp`. New schemes
/// added to the policy grammar in future PRs should also be
/// listed here.
fn default_port(scheme: &str) -> Option<u16> {
    match scheme.to_ascii_lowercase().as_str() {
        "http" => Some(80),
        "https" => Some(443),
        "ws" => Some(80),
        "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(input: &str) -> SourceImportPolicy {
        SourceImportPolicy::parse(input).expect("static policy must parse")
    }

    #[test]
    fn parse_file_url() {
        let policy = p("file:///var/lib/repos/foo.git");
        assert_eq!(policy.scheme, "file");
        assert_eq!(policy.host, "");
        assert_eq!(
            policy.path_prefix.as_deref(),
            Some("/var/lib/repos/foo.git")
        );
    }

    #[test]
    fn parse_https_with_path_trailing_slash() {
        // The trailing slash is preserved — it is the
        // operator's signal of "this is a directory".
        let policy = p("https://github.com/debian/");
        assert_eq!(policy.scheme, "https");
        assert_eq!(policy.host, "github.com");
        assert_eq!(policy.path_prefix.as_deref(), Some("/debian/"));
    }

    #[test]
    fn parse_https_with_path_no_trailing_slash() {
        // No trailing slash → broader prefix semantics.
        let policy = p("https://github.com/debian");
        assert_eq!(policy.path_prefix.as_deref(), Some("/debian"));
    }

    #[test]
    fn parse_https_with_port() {
        let policy = p("https://example.com:8443/");
        assert_eq!(policy.scheme, "https");
        assert_eq!(policy.host, "example.com");
        assert_eq!(policy.port, Some(8443));
        assert_eq!(policy.path_prefix, None);
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(matches!(
            SourceImportPolicy::parse(""),
            Err(SourceImportPolicyError::Empty)
        ));
        assert!(matches!(
            SourceImportPolicy::parse("   "),
            Err(SourceImportPolicyError::Empty)
        ));
    }

    #[test]
    fn parse_rejects_scp_form() {
        // `git@host:path` is not a URL; `url::Url::parse` rejects.
        assert!(matches!(
            SourceImportPolicy::parse("git@github.com:foo/bar.git"),
            Err(SourceImportPolicyError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn matches_file_url_against_file_policy() {
        let policy = p("file:///var/lib/repos/foo.git");
        let url = Url::parse("file:///var/lib/repos/foo.git").unwrap();
        assert!(policy.matches(&url));
    }

    #[test]
    fn matches_https_path_prefix_does_not_overflow_to_sibling() {
        // The headline bug fix: `https://github.com/debian/`
        // (trailing slash → directory prefix) does NOT match
        // `https://github.com/debian-mirror/foo`.
        let policy = p("https://github.com/debian/");
        let mirror = Url::parse("https://github.com/debian-mirror/foo.git").unwrap();
        assert!(!policy.matches(&mirror), "prefix must not over-match");
        let real = Url::parse("https://github.com/debian/foo.git").unwrap();
        assert!(policy.matches(&real), "real path under prefix must match");
    }

    #[test]
    fn matches_https_path_prefix_with_no_trailing_slash() {
        // `https://github.com/debian` (no trailing slash) is
        // the broader prefix: matches both `/debian/foo` AND
        // `/debian-mirror/foo` because the operator opted
        // into the broader semantics by omitting the slash.
        let policy = p("https://github.com/debian");
        let mirror = Url::parse("https://github.com/debian-mirror/foo.git").unwrap();
        assert!(
            policy.matches(&mirror),
            "policy without trailing slash is the broader one"
        );
        let real = Url::parse("https://github.com/debian/foo.git").unwrap();
        assert!(policy.matches(&real));
    }

    #[test]
    fn matches_https_default_port_matches() {
        // `https://example.com/` (no explicit port) → matches
        // `https://example.com/foo`.
        let policy = p("https://example.com/");
        let url = Url::parse("https://example.com/foo").unwrap();
        assert!(policy.matches(&url));
    }

    #[test]
    fn matches_https_default_port_does_not_match_explicit_port() {
        // Policy `https://example.com/` says "the default port
        // for https, which is 443". A URL that carries an
        // explicit non-default port does NOT match.
        let policy = p("https://example.com/");
        let url = Url::parse("https://example.com:8443/foo").unwrap();
        assert!(!policy.matches(&url));
    }

    #[test]
    fn matches_https_explicit_port_matches() {
        let policy = p("https://example.com:8443/");
        let url = Url::parse("https://example.com:8443/foo").unwrap();
        assert!(policy.matches(&url));
        let url_wrong_port = Url::parse("https://example.com:443/foo").unwrap();
        assert!(!policy.matches(&url_wrong_port));
    }

    #[test]
    fn matches_host_is_case_insensitive() {
        let policy = p("https://Example.com/");
        let url = Url::parse("https://example.com/foo").unwrap();
        assert!(policy.matches(&url));
    }

    #[test]
    fn matches_scheme_is_case_insensitive() {
        let policy = p("HTTPS://example.com/");
        let url = Url::parse("https://example.com/foo").unwrap();
        assert!(policy.matches(&url));
    }

    #[test]
    fn matches_different_scheme_rejected() {
        let policy = p("file:///tmp/foo");
        let url = Url::parse("https://example.com/foo").unwrap();
        assert!(!policy.matches(&url));
    }

    #[test]
    fn matches_javascript_scheme_rejected() {
        let policy = p("file:///tmp/foo");
        let url = Url::parse("javascript:alert(1)").unwrap();
        assert!(!policy.matches(&url));
    }

    #[test]
    fn matches_userinfo_ignored() {
        // The policy is host-keyed; a URL that carries
        // credentials but resolves to the same host matches.
        let policy = p("https://example.com/");
        let url = Url::parse("https://user:pass@example.com/foo").unwrap();
        assert!(policy.matches(&url));
    }

    #[test]
    fn matches_punycode_host_matches_ascii() {
        // IDN: `bücher.example` in Unicode is the same host
        // as `xn--bcher-kva.example` in punycode. `url::Url`
        // normalises to the punycode form on parse.
        let policy = p("https://xn--bcher-kva.example/");
        let url_unicode = Url::parse("https://bücher.example/foo").unwrap();
        assert!(
            policy.matches(&url_unicode),
            "punycode policy must match Unicode URL after WHATWG normalisation"
        );
    }

    #[test]
    fn default_policy_denies_all_real_urls() {
        // `Default::default()` is `https://` with no host.
        let policy = SourceImportPolicy::default();
        let url = Url::parse("https://example.com/foo").unwrap();
        assert!(!policy.matches(&url));
    }

    #[test]
    fn serde_round_trip() {
        let policy = SourceImportPolicy {
            scheme: "https".into(),
            host: "github.com".into(),
            port: None,
            path_prefix: Some("/debian/".into()),
        };
        let json = serde_json::to_string(&policy).unwrap();
        let parsed: SourceImportPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, parsed);
    }

    #[test]
    fn display_parse_round_trip() {
        // The CLI / env-var surface round-trips a policy
        // through `Display` then `parse`. The matcher must
        // see the same policy after a round trip.
        for input in [
            "file:///var/lib/repos/foo.git",
            "https://github.com/debian/",
            "https://github.com/debian",
            "https://example.com:8443/",
            "https://Example.com/",
        ] {
            let policy = p(input);
            let rendered = policy.to_string();
            let reparsed = SourceImportPolicy::parse(&rendered).expect("round-trip parse");
            assert_eq!(
                policy, reparsed,
                "Display → parse must reproduce the same policy (input was `{input}`)"
            );
        }
    }
}
