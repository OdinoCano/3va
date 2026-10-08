#!/usr/bin/env bash
# coverage.sh — real per-crate coverage with cargo-llvm-cov.
#
# Uses the same configuration as the CI "Coverage (cargo-llvm-cov)" job.
#
# Why not tarpaulin: run at the workspace root it selects every member and
# aborts on the first test binary that raises a signal (a SIGILL from the
# wasm/V8 tests), so it writes no report at all — which is why an earlier
# measurement read 0%. Run per crate it also counts vendored path deps
# (vendor/*) in the denominator, so its percentage is not the project's.
#
# Usage: scripts/coverage.sh
set -euo pipefail
cd "$(dirname "$0")/.."

export CARGO_PROFILE_DEV_DEBUG=line-tables-only
report="$(mktemp)"
trap 'rm -f "$report"' EXIT

cargo llvm-cov --workspace --summary-only --no-fail-fast | tee "$report"

awk '
  /^Filename/ { next }
  /^-+$/      { next }
  $1 == "TOTAL" { next }
  NF >= 10 && $1 ~ /\// {
    split($1, p, "/"); c = p[1]
    reg[c] += $2; mreg[c] += $3; ln[c] += $8; mln[c] += $9
    treg += $2; tmreg += $3; tln += $8; tmln += $9
    seen[c] = 1
  }
  END {
    printf "\n%-14s %8s %8s %8s %8s\n", "crate", "regions", "reg%", "lines", "line%"
    for (c in seen) {
      rp = reg[c] ? (reg[c] - mreg[c]) / reg[c] * 100 : 0
      lp = ln[c] ? (ln[c] - mln[c]) / ln[c] * 100 : 0
      printf "%-14s %8d %7.1f%% %8d %7.1f%%\n", c, reg[c], rp, ln[c], lp
    }
    trp = treg ? (treg - tmreg) / treg * 100 : 0
    tlp = tln ? (tln - tmln) / tln * 100 : 0
    printf "%-14s %8d %7.1f%% %8d %7.1f%%\n", "TOTAL", treg, trp, tln, tlp
  }
' "$report"
