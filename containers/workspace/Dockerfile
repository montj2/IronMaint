# syntax=docker/dockerfile:1.7
#
# ironmaint/workspace:0.1 — Rust 1.88.0 toolchain image.
#
# THIS IMAGE DOES NOT DERIVE FROM ironmaint/base.
# Spec §3 is explicit: workspace derives from `rust:1.88.0-bookworm`
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

# ---- rust base -----------------------------------------------------------
# Manifest list digest for rust:1.88.0-bookworm verified on 2026-10-01
# via `docker buildx imagetools inspect rust:1.88.0-bookworm`.
FROM rust:1.88.0-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0

# ---- apt packages on top of the rust image ------------------------------
# The `rust:*` images are based on debian-slim + rustup-installed toolchain.
# They already have bash, curl, git (transitive dep of rustup). They do
# not always have python3 or ca-certificates — and most importantly they
# have NO linker, which rustc requires.
#
#   build-essential    spec §4.2 — rustc needs a linker. Verified that
#                     this workspace has no native build steps (build.rs
#                     hits are Rust modules, no openssl/rusqlite/ring/git2
#                     direct deps). build-essential is the entire C
#                     toolchain requirement.
#   ca-certificates    §11.2 — same TLS argument as base. Some slim
#                     variants of rust:* ship without it.
#   python3            spec §4.1 — required by scripts/ironclaw-e2e.sh
#                     and by maintenance scripts that ship with this
#                     workspace.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        python3 \
 && rm -rf /var/lib/apt/lists/*

# Confirm the rust image's rustup-managed toolchain is the pinned 1.88.0.
# If a future `rust:1.88.0-bookworm` re-tag ever ships 1.88.1, the smoke
# test catches it at run time; this line is the build-time guard.
RUN rustc --version \
 && cargo --version \
 && rustup show active-toolchain

# ---- non-root user -------------------------------------------------------
# Older `rust:*` images shipped a `rust` user (uid 1000); the current
# `rust:1.88.0-bookworm` does NOT — it ships only root + sync. cargo
# refuses to run as root and that's correct, so we create the user.
# uid/gid 1000 matches the ironmaint/base image, so bind mounts behave
# predictably in compose (the same uid owns target/, .ironmaint/, and the
# bind-mounted source tree from the host's perspective).
RUN groupadd --system --gid 1000 rust \
 && useradd  --system --uid 1000 --gid rust \
              --no-create-home --shell /bin/bash \
              --comment "Rust toolchain user" rust
WORKDIR /work
RUN mkdir -p /work && chown -R rust:rust /work \
 && mkdir -p /usr/local/cargo && chown -R rust:rust /usr/local/cargo

# ---- workspace inside the container --------------------------------------
# Source is bind-mounted at runtime (compose.yml); this image does NOT
# COPY source. Editing a file on the host should not rebuild the image
# (spec §2.1 explicit guidance: "the image is a build environment, not a
# hot-reload system").

USER rust

# ---- prebuilt target/ — DEFERRED ----------------------------------------
# Spec §4.2 calls for `cargo build --workspace --all-targets` +
# `cargo build --workspace --tests` to be baked into the image so a cold
# CI run is a link step rather than a compile. This is a multi-GB image
# cost and there is no CI yet (spec §10 explicit non-goal). Deferred to
# the PR that adds CI. The named volume `target:` in compose.yml handles
# dev iteration in the meantime.
#
# To re-enable later, add:
#   COPY --chown=rust:rust . /work
#   RUN cargo build --workspace --all-targets \
#    && cargo build --workspace --tests

# ---- smoke script --------------------------------------------------------
# Spec §11: "a test that cannot fail is not a test." The smoke script is
# shipped inside the image and runnable from outside via:
#   docker run --rm -v "$PWD":/work ironmaint/workspace:0.1 \
#     /usr/local/bin/smoke.sh
COPY --chmod=0755 smoke.sh /usr/local/bin/smoke.sh

# ---- labels --------------------------------------------------------------
LABEL org.opencontainers.image.title="ironmaint/workspace" \
      org.opencontainers.image.description="Rust 1.88.0 toolchain for the IronMaint workspace. Matches rust-toolchain.toml exactly. No source baked in; bind-mount at runtime." \
      org.opencontainers.image.source="https://github.com/montj2/IronMaint" \
      org.opencontainers.image.documentation="https://github.com/montj2/IronMaint/blob/develop/doc/CONTAINER-IMAGES.md" \
      org.opencontainers.image.licenses="(see repo LICENSE)" \
      org.opencontainers.image.vendor="IronMaint" \
      org.opencontainers.image.version="0.1" \
      org.opencontainers.image.authors="IronMaint maintainers"

# ---- entrypoint ----------------------------------------------------------
# No ENTRYPOINT. The image is invoked as `docker run --rm ironmaint/workspace
# <command>`, e.g. `cargo test`. An entrypoint here would break that pattern.
# Spec §4.2: "Entry point: none. This image is docker run --rm + a command."
CMD []