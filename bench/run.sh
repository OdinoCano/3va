#!/usr/bin/env bash
# Reproducible startup/throughput/memory/install benchmarks for 3va, and
# for Node/Bun where available, all on whatever machine this runs on.
#
# Why this exists: numbers pasted into a README are a "trust me." This
# script is the "check me" — clone the repo, run it, get your own numbers
# on your own hardware. See bench/README.md for what each figure means and
# why the throughput test needs bench/3va.config.json.
#
# Usage:
#   bench/run.sh                # runs everything available
#   BIN_3VA=/path/to/3va bench/run.sh   # point at a specific 3va binary
#
# Requires: hyperfine, oha (both `cargo install <name>`), curl.
# Node/Bun are optional — skipped with a note if not on PATH.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

for tool in hyperfine oha curl python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "✗ $tool is required. Install: cargo install $tool (curl is usually preinstalled)." >&2
    exit 1
  fi
done

BIN_3VA="${BIN_3VA:-}"
if [ -z "$BIN_3VA" ]; then
  if [ -x "../target/release/3va" ]; then
    BIN_3VA="$(cd .. && pwd)/target/release/3va"
  elif command -v 3va >/dev/null 2>&1; then
    BIN_3VA="$(command -v 3va)"
  else
    echo "✗ No 3va release binary found. Run: cargo build --release -p vvva_cli" >&2
    exit 1
  fi
