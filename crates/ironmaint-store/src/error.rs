//! Store error type — the single error that every sub-trait method returns.

use std::fmt;

use ironmaint_core::JobId;

/// The error type every sub-trait method returns.
///
/// Variants:
/// - [`StoreErrorKind::NotFound`] — the requested entity doesn't exist
/// - [`StoreErrorKind::Conflict`] — an optimistic-concurrency check failed
///   (stale `expected_version`, lost race on a sequence number, etc.)
/// - [`StoreErrorKind::SequenceOutOfRange`] — append-event violated monotonicity
/// - [`StoreErrorKind::Corrupt`] — on-disk state cannot be interpreted
///   (failed schema introspection, truncated row, etc.)
/// - [`StoreErrorKind::Backend`] — wrapped backend error (sqlx, io, etc.)
///
/// Each variant carries a structured detail string suitable for
/// logging. Backends SHOULD NOT include arbitrary user input in the
/// detail; PHASE-0B.md §65 forbids logging untrusted input in domain
/// crates, and the store sits one layer above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError {
    pub kind: StoreErrorKind,
    pub detail: String,
}

impl StoreError {
    #[must_use]
    pub fn new(kind: StoreErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    #[must_use]
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(StoreErrorKind::NotFound, what)
    }

    #[must_use]
    pub fn conflict(what: impl Into<String>) -> Self {
        Self::new(StoreErrorKind::Conflict, what)
    }

    #[must_use]
    pub fn sequence_out_of_range(job_id: JobId, expected: u64, got: u64) -> Self {
        Self::new(
            StoreErrorKind::SequenceOutOfRange,
            format!("job {job_id}: expected sequence {expected}, got {got}"),
        )
    }

    #[must_use]
    pub fn corrupt(what: impl Into<String>) -> Self {
        Self::new(StoreErrorKind::Corrupt, what)
    }

    #[must_use]
    pub fn backend(what: impl Into<String>) -> Self {
        Self::new(StoreErrorKind::Backend, what)
    }

    #[must_use]
    pub fn kind(&self) -> &StoreErrorKind {
        &self.kind
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreErrorKind {
    NotFound,
    Conflict,
    SequenceOutOfRange,
    Corrupt,
    Backend,
}

impl fmt::Display for StoreErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::SequenceOutOfRange => "sequence_out_of_range",
            Self::Corrupt => "corrupt",
            Self::Backend => "backend",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_formats_kind_then_detail() {
        let err = StoreError::not_found("projection for job 42");
        assert_eq!(err.kind(), &StoreErrorKind::NotFound);
        assert_eq!(err.detail, "projection for job 42");
        assert_eq!(format!("{err}"), "NotFound: projection for job 42");
    }

    #[test]
    fn sequence_out_of_range_includes_job_and_numbers() {
        let job = JobId::from_uuid(uuid::Uuid::nil());
        let err = StoreError::sequence_out_of_range(job, 5, 7);
        assert_eq!(err.kind(), &StoreErrorKind::SequenceOutOfRange);
        assert!(err.detail.contains("5"));
        assert!(err.detail.contains("7"));
    }

    #[test]
    fn conflict_constructor_sets_kind() {
        let err = StoreError::conflict("expected_version=3, found=5");
        assert_eq!(err.kind(), &StoreErrorKind::Conflict);
    }
}
