//! On-disk layout for the artifact store.
//!
//! Layout under an `ArtifactRoot`:
//!
//! ```text
//! <root>/                (the state-dir subdirectory used for blobs)
//! ├── staging/           (tempfile staging during writes; GC on next start)
//! └── aa/bb/<full-hex>   (the canonical content-addressed path)
//! ```
//!
//! `aa` and `bb` are the first two hex-byte pairs of the
//! digest — classic two-level sharding so a single directory
//! never holds more than ~65k entries per parent.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ArtifactRoot(PathBuf);

impl ArtifactRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self(root.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn staging(&self) -> PathBuf {
        self.0.join("staging")
    }

    /// Final path for a given digest: `<root>/<aa>/<bb>/<digest>`.
    pub fn final_path_for(&self, digest_hex: &str) -> PathBuf {
        let (aa, bb) = digest_split(digest_hex);
        self.0.join(&aa).join(&bb).join(digest_hex)
    }
}

fn digest_split(digest_hex: &str) -> (String, String) {
    let chars: Vec<char> = digest_hex.chars().collect();
    let aa: String = chars.iter().take(2).collect();
    let bb: String = chars.iter().take(4).collect::<String>()[2..].to_string();
    (aa, bb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharding_is_first_two_byte_pairs() {
        let root = ArtifactRoot::new("/tmp/artifacts");
        let digest = "deadbeef00000000000000000000000000000000000000000000000000000000";
        let path = root.final_path_for(digest);
        assert_eq!(
            path,
            PathBuf::from(
                "/tmp/artifacts/de/ad/deadbeef00000000000000000000000000000000000000000000000000000000"
            )
        );
    }
}
