#!/usr/bin/env bash
# Local performance regression check: 3va startup and plain-HTTP throughput
# against a baseline taken on THIS machine (numbers don't travel between
# machines, so the baseline lives in ~/.cache, not in the repo).
#
#   scripts/perf-check.sh --update   # on main, before your change: record
#   scripts/perf-check.sh            # after your change: compare, exit 1 on regression
#
# Needs: hyperfine, oha, jq, curl. BIN_3VA=/path/to/3va to test another binary
# (default target/release/3va). TOL=<percent> sets the noise margin (default 15).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

for t in hyperfine oha jq curl; do command -v "$t" >/dev/null || { echo "missing $t" >&2; exit 2; }; done
BIN="${BIN_3VA:-target/release/3va}"
[ -x "$BIN" ] || { echo "build first: cargo build --release ($BIN not found)" >&2; exit 2; }
BASE="${XDG_CACHE_HOME:-$HOME/.cache}/3va/perf-baseline.json"
TOL="${TOL:-15}"
PORT=48771

HF=$(mktemp); trap 'rm -f "$HF"' EXIT
hyperfine -N --warmup 5 --runs 40 --export-json "$HF" "$BIN run bench/hello.js" >/dev/null 2>&1
startup_ms=$(jq '.results[0].median * 1000' "$HF")

PORT=$PORT "$BIN" run bench/server.js --allow-net=127.0.0.1 --allow-env=PORT >/dev/null 2>&1 &
SRV=$!
trap 'kill -9 $SRV 2>/dev/null || true; rm -f "$HF"' EXIT
for _ in $(seq 50); do curl -fs "http://127.0.0.1:$PORT/" >/dev/null 2>&1 && break; sleep 0.1; done
oha -n 20000 -c 64 --no-tui "http://127.0.0.1:$PORT/" >/dev/null 2>&1   # warm up
runs=""
for _ in 1 2 3; do
  runs+="$(oha -n 100000 -c 256 --no-tui --output-format json "http://127.0.0.1:$PORT/" 2>/dev/null \
    | jq -c '[.summary.requestsPerSec, .latencyPercentiles.p99 * 1000]') "
done
# median run by req/s
read -r rps p99 < <(echo "$runs" | jq -s -r 'sort_by(.[0]) | .[1] | "\(.[0]) \(.[1])"')

now=$(jq -n --argjson s "$startup_ms" --argjson r "$rps" --argjson p "$p99" \
  '{startup_ms:$s, http_rps:$r, http_p99_ms:$p}')
echo "now:  $now"

if [ "${1:-}" = "--update" ]; then
  mkdir -p "$(dirname "$BASE")"; echo "$now" > "$BASE"; echo "baseline saved to $BASE"; exit 0
fi
[ -f "$BASE" ] || { echo "no baseline: run '$0 --update' on main first" >&2; exit 2; }
echo "base: $(jq -c . "$BASE")"

# regression = startup more than TOL% higher, p99 more than 2×TOL% higher (it is the noisiest), or req/s more than TOL% lower
msgs=$(jq -n --argjson n "$now" --slurpfile b "$BASE" --argjson t "$TOL" '
  $b[0] as $b | ($t/100) as $f
  | [ if $n.startup_ms  > $b.startup_ms  * (1+$f) then "startup slower: \($n.startup_ms) ms vs \($b.startup_ms)" else empty end,
      if $n.http_rps    < $b.http_rps    * (1-$f) then "throughput lower: \($n.http_rps) req/s vs \($b.http_rps)" else empty end,
      if $n.http_p99_ms > $b.http_p99_ms * (1+2*$f) then "p99 higher: \($n.http_p99_ms) ms vs \($b.http_p99_ms)" else empty end ]
  | .[]' -r)
if [ -n "$msgs" ]; then echo "$msgs" | sed 's/^/REGRESSION: /'; exit 1; fi
echo "OK: within ${TOL}% of baseline"
