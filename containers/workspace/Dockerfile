# syntax=docker/dockerfile:1.7
#
# ironmaint/workspace:0.1 — Rust 1.88.0 toolchain image.
#
# THIS IMAGE DOES NOT DERIVE FROM ironmaint/base.
# Spec §4.2 is explicit: workspace derives from `rust:1.88.0-bookworm`
# directly, because the heavy Rust image shouldn't drag in the base
# image's apt cache, and because 1.88.0 + the apt packages listed in
# §4.1 are enough on their own. The future `agent` image (Rust 1.98.0)
# also derives from `rust:*` directly for the same reason.
#
# DO NOT CHANGE the FROM tag without changing rust-toolchain.toml and
# vice versa. The pin is the whole point of the image. Spec §11.1 calls
# this out as the single most likely thing to get wrong — "an image that
# resolves to a newer toolchain is worse than no image, because it
# converts an unenforced claim into an actively false one."
#
# Skeleton scaffolded in commit 1; apt packages and USER directive land
# in commit 4.

FROM rust:1.88.0-bookworm@sha256:PLACEHOLDER_PINNED_IN_COMMIT_4