fi
echo "3va binary: $BIN_3VA" >&2
case "$BIN_3VA" in
  */debug/*)
    echo "⚠ This looks like a debug build (path contains /debug/) — numbers" >&2
    echo "  will not be representative. Build with: cargo build --release -p vvva_cli" >&2
    ;;
esac

HAVE_NODE=0; command -v node >/dev/null 2>&1 && HAVE_NODE=1
HAVE_BUN=0; command -v bun >/dev/null 2>&1 && HAVE_BUN=1

RESULTS_DIR="$(mktemp -d)"
trap 'rm -rf "$RESULTS_DIR"' EXIT

# ── Startup: hello world ────────────────────────────────────────────────────
echo "## Startup (hello world)"
echo
echo "| Runtime | Mean | Range |"
echo "|---|---|---|"

hf_row() {
  local label="$1"; shift
  hyperfine --warmup 5 --min-runs 30 --export-json "$RESULTS_DIR/$label.json" "$@" >&2
  python3 - "$RESULTS_DIR/$label.json" "$label" <<'EOF'
import json, sys
data = json.load(open(sys.argv[1]))["results"][0]
mean_ms = data["mean"] * 1000
mn_ms = data["min"] * 1000
mx_ms = data["max"] * 1000
print(f"| {sys.argv[2]} | {mean_ms:.1f} ms | {mn_ms:.1f}–{mx_ms:.1f} ms |")
EOF
}

hf_row "3va" "$BIN_3VA run hello.js --allow-read=."
[ "$HAVE_NODE" = 1 ] && hf_row "node" "node hello.js"
[ "$HAVE_BUN" = 1 ] && hf_row "bun" "bun run hello.js"
echo

# ── Install: warm, already satisfied ────────────────────────────────────────
echo "## Install (warm — dependency already present)"
echo
WORKDIR="$RESULTS_DIR/install"
mkdir -p "$WORKDIR"
cat > "$WORKDIR/package.json" <<'EOF'
{"name":"bench","version":"1.0.0","dependencies":{"is-odd":"^3.0.1"}}
EOF
( cd "$WORKDIR" && "$BIN_3VA" install --allow-net=registry.npmjs.org >/dev/null 2>&1 )
echo "| Tool | Mean | Range |"
echo "|---|---|---|"
( cd "$WORKDIR" && hyperfine --warmup 3 --min-runs 15 --export-json "$RESULTS_DIR/install-3va.json" \
    "$BIN_3VA install --allow-net=registry.npmjs.org" >&2 )
python3 - "$RESULTS_DIR/install-3va.json" "3va install" <<'EOF'
import json, sys
data = json.load(open(sys.argv[1]))["results"][0]
print(f"| {sys.argv[2]} | {data['mean']*1000:.1f} ms | {data['min']*1000:.1f}–{data['max']*1000:.1f} ms |")
EOF
echo
echo "_Only 3va is measured here — this is a different comparison than the_"
echo "_npm/pnpm/Bun install-speed numbers in the main README, which aren't_"
echo "_yet scripted. See bench/README.md._"
echo

# ── HTTP throughput + memory ─────────────────────────────────────────────────
echo "## HTTP throughput (100k requests, 1,000 concurrent) and memory"
echo
echo "| Runtime | Req/s | Success | Memory (idle) | Memory (post-load) |"
echo "|---|---|---|---|---|"

# Sets the global SERVER_PID rather than `echo`ing the PID for a caller to
# capture via `$(...)` — a background job started inside a function called
# through command substitution is not guaranteed to survive the subshell
# that command substitution creates. Assigning a global from a directly
# (non-substituted) invoked function avoids that entirely.
#
# Deliberately no `eval` here: `eval "PORT=$2 $1" &` forks an intermediate
# shell to run the eval'd string, and `$!` ends up pointing at *that*
# wrapper — not the actual runtime process, which shows up as its child.
# `$1` is intentionally unquoted so it word-splits into argv the normal
# way; every caller in this file is a literal command we wrote ourselves,
# not external input, so that's safe here.
start_server() {
  PORT="$2" $1 >/tmp/bench-server.log 2>&1 &
  SERVER_PID=$!
}

rss_kb() {
  awk '/^VmRSS:/ {print $2}' "/proc/$1/status" 2>/dev/null || echo "0"
}

# Polls instead of a single fixed sleep: a loaded machine (or a runtime
# with a slower cold start) can take longer than a flat 1s to actually
# accept connections, and a Server object's RSS can read as near-zero for
# a moment right after fork before its heap is warmed up.
wait_for_server() {
  local port="$1" pid="$2" tries=0
  while [ "$tries" -lt 50 ]; do
    if kill -0 "$pid" 2>/dev/null && curl -s -m 1 -o /dev/null "http://127.0.0.1:$port/"; then
      return 0
    fi
    tries=$((tries + 1))
    sleep 0.1
  done
  return 1
}

wait_for_rss() {
  local pid="$1" tries=0 kb
  while [ "$tries" -lt 30 ]; do
    kb=$(rss_kb "$pid")
    if [ "$kb" -gt 3000 ] 2>/dev/null; then
      echo "$kb"
      return 0
    fi
    tries=$((tries + 1))
    sleep 0.1
  done
  echo "$kb"
}

bench_http() {
  local label="$1" cmd="$2" port="$3"
  start_server "$cmd" "$port"
  local pid="$SERVER_PID"
  if ! wait_for_server "$port" "$pid"; then
    echo "| $label | — | — | — | server did not start |"
    kill -9 "$pid" 2>/dev/null || true
    return
  fi
  local idle_kb
  idle_kb=$(wait_for_rss "$pid")
  local out
  out=$(oha -n 100000 -c 1000 --no-tui --output-format json "http://127.0.0.1:$port/" 2>/dev/null)
  local loaded_kb
  loaded_kb=$(rss_kb "$pid")
  kill -9 "$pid" 2>/dev/null || true
  echo "$out" > "$RESULTS_DIR/http-$label.json"
  python3 - "$RESULTS_DIR/http-$label.json" "$label" "$idle_kb" "$loaded_kb" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1]))
rps = d["summary"]["requestsPerSec"]
success = d["summary"]["successRate"] * 100
idle_mb = int(sys.argv[3]) / 1024
loaded_mb = int(sys.argv[4]) / 1024
print(f"| {sys.argv[2]} | {rps:,.0f} | {success:.1f}% | {idle_mb:.1f} MB | {loaded_mb:.1f} MB |")
EOF
}

bench_http "3va" "$BIN_3VA run server.js --allow-net= --allow-read=." 8811
if [ "$HAVE_NODE" = 1 ]; then bench_http "node" "node server.js" 8812; fi
if [ "$HAVE_BUN" = 1 ]; then bench_http "bun" "bun run server.js" 8813; fi
echo
echo "_3va's HTTP throughput above uses bench/3va.config.json's opened-up_"
echo "_firewall limits (this directory is the server's cwd). With 3va's_"
echo "_**default** firewall (100 req/s, 50 connections per IP) a single-IP_"
echo "_load test like this one is mostly rejected with 403 — that's the_"
echo "_firewall working as designed, not a bug. See bench/README.md._"
echo

echo "Done."
