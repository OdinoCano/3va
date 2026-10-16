#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (c) 3va contributors
#
# Generate the measured Node.js compatibility table from a nodejs/node
# `test/parallel` run. The counts (per module: passed, failed, skipped) come from
# the runner's NODE_TESTS_COUNTS_OUT (crates/test/tests/node_tests.rs); the
# pinned nodejs/node commit comes from tests/node/COMMIT.
#
# Usage: scripts/compat-table.sh <counts.tsv> [--title]
#   counts.tsv: lines of "<module>\t<passed>\t<failed>\t<skipped>"
#
# Compatibility % = passed / (passed + failed). Skipped tests are reported
# separately and are not part of the percentage (see
# docs/09-testing/07-compatibilidad-node.md §7.4).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

COUNTS="${1:?usage: scripts/compat-table.sh <counts.tsv>}"
REF="$(cat tests/node/REF 2>/dev/null || echo 'unknown')"
COMMIT="$(cat tests/node/COMMIT 2>/dev/null | cut -c1-12 || echo 'unknown')"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/cli/Cargo.toml | head -1)"
DATE="$(date -u +%Y-%m-%d)"

echo "| Module | Passed | Failed | Skipped | Compatibility |"
echo "|---|---:|---:|---:|---:|"

total_p=0; total_f=0; total_s=0
# Sort by module name for a stable table.
while IFS=$'\t' read -r module passed failed skipped; do
    [ -n "$module" ] || continue
    total_p=$((total_p + passed)); total_f=$((total_f + failed)); total_s=$((total_s + skipped))
    denom=$((passed + failed))
    pct=$(awk -v p="$passed" -v d="$denom" 'BEGIN { if (d==0) printf "0.0"; else printf "%.1f", p*100/d }')
    echo "| \`$module\` | $passed | $failed | $skipped | ${pct}% |"
done < <(sort "$COUNTS")

denom=$((total_p + total_f))
pct=$(awk -v p="$total_p" -v d="$denom" 'BEGIN { if (d==0) printf "0.0"; else printf "%.1f", p*100/d }')
echo "| **total** | **$total_p** | **$total_f** | **$total_s** | **${pct}%** |"

if [ "${2:-}" = "--title" ]; then
    echo
    echo "_Measured on 3va ${VERSION} against nodejs/node ${REF} (${COMMIT}) on ${DATE}. \
Compatibility = passed / (passed + failed); skipped tests (unsupported flags or Node \
internals) are listed separately and excluded. Regenerate with \
\`scripts/setup-node-tests.sh\` + the node_tests runner + \`scripts/compat-table.sh\`._"
fi
