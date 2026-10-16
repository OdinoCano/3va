#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (c) 3va contributors
#
# Release-gate scenario: run the client app with the released 3va binary inside
# debian:bookworm-slim against postgres 16 (SCRAM), a mock upstream and nginx,
# then drive the whole flow with a node client. Fails if any check fails.
# See tests/client-scenario/README.md and docs/09-testing/07-compatibilidad-node.md
# (§7.4 step 5, §7.5).
#
# Usage: THREEVA_BIN=/path/to/3va scripts/client-scenario.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCEN="$ROOT/tests/client-scenario"

if [ -z "${THREEVA_BIN:-}" ]; then
  if [ -x "$ROOT/target/release/3va" ]; then THREEVA_BIN="$ROOT/target/release/3va"; else THREEVA_BIN=3va; fi
fi
if [ ! -x "$THREEVA_BIN" ] && ! command -v "$THREEVA_BIN" >/dev/null 2>&1; then
  echo "3va binary not found: $THREEVA_BIN" >&2
  exit 2
fi
if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for the client scenario" >&2
  exit 2
fi

# The image build copies this file into /usr/local/bin/3va.
cp "$THREEVA_BIN" "$SCEN/3va"
chmod +x "$SCEN/3va"

cd "$SCEN"
cleanup() {
  docker compose -f docker-compose.yml down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "Client scenario with $("$THREEVA_BIN" --version 2>/dev/null | head -1)"
set +e
docker compose -f docker-compose.yml up --build --abort-on-container-exit --exit-code-from client
status=$?
set -e

if [ "$status" -ne 0 ]; then
  echo "── client-scenario failed (exit $status); app and nginx logs: ──"
  docker compose -f docker-compose.yml logs --no-color --tail 80 app nginx client 2>/dev/null || true
fi
exit "$status"
