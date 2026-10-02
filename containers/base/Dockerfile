# syntax=docker/dockerfile:1.7
#
# ironmaint/base:0.1 — substrate image.
#
# The packages here are the common floor for every other image in the set.
# Do not add tools here that belong to one distribution (those go in
# debian-tools / fedora-tools). Do not add a Rust toolchain here
# (workspace derives from rust:1.94.0-bookworm directly; see
# doc/CONTAINER-IMAGES.md §3).
#
# Spec: doc/CONTAINER-IMAGES.md §4.1.

# ---- base layer ----------------------------------------------------------
# Manifest list digest from docker buildx imagetools inspect debian:trixie-slim
# on 2026-10-01. Re-pin via:
#   docker buildx imagetools inspect debian:trixie-slim --raw \
#     | jq -r '.manifests[] | select(.platform.os=="linux" and .platform.architecture=="amd64").digest'
# (and analogously for arm64). The manifest list digest pins the index;
# per-platform digests would pin individual images but aren't supported
# in a single FROM line for multi-arch builds.
FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a

# ---- apt packages --------------------------------------------------------
# Single RUN keeps the layer count down. Clean /var/lib/apt/lists/* in the
# same layer so the final image doesn't carry the index (~30 MB).
#
# Why each package:
#   ca-certificates    §11.2 — without this, rustls' native-roots TLS backend
#                     silently fails; looks like "the network is broken".
#                     This is the single most likely thing to be missed.
#   tini               §5.3 — every long-running container in this set
#                     needs PID 1 to forward SIGTERM/SIGINT. ironmaintd
#                     installs both handlers (verified: main.rs:369-401).
#   bash               spec §4.1; also a dependency of many scripts.
#   curl               spec §4.1; used by smoke test to prove TLS works.
#   git                spec §4.1; cargo resolves git-sourced deps; spec §24
#                     makes git a first-class runtime dep.
#   python3            spec §4.1; scripts/ironclaw-e2e.sh's JSON helper is
#                     `python3 -c` at lines 59-62.
#
# Deliberately NOT included (spec §4.1):
#   - no compiler (workspace handles Rust; debian-tools/fedora-tools handle C)
#   - no sqlite3 CLI (the store is reached through sqlx, never through a shell)
#   - no jq (e2e script doesn't need it; verified)
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        tini \
        bash \
        curl \
        git \
        python3 \
 && rm -rf /var/lib/apt/lists/*

# ---- non-root user -------------------------------------------------------
# Spec §4.1: uid 1000 matches the convention used by the upstream
# `rust:*` images (where the user is named `rust`); keeping the same uid
# across images means bind mounts behave predictably in compose.
# No password, no home dir lock-in: home is created at USER switch.
# The shell must be bash, not sh, because entrypoint scripts use bashisms.
RUN groupadd --system --gid 1000 ironmaint \
 && useradd  --system --uid 1000 --gid ironmaint \
              --no-create-home --shell /bin/bash \
              --comment "IronMaint non-root user" ironmaint

# ---- workspace inside the container --------------------------------------
# /work is the bind-mount point compose.yml uses for the source tree.
# Owned by the non-root user so files written into .ironmaint/, target/
# (named volume) appear owned by uid 1000 on the host (acceptable; see
# containers/README.md §"macOS uid considerations").
WORKDIR /work
RUN mkdir -p /work && chown -R ironmaint:ironmaint /work

# ---- smoke script --------------------------------------------------------
# Spec §11: "a test that cannot fail is not a test." The smoke script is
# shipped inside the image and runnable from outside via:
#   docker run --rm --entrypoint /usr/local/bin/smoke.sh ironmaint/base:0.1
COPY --chmod=0755 smoke.sh /usr/local/bin/smoke.sh

# ---- labels (OCI image metadata) ----------------------------------------
# These show up in `docker inspect` and `docker buildx imagetools inspect`.
# Title and description are what container registries display.
LABEL org.opencontainers.image.title="ironmaint/base" \
      org.opencontainers.image.description="Substrate for IronMaint images: CA roots, tini, git, bash, curl, python3. No compiler, no sqlite CLI. Runs as uid 1000." \
      org.opencontainers.image.source="https://github.com/montj2/IronMaint" \
      org.opencontainers.image.documentation="https://github.com/montj2/IronMaint/blob/develop/doc/CONTAINER-IMAGES.md" \
      org.opencontainers.image.licenses="(see repo LICENSE)" \
      org.opencontainers.image.vendor="IronMaint" \
      org.opencontainers.image.version="0.1" \
      org.opencontainers.image.authors="IronMaint maintainers"

# ---- user + entrypoint ---------------------------------------------------
# Switch to non-root. Everything after this runs as uid 1000.
USER ironmaint:ironmaint

# tini as PID 1, exec form. Anything passed via `docker run … CMD` runs as
# a child of tini, so SIGTERM reaches the process and is forwarded.
# `--` ends tini's own options so the CMD can't be mistaken for one.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["bash"]