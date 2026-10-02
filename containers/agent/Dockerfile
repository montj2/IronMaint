# syntax=docker/dockerfile:1.7
#
# ironmaint/agent:0.1 — IronClaw LLM agent runtime.
#
# Spec: doc/CONTAINER-IMAGES.md §4.5.
# Talks to ironmaintd over HTTP; closes §102 item 30.
#
# Layer graph: derives FROM ironmaint/base:0.1 (same Debian base,
# same uid 1000 ironmaint, same tini). The Rust toolchain is NOT
# needed — ironclaw ships as a prebuilt tarball on GitHub Releases
# (https://github.com/nearai/ironclaw/releases/tag/ironclaw-v1.4.1).
# We fetch the matching arch tarball, sha256-check, and COPY the
# binary into the image. End result: ~80MB instead of the
# submodule's ~1.5GB Railway-deployment image, no compilation, no
# Node toolchain, and SKIP_FRONTEND_BUILD is irrelevant because
# nothing here builds the WebUI at all.
#
# The submodule's published Docker image (Docker Hub) is amd64-only;
# using the release tarball gives us a real native arm64 build
# without the cold-build wait.

# ---- base ---------------------------------------------------------------
# ironmaint/base provides: uid 1000 ironmaint, tini, ca-certificates,
# curl, git, bash, python3. All of §4.5's runtime prerequisites.
FROM ironmaint/base:0.1

# ---- build-time args ----------------------------------------------------
# Pin the ironclaw release we are pulling. Bumping this is one
# change, but two follow-ups: update the SHA256 lines below, then
# verify the upstream tarball actually downloads and checks out (smoke
# does the verify at build time). Spec §11.1.
ARG IRONCLAW_RELEASE=ironclaw-v1.4.1

# TARGETARCH is set by buildx automatically (arm64 / amd64).
# Pick the glibc tarball: matches debian:trixie-slim's glibc. The
# MUSL build is also an option but pulls musl-specific runtime
# libraries we don't otherwise need.
ARG TARGETARCH

# ---- ironclaw binary ------------------------------------------------------
# Two layout-shapes (GNU vs MUSL) and two arches (aarch64 vs x86_64).
# Each has its own SHA256 from the matching .sha256-
# file alongside the tarball on the release. SHA256s are pinned per
# spec §11.1; an attacker who can forge the tarball cannot forge the
# release page that publishes the digest.

# 30acf81b5b8ec71e34547a27a43ef0bbc4bae8495745fee2e11ce3067718743b
#   *ironclaw-aarch64-unknown-linux-gnu.tar.gz
# 72b893d3e8f5f5a431cfaf9bc48e9dae71755e2d049ab4119c0646d352da4630
#   *ironclaw-x86_64-unknown-linux-gnu.tar.gz
#
# Base sets USER ironmaint. We need root for install paths, AND for
# creating /var/lib/ironclaw (root-owned by default on Debian), so this
# RUN does both under USER root and chowns the volume dir to ironmaint
# before dropping back. Same USER root/USER pattern as
# containers/debian-tools/Dockerfile.
USER root
RUN set -eux; \
    case "${TARGETARCH}" in \
        arm64) \
            SUFFIX=aarch64-unknown-linux-gnu; \
            SHA256=30acf81b5b8ec71e34547a27a43ef0bbc4bae8495745fee2e11ce3067718743b; \
            ;; \
        amd64) \
            SUFFIX=x86_64-unknown-linux-gnu; \
            SHA256=72b893d3e8f5f5a431cfaf9bc48e9dae71755e2d049ab4119c0646d352da4630; \
            ;; \
        *) \
            echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; \
            exit 1; \
            ;; \
    esac; \
    TARBALL="ironclaw-${SUFFIX}.tar.gz"; \
    URL="https://github.com/nearai/ironclaw/releases/download/${IRONCLAW_RELEASE}/${TARBALL}"; \
    curl -fsSL --retry 3 --retry-delay 5 -o "/tmp/${TARBALL}" "${URL}"; \
    echo "${SHA256}  /tmp/${TARBALL}" | sha256sum -c -; \
    mkdir -p /tmp/ironclaw-extract; \
    tar -xzf "/tmp/${TARBALL}" -C /tmp/ironclaw-extract; \
    install -m 0755 /tmp/ironclaw-extract/ironclaw-${SUFFIX}/ironclaw /usr/local/bin/ironclaw; \
    rm -rf /tmp/ironclaw-extract "/tmp/${TARBALL}"; \
    ironclaw --version; \
    mkdir -p /var/lib/ironclaw; \
    chown ironmaint:ironmaint /var/lib/ironclaw

