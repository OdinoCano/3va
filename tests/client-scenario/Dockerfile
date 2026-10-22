# SPDX-License-Identifier: MIT
# Copyright (c) 3va contributors
#
# Release-gate image: the released 3va binary on the oldest supported glibc
# (Debian 12 / bookworm-slim, glibc 2.36), running the client app. `nodejs`/
# `npm` are only used to install the pinned dependencies at build time; the app
# itself runs under 3va.
#
# BASE_IMAGE can be overridden (build arg) to run a binary that needs a newer
# glibc, e.g. the published 2.12.0 binaries, which were built on glibc 2.39 and
# predate the release workflow's glibc-floor check.
ARG BASE_IMAGE=debian:bookworm-slim
FROM ${BASE_IMAGE}

RUN apt-get update \
    && apt-get install -y --no-install-recommends nodejs npm ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY app/package.json app/package-lock.json ./
RUN npm ci --omit=dev --no-audit --no-fund

COPY app/ ./
COPY 3va /usr/local/bin/3va
RUN chmod +x /usr/local/bin/3va

ENV NODE_ENV=production
