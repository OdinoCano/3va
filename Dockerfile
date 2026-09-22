# 3va — secure JavaScript/TypeScript runtime, deny-by-default.
# Generic runtime image: no application code baked in, just the `3va` binary.
#
#   docker run --rm -v "$PWD":/app ghcr.io/odinocano/3va run app.ts --allow-net=api.example.com
#
# syntax=docker/dockerfile:1

FROM debian:trixie-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl tar \
  && rm -rf /var/lib/apt/lists/*

ARG RUNTIME_VERSION=v2.8.0
ARG TARGETARCH

RUN case "${TARGETARCH}" in \
      amd64) TRIPLE=x86_64-unknown-linux-gnu ;; \
      arm64) TRIPLE=aarch64-unknown-linux-gnu ;; \
      *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
  && curl -fsSL "https://github.com/OdinoCano/3va/releases/download/${RUNTIME_VERSION}/3va-${RUNTIME_VERSION}-${TRIPLE}.tar.gz" -o /tmp/3va.tar.gz \
  && tar -xzf /tmp/3va.tar.gz -C /usr/local/bin \
  && rm /tmp/3va.tar.gz \
  && chmod +x /usr/local/bin/3va \
  && /usr/local/bin/3va --version

RUN groupadd --gid 10001 3va \
  && useradd --uid 10001 --gid 10001 --home-dir /app --no-log-init --no-create-home 3va
WORKDIR /app
USER 3va

ENTRYPOINT ["3va"]
CMD ["--help"]