USER ironmaint:ironmaint

# ---- state directory ----------------------------------------------------
# /var/lib/ironclaw is the agent's home for state, secrets cache,
# embedding DB, etc. Marking it VOLUME means compose will surface it
# as a named volume by default; a developer running `docker run`
# without -v gets a fresh state on each start (acceptable for the
# smoke gate, which only does `--version`). Created and chowned in the
# install RUN above (USER root needed for both).
VOLUME ["/var/lib/ironclaw"]

# ---- default env vars (non-secret contract) -----------------------------
# Secrets are intentionally NOT set here:
#   - MINIMAX_API_KEY: must be injected at run time
#   - IRONCLAW_REBORN_SECRET_MASTER_KEY: must be injected at run time
# Defaults for the public side of the contract are baked so a
# developer can `docker run` with `-e MINIMAX_API_KEY=…` and have
# everything else match spec §4.5.
ENV LLM_BACKEND=minimax \
    MINIMAX_MODEL=MiniMax-M3.1-Flash-Preview \
    MINIMAX_BASE_URL=https://api.minimax.io/v1 \
    IRONCLAW_REBORN_SERVE_HOST=0.0.0.0 \
    IRONCLAW_REBORN_HOME=/var/lib/ironclaw

# ---- labels -------------------------------------------------------------
LABEL org.opencontainers.image.title="ironmaint/agent" \
      org.opencontainers.image.description="IronMaint LLM agent runtime: ironclaw 1.4.1 (prebuilt release tarball, glibc) running under tini as uid 1000 (ironmaint). Talks to ironmaintd over HTTP. Secrets (MINIMAX_API_KEY, IRONCLAW_REBORN_SECRET_MASTER_KEY) are NOT baked — injected via -e or compose env_file. /var/lib/ironclaw is a volume for state persistence." \
      org.opencontainers.image.source="https://github.com/montj2/IronMaint" \
      org.opencontainers.image.documentation="https://github.com/montj2/IronMaint/blob/develop/doc/CONTAINER-IMAGES.md" \
      org.opencontainers.image.licenses="(see repo LICENSE)" \
      org.opencontainers.image.vendor="IronMaint" \
      org.opencontainers.image.version="0.1" \
      org.opencontainers.image.authors="IronMaint maintainers" \
      ironclaw.release="ironclaw-v1.4.1"

# ---- smoke script -------------------------------------------------------
# Spec §11: a test that cannot fail is not a test. The smoke verifies
# the binary is present, the version pin holds (1.4.1, exactly), the
# required env vars are wired in (with secrets correctly refused),
# tini is PID 1, and the volume is writable. NOT a full
# `ironclaw serve` smoke: that needs a real postgres sidecar (spec
# §4.5), and the build smoke is intentionally just here.
COPY --chmod=0755 smoke.sh /usr/local/bin/smoke.sh

# ---- entrypoint ---------------------------------------------------------
# Exec form, tini as PID 1. spec §4.5 last paragraph: tini forwards
# SIGTERM to ironclaw serve, which is what makes the acceptance run's
# `docker stop` actually drain.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["ironclaw", "--help"]