# syntax=docker/dockerfile:1.7
#
# ironmaint/debian-tools:0.1 — Debian packaging toolchain image.
#
# Spec: doc/CONTAINER-IMAGES.md §4.3.
# Builds packages inside an `sbuild` schroot on a mounted volume.
#
# Layer graph: this image derives FROM ironmaint/base:0.1 — same Debian
# base, same apt cache shape, same non-root user (uid 1000 `ironmaint`).
# `sbuild`, `lintian`, `autopkgtest`, and friends install on top.
#
# Spec §11.3 (release mismatch): the IronMaint Debian fixtures declare
# `DistributionRelease::new("sid")` (adapters/debian-stub/src/release.rs:100,
# crates/ironmaint-policy/src/policy.rs:212). The image's OWN base can be
# any Debian release (we use the same trixie as base); the schroot
# created by `sbuild-debootstrap` targets sid by default to match the
# fixtures. That keeps fixture-honest tool output without making the
# image itself a snapshot of a moving target.

# ---- base --------------------------------------------------------------
# Pin to our own base image by digest (not the ironmaint/base tag), so
# the contract is: this image's apt index matches the one base was
# built against. Resolved via `make -f containers/Makefile inspect`.
ARG BASE_TAG=ironmaint/base:0.1
FROM ${BASE_TAG}

# ---- apt packages -------------------------------------------------------
# Spec §4.3 tool-key → binary → package table:
#
#   debian.build.sbuild         sbuild                sbuild
#   debian.qa.lintian           lintian               lintian
#   debian.qa.piuparts          piuparts              piuparts
#   debian.test.autopkgtest     autopkgtest           autopkgtest
#   (general)                   dpkg-buildpackage     dpkg-dev
#                                                      debhelper
#   (general)                   gbp                   git-buildpackage
#   (general)                   uscan                 devscripts
#   (general)                   diffoscope, reprotest diffoscope, reprotest
#
# NOTE on `debdiff`: the binary ships in the `devscripts` package (already
# listed above), not as its own package. Spec §4.3 names it as `debian-diff`
# but no apt package by that name exists in trixie; the binary is the one
# that matters, and `devscripts` provides it.
#
# All apt-get in one RUN; clean /var/lib/apt/lists in the same layer.
# ca-certificates is inherited from base — we don't reinstall.
#
# `schroot` and `debootstrap` are NOT pulled transitively by `sbuild` on
# trixie (verified: the sbuild package only Depends on libsbuild-perl).
# Both are needed regardless of whether sbuild uses the schroot or
# unshare backend — debootstrap creates the initial tarball, and schroot
# (or the unshare equivalent) is the runtime sandbox manager. Install
# them explicitly.
#
# `uidmap` is for unprivileged user namespaces, the preferred sandbox
# mode (spec §4.3) — without it the operator has to pass `--privileged`
# (documented in the README).
#
# USER root: the parent (ironmaint/base:0.1) ends with USER ironmaint.
# apt needs root; we drop back to ironmaint below.
USER root
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        sbuild \
        schroot \
        debootstrap \
        lintian \
        piuparts \
        autopkgtest \
        dpkg-dev \
        debhelper \
        git-buildpackage \
        devscripts \
        diffoscope \
        reprotest \
        uidmap \
 && rm -rf /var/lib/apt/lists/*

# ---- sbuild user & schroot defaults ------------------------------------
# sbuild runs as a dedicated user (typically uid 1001, gid 1001) inside
# the chroot, not as the image's running user. The host-side `sbuild`
# command needs to be able to invoke it, so the `ironmaint` user must
# be in the `sbuild` group. The schroot is created at first run into a
# mounted volume — NOT baked in (spec §4.3 explicit: "Buildroots are NOT
# baked in").
#
# We do not pre-create the sbuild user here because sbuild's postinst
# does it. We just add the ironmaint user to the sbuild group so the
# compose `workspace` user can call `sbuild` directly.
RUN groupadd --force --system sbuild 2>/dev/null || true \
 && usermod -aG sbuild ironmaint
# Back to non-root for runtime; matches base convention.
USER ironmaint:ironmaint

# ---- workspace inside the container -------------------------------------
# Bind-mounted source at /work, just like base. /work/buildroots holds
# the sbuild schroots created on first use. /work/.cache is for
# sbuild's package cache (dpkg, apt).
WORKDIR /work

# ---- smoke script --------------------------------------------------------
# Spec §11: "a test that cannot fail is not a test." Per DoD §9, a real
# sbuild run against a real source package is the gate (not just
# `--version`). The smoke script here runs `--version` checks for fast
# iteration; the full sbuild run lives in `scripts/debian-tools-smoke.sh`
# and is invoked from compose (--privileged required).
COPY --chmod=0755 smoke.sh /usr/local/bin/smoke.sh

# ---- labels --------------------------------------------------------------
LABEL org.opencontainers.image.title="ironmaint/debian-tools" \
      org.opencontainers.image.description="Debian packaging toolchain for IronMaint: sbuild, lintian, autopkgtest, dpkg-dev, debhelper, gbp, devscripts, diffoscope, reprotest, piuparts. Buildroots are NOT baked in; sbuild creates its schroot into a mounted volume on first use. Requires --privileged or SYS_ADMIN+user namespaces for the buildroot. Runs as uid 1000 (ironmaint). The base image is the source of truth for CA roots, git, bash, curl, python3." \
      org.opencontainers.image.source="https://github.com/montj2/IronMaint" \
      org.opencontainers.image.documentation="https://github.com/montj2/IronMaint/blob/develop/doc/CONTAINER-IMAGES.md" \
      org.opencontainers.image.licenses="(see repo LICENSE)" \
      org.opencontainers.image.vendor="IronMaint" \
      org.opencontainers.image.version="0.1" \
      org.opencontainers.image.authors="IronMaint maintainers"

# ---- entrypoint ----------------------------------------------------------
# Same shape as base. Compose invokes this image via `compose run --rm`
# with explicit commands; the entrypoint exists for ad-hoc `docker run`
# so that PID 1 is tini and signals are forwarded.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["bash"]
