#!/usr/bin/env bash
# docker-publish.sh — Build and push the multi-arch runtime image to Docker Hub.
# Run AFTER the GitHub release for the version is published: the image
# downloads the release binaries. Requires a prior `docker login`.
#
# Usage:
#   ./scripts/docker-publish.sh               # version from Dockerfile
#   DOCKER_REPO=me/3va ./scripts/docker-publish.sh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="${DOCKER_REPO:-edge166/3va}"
VERSION=$(sed -n 's/^ARG RUNTIME_VERSION=//p' "$ROOT_DIR/Dockerfile")

[[ -n "$VERSION" ]] || { echo "Error: RUNTIME_VERSION not found in Dockerfile" >&2; exit 1; }

# The default `docker` driver can't push multi-platform images.
# Note: buildx rejects builder names that start with a digit, so "3va-multi"
# (which starts with "3") is renamed to "va3-multi".
BUILDER=va3-multi
docker buildx inspect "$BUILDER" >/dev/null 2>&1 \
    || docker buildx create --name "$BUILDER" --driver docker-container >/dev/null

echo "Publishing $REPO:$VERSION and $REPO:latest (linux/amd64, linux/arm64)"
docker buildx build --builder "$BUILDER" \
    --platform linux/amd64,linux/arm64 \
    -t "$REPO:$VERSION" -t "$REPO:latest" \
    --push "$ROOT_DIR"
