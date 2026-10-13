//! `debian/copyright` (DEP-5) upstream-metadata parser
//! (1C.3's only new data source beyond the 1C.2
//! inventory).
//!
//! Reads the first paragraph of `debian/copyright` for
//! `Upstream-Name` and `Upstream-Contact`, and any
//! `Files:` paragraph for the `Source:` field (the
//! canonical upstream URL for the Files stanza). The
//! `Source:` field appears in the first paragraph and
//! in each `Files:` paragraph; this parser takes the
//! first non-empty value across the document so the
//! 1C.3 report carries the most-specific match.

use std::path::Path;
use std::str::FromStr;

use deb822_lossless::Deb822;

/// The DEP-5 upstream-metadata block.
///
/// Three `Option<String>` fields, all `None` when the
/// corresponding copyright paragraph is absent or
/// empty. The 1C.3 `UpstreamRelationship` struct
/// carries these three alongside the three
/// `VcsMetadata` fields (from `debian/control`) — see
/// `maintenance_context::run` for the merge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpstreamMetadata {
    /// The DEP-5 `Upstream-Name:` field. `None`
    /// when absent.
    pub upstream_name: Option<String>,
    /// The DEP-5 `Upstream-Contact:` field.
    /// `None` when absent.
    pub upstream_contact: Option<String>,
    /// The DEP-5 `Source:` field. `None` when
    /// absent.
    pub upstream_url: Option<String>,
}

/// Read the upstream-metadata block from
/// `debian/copyright` at `<debian_dir>/copyright`.
///
/// `None` everywhere when the file is absent or
/// unparsable (the latter is a parser bug, not a
/// runtime error — the 1C.3 verdict still goes to
/// `Pass` because the field is observational, not
/// mandatory).
#[must_use]
pub fn read_upstream_metadata(debian_dir: &Path) -> UpstreamMetadata {
    let path = debian_dir.join("copyright");
    let Ok(body) = std::fs::read_to_string(&path) else {
        return UpstreamMetadata::default();
    };
    parse_upstream_metadata(&body)
}

fn parse_upstream_metadata(body: &str) -> UpstreamMetadata {
    let Ok(parsed) = Deb822::from_str(body) else {
        return UpstreamMetadata::default();
    };
    let paragraphs: Vec<_> = parsed.paragraphs().collect();
    let first = paragraphs.first();
    let mut upstream_name: Option<String> = None;
    let mut upstream_contact: Option<String> = None;
    let mut upstream_url: Option<String> = None;
    if let Some(p) = first {
        upstream_name = opt_field(p, "Upstream-Name");
        upstream_contact = opt_field(p, "Upstream-Contact");
    }
    for p in &paragraphs {
        if upstream_url.is_none() {
            upstream_url = opt_field(p, "Source");
        }
    }
    UpstreamMetadata {
        upstream_name,
        upstream_contact,
        upstream_url,
    }
}

fn opt_field(para: &deb822_lossless::Paragraph, key: &str) -> Option<String> {
    let v = para.get(key)?.trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_all_none() {
        let md = parse_upstream_metadata("");
        assert!(md.upstream_name.is_none());
        assert!(md.upstream_contact.is_none());
        assert!(md.upstream_url.is_none());
    }

    #[test]
    fn first_paragraph_upstream_name_and_contact() {
        let body = "\
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: example
Upstream-Contact: IronMaint Test <test@example.invalid>

Files: *
Copyright: 2026 IronMaint Test <test@example.invalid>
License: Expat
";
        let md = parse_upstream_metadata(body);
        assert_eq!(md.upstream_name.as_deref(), Some("example"));
        assert_eq!(
            md.upstream_contact.as_deref(),
            Some("IronMaint Test <test@example.invalid>")
        );
        assert!(md.upstream_url.is_none());
    }

    #[test]
    fn source_field_populated_from_files_paragraph() {
        let body = "\
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: example
Upstream-Contact: IronMaint Test <test@example.invalid>

Files: *
Copyright: 2026 IronMaint Test <test@example.invalid>
License: Expat
Source: https://example.invalid/example
";
        let md = parse_upstream_metadata(body);
        assert_eq!(md.upstream_name.as_deref(), Some("example"));
        assert_eq!(
            md.upstream_contact.as_deref(),
            Some("IronMaint Test <test@example.invalid>")
        );
        assert_eq!(
            md.upstream_url.as_deref(),
            Some("https://example.invalid/example")
        );
    }

    #[test]
    fn missing_copyright_yields_default() {
        let md = read_upstream_metadata(Path::new("/nonexistent/debian"));
        assert!(md.upstream_name.is_none());
        assert!(md.upstream_contact.is_none());
        assert!(md.upstream_url.is_none());
    }
}
