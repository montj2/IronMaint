//! Shared parser helpers for `ironmaint-debian-tool`.
//!
//! The 1C.2 `source_analysis::run` parser composition walks
//! `debian/control`, `debian/copyright`, `debian/watch`,
//! `debian/patches/series` + DEP-3 patch headers,
//! `debian/tests/control`, `debian/rules`, the four
//! maintainer scripts, and `debian/gbp.conf`.
//!
//! The 1C.3 `maintenance_context::run` re-uses those
//! helpers for the §26 "patches / tests / watch / GBP /
//! maintainer scripts" group, and adds two new helpers
//! (in this module) for the §26 "upstream relationship
//! metadata" group:
//!
//! - `copyright::read_upstream_metadata(debian_dir) ->
//!   UpstreamMetadata` — reads `debian/copyright`'s
//!   `Upstream-Name`, `Upstream-Contact`, and `Source`
//!   fields.
//! - `control::read_vcs_metadata(debian_dir) ->
//!   VcsMetadata` — reads `debian/control`'s
//!   `Homepage`, `Vcs-Git`, `Vcs-Browser` fields.
//!
//! The 1C.2 helpers stay in `source_analysis.rs` for
//! now; the 1C.3 PR's only structural change to the
//! source tree is this `parsers/` module. Future
//! extraction (1C.4+, 1D.x) is a follow-up that
//! follows the same pattern.

pub mod control;
pub mod copyright;
