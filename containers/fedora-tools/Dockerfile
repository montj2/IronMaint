# syntax=docker/dockerfile:1.7
#
# ironmaint/fedora-tools:0.1 — Fedora packaging toolchain image.
#
# Spec: doc/CONTAINER-IMAGES.md §4.4.
# Builds RPMs inside a `mock` buildroot on a mounted volume.
#
# Layer graph: this image does NOT derive from ironmaint/base. Spec §3
# is explicit: "debian-tools and fedora-tools must not derive from
# each other. base distributions differ, Debian-shaped check runs on
# a Debian-shaped system." base is debian:trixie-slim; fedora-tools
# starts from fedora:44.
#
# §11.3 (release mismatch): the Fedora fixtures declare whatever
# `DistributionRelease` the stub declares (adapters/fedora-stub/).
# The image's OWN base can be any Fedora release (we use fedora:44);
# the buildroot created by `mock -r fedora-XX-aarch64 --init`
# targets the same release as the fixture. That keeps fixture-honest
# tool output without making the image itself a snapshot of a moving
# target.

# ---- base ---------------------------------------------------------------
# Manifest list digest for fedora:44 verified on 2026-10-01 via
# `docker buildx imagetools inspect fedora:44`. The spec was written
# against fedora:42 (which is the previous stable); the user
# explicitly corrected to fedora:44 as the current standard. Spec
# §11.1 applies: pinning the digest makes the FROM reproducible.
# Pinned to the MULTI-ARCH INDEX digest, not a per-platform manifest digest.
# The previous pin named a single amd64 manifest, so `--platform linux/arm64`
# resolved amd64 and the image inspected as arm64 while every binary was x86_64.
# This one matters twice over here: `mock` builds a buildroot for the machine it
# believes it is on, and a fedora-tools image carrying amd64 binaries would have
# told it to build for the wrong architecture. See D-23.
FROM fedora:44@sha256:43b29f65a41eb9c35e1cd5323e3bdf3b655c2357a9f4f1ff2f9c2798e5045d80

# ---- dnf packages --------------------------------------------------------
# Spec §4.4 tool-key → binary → package table:
#
#   fedora.build.mock         mock            mock
#   fedora.qa.rpmlint         rpmlint         rpmlint
#   fedora.build.spectool     spectool        rpm-build
#   fedora.test.tmt           tmt             tmt
#   (general)                 rpmbuild        rpm-build
#   (general)                 fedpkg          fedpkg
#   (general)                 koji            koji
#   (general)                 bodhi           bodhi-client
#   (general)                 packit          packit
#
# dnf in one RUN, clean dnf cache in the same layer. fedora:44 is a
# minimal image; we need `dnf` (not `microdnf`) for the group
# installs that mock pulls in.
#
# `tini` is NOT in fedora:44 minimal — we install it explicitly because
# spec §5.3 requires PID 1 to forward SIGTERM to long-running processes
# in this image (mock itself, for one). Same pattern as ironmaint/base.
#
# git, bash, curl, python3 come from fedora:44 base. We re-install
# ca-certificates anyway because some Fedora minimal images drop it;
# this matches the §11.2 argument in base.
RUN dnf install -y \
        tini \
        ca-certificates \
        mock \
        rpmlint \
        rpm-build \
        tmt \
        fedpkg \
        koji \
        bodhi-client \
        packit \
 && dnf clean all \
 && rm -rf /var/cache/dnf

# ---- mock group ----------------------------------------------------------
# mock requires the caller to be in the `mock` group, which mock's
# postinst creates. We don't pre-create the user because mock handles
# that, but we do need to make sure the runtime user exists and is in
# the group. fedora:44 ships with no non-root user (only root + the
# systemd-journal-remote-style system users), so we add `ironmaint`
# matching the Debian-side images' uid/gid convention.
RUN groupadd --force --system mock 2>/dev/null || true \
 && groupadd --system --gid 1000 ironmaint 2>/dev/null || true \
 && useradd  --system --uid 1000 --gid ironmaint \
              --no-create-home --shell /bin/bash \
              --comment "IronMaint non-root user" ironmaint 2>/dev/null || true \
 && usermod -aG mock ironmaint

# ---- workspace inside the container --------------------------------------
# Same shape as base: bind-mount source at /work. /var/lib/mock is
# the buildroot root (managed by mock at run time, NOT baked in —
# spec §4.4 explicit). Buildroots are heavy (multi-GB) so we mount
# them as a named volume in compose.
WORKDIR /work

# ---- smoke script --------------------------------------------------------
# Spec §11: "a test that cannot fail is not a test." Per DoD §9, the
# fedora-tools gate is `mock -r fedora-44-aarch64 --init` succeeds +
# a trivial RPM builds inside the chroot. The init smoke lives here;
# the trivial-build smoke lives in scripts/fedora-tools-build-smoke.sh
# and runs separately from compose with --privileged.
COPY --chmod=0755 smoke.sh /usr/local/bin/smoke.sh

# ---- labels --------------------------------------------------------------
LABEL org.opencontainers.image.title="ironmaint/fedora-tools" \
      org.opencontainers.image.description="Fedora packaging toolchain for IronMaint: mock, rpmlint, rpm-build, tmt, fedpkg, koji, bodhi (from bodhi-client), packit. Buildroots are NOT baked in; mock creates them into a mounted volume on first use. Requires --privileged for the buildroot. Runs as uid 1000 (ironmaint)." \
      org.opencontainers.image.source="https://github.com/montj2/IronMaint" \
      org.opencontainers.image.documentation="https://github.com/montj2/IronMaint/blob/develop/doc/CONTAINER-IMAGES.md" \
      org.opencontainers.image.licenses="(see repo LICENSE)" \
      org.opencontainers.image.vendor="IronMaint" \
      org.opencontainers.image.version="0.1" \
      org.opencontainers.image.authors="IronMaint maintainers"

# ---- entrypoint ----------------------------------------------------------
# Same shape as base: tini as PID 1, default to bash. Compose invokes
# this image via `compose run --rm` with explicit commands; the
# entrypoint exists for ad-hoc `docker run` so PID 1 is tini and
# signals are forwarded.
#
# USER ironmaint:ironmaint: fedora:44 ships root-only by default; we
# create ironmaint (uid 1000, gid 1000) above and switch here. Matches
# the convention in ironmaint/base:0.1 so bind mounts behave
# predictably in compose.
USER ironmaint:ironmaint
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["bash"]
