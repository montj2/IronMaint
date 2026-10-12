//! `debian/control` (source paragraph) VCS-metadata
//! parser (1C.3's only new data source beyond the 1C.2
//! inventory).
//!
//! Reads the source paragraph (the first paragraph) of
//! `debian/control` for the `Homepage`, `Vcs-Git`, and
//! `Vcs-Browser` fields. These are the VCS / canonical
//! URLs for the package; the 1C.2 `SourceMetadata`
//! already captures them but the 1C.3 parser exposes
//! them as a standalone sub-struct so the
//! `UpstreamRelationship` block can carry them.

use std::path::Path;
use std::str::FromStr;

use deb822_lossless::Deb822;

/// The VCS-metadata block from `debian/control`.
///
/// Three `Option<String>` fields, all `None` when the
/// corresponding control field is absent or empty.
/// The 1C.3 `UpstreamRelationship` struct carries these
/// three alongside the three `UpstreamMetadata` fields
/// (from `debian/copyright`) — see
/// `maintenance_context::run` for the merge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VcsMetadata {
    /// The `Vcs-Git:` field. `None` when absent.
    pub vcs_git: Option<String>,
    /// The `Vcs-Browser:` field. `None` when
    /// absent.
    pub vcs_browser: Option<String>,
    /// The `Homepage:` field. `None` when absent.
    pub homepage: Option<String>,
}

/// Read the VCS-metadata block from `debian/control`
/// at `<debian_dir>/control`.
///
/// `None` everywhere when the file is absent, empty,
/// or unparsable.
#[must_use]
pub fn read_vcs_metadata(debian_dir: &Path) -> VcsMetadata {
    let path = debian_dir.join("control");
    let Ok(body) = std::fs::read_to_string(&path) else {
        return VcsMetadata::default();
    };
    parse_vcs_metadata(&body)
}

fn parse_vcs_metadata(body: &str) -> VcsMetadata {
    let Ok(parsed) = Deb822::from_str(body) else {
        return VcsMetadata::default();
    };
    let first = match parsed.paragraphs().next() {
        Some(p) => p,
        None => return VcsMetadata::default(),
    };
    VcsMetadata {
        vcs_git: opt_field(&first, "Vcs-Git"),
        vcs_browser: opt_field(&first, "Vcs-Browser"),
        homepage: opt_field(&first, "Homepage"),
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
        let md = parse_vcs_metadata("");
        assert!(md.vcs_git.is_none());
        assert!(md.vcs_browser.is_none());
        assert!(md.homepage.is_none());
    }

    #[test]
    fn source_paragraph_vcs_fields() {
        let body = "\
Source: example
Maintainer: IronMaint Test <test@example.invalid>
Section: utils
Homepage: https://example.invalid/example
Vcs-Git: https://example.invalid/example.git
Vcs-Browser: https://example.invalid/cgit/example.git

Package: example
Architecture: any
Description: trivial example package
";
        let md = parse_vcs_metadata(body);
        assert_eq!(
            md.homepage.as_deref(),
            Some("https://example.invalid/example")
        );
        assert_eq!(
            md.vcs_git.as_deref(),
            Some("https://example.invalid/example.git")
        );
        assert_eq!(
            md.vcs_browser.as_deref(),
            Some("https://example.invalid/cgit/example.git")
        );
    }

    #[test]
    fn binary_paragraph_does_not_contribute() {
        // The binary paragraph has no Vcs- fields; the
        // parser only reads the source (first) paragraph.
        let body = "\
Source: example

Package: example
Architecture: any
Vcs-Git: should-be-ignored
";
        let md = parse_vcs_metadata(body);
        assert!(md.vcs_git.is_none());
    }

    #[test]
    fn missing_control_yields_default() {
        let md = read_vcs_metadata(Path::new("/nonexistent/debian"));
        assert!(md.vcs_git.is_none());
        assert!(md.vcs_browser.is_none());
        assert!(md.homepage.is_none());
    }
}
