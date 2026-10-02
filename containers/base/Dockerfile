# syntax=docker/dockerfile:1.7
#
# ironmaint/base:0.1 — substrate image.
#
# Skeleton scaffolded in commit 1; apt packages, non-root user, and smoke
# script COPY land in commit 3.

# Manifest list digest from docker buildx imagetools inspect debian:trixie-slim
# on 2026-10-01. Re-pin via:
#   docker buildx imagetools inspect debian:trixie-slim --raw \
#     | jq -r '.manifests[] | select(.platform.os=="linux" and .platform.architecture=="amd64").digest'
# (and analogously for arm64). The manifest list digest pins the index;
# per-platform digests would pin individual images but aren't supported
# in a single FROM line for multi-arch builds.
FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
