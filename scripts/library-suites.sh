#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (c) 3va contributors
#
# Run the *real* test suites of common libraries with 3va instead of node.
# Step 4 of the roadmap in docs/09-testing/07-compatibilidad-node.md (§7.4).
#
# Method: clone the library at a pinned tag, `npm install` its dev deps, then run
# the package's own `test` script with a `node` shim first on PATH that delegates
# to `3va run`. npm itself is *not* put behind the shim (that killed npm before
# it spawned anything): the script is extracted from package.json and run with
# `sh -c`, and only the tooling it spawns (mocha/jest/…) goes through the shim.
# Results (one line per library) go to tests/library-suites/results.txt.
#
# Limitations (documented, not hidden) — a "fail" here is not necessarily a 3va
# bug:
#   - tests that require Node internals (`internalBinding`, `require('internal/…')`)
#     cannot run;
#   - tests that spawn the *node* binary (e.g. `process.execPath`) exercise 3va
#     through the shim only if they go through the shebang;
#   - Node flags our shim does not translate (`--test`, `--require`, `-e`, …) are
#     passed as script argv, so runners that rely on them fail for the wrong
#     reason;
#   - some suites expect to be run from a git checkout with a specific runner.
# The shim translates `node <script> [args…]` into `3va run <script> -- [args…]`.
#
# Usage:
#   scripts/library-suites.sh                 # all targets
#   scripts/library-suites.sh express body    # substring filter
#   scripts/library-suites.sh --list
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${LIBRARY_WORK_DIR:-$ROOT/target/library-suites}"
RESULTS="$ROOT/tests/library-suites/results.txt"
BIN="${THREEVA_BIN:-$ROOT/target/release/3va}"
[ -x "$BIN" ] || BIN="$ROOT/target/debug/3va"

# name|repo|tag|version-for-the-report
TARGETS=(
  "express4|https://github.com/expressjs/express.git|4.22.0|4.22.0"
  "express5|https://github.com/expressjs/express.git|v5.3.0|5.3.0"
  "body-parser|https://github.com/expressjs/body-parser.git|2.0.2|2.0.2"
  "axios|https://github.com/axios/axios.git|v1.20.0|1.20.0"
  "multer|https://github.com/expressjs/multer.git|v2.4.0|2.4.0"
  "ws|https://github.com/websockets/ws.git|8.19.0|8.19.0"
  "fastify|https://github.com/fastify/fastify.git|v5.12.7|5.12.7"
  # Tags are per release, not per npm version; the report shows the tag used.
  "pg|https://github.com/brianc/node-postgres.git|pg@8.19.0|8.19.0"
)

if [ "${1:-}" = "--list" ]; then
  for t in "${TARGETS[@]}"; do echo "${t%%|*}"; done
  exit 0
fi

FILTER="$*"
matches() { [ -z "$FILTER" ] && return 0; case "$1" in *"$FILTER"*) return 0 ;; esac; case " $FILTER " in *" $1 "*) return 0 ;; esac; return 1; }

[ -x "$BIN" ] || { echo "3va binary not found (set THREEVA_BIN): $BIN" >&2; exit 2; }

mkdir -p "$WORK/bin" "$(dirname "$RESULTS")"

# The node shim: `node script [args]` → `3va run script -- [args]`, except for
# npm/npx's own CLI, which must keep running on the real node (otherwise npm
# itself is executed by 3va and dies before spawning any test).
REAL_NODE="$(command -v node || true)"
cat > "$WORK/bin/node" <<SHIM
#!/usr/bin/env bash
if [ "\$#" -eq 0 ]; then exec "$BIN" run --no-prompt; fi
script="\$1"; shift
case "\$script" in
  */npm-cli.js|*/npx-cli.js) exec "$REAL_NODE" "\$script" "\$@" ;;
esac
exec "$BIN" run --no-prompt --allow-read --allow-write --allow-net \\
  --allow-child-process --allow-env --allow-ffi --allow-insecure \\
  "\$script" -- "\$@"
SHIM
chmod +x "$WORK/bin/node"

date_utc="$(date -u +%Y-%m-%d)"
{
  echo "# Real library test suites run with 3va ($("$BIN" --version 2>/dev/null | head -1)) — $date_utc"
  echo "# One line per library: name, version, npm-test exit, tail of the output."
  echo "# Method and limitations: scripts/library-suites.sh header."
} > "$RESULTS"

for entry in "${TARGETS[@]}"; do
  IFS='|' read -r name repo tag report <<<"$entry"
  matches "$name" || continue
  dir="$WORK/$name"
  echo "── $name ($report)"
  if [ ! -d "$dir/.git" ]; then
    rm -rf "$dir"
    if ! git clone --quiet --depth 1 --branch "$tag" "$repo" "$dir" 2>/dev/null; then
      printf '%s\t%s\tSKIP\ttag %s not found\n' "$name" "$report" "$tag" >> "$RESULTS"
      echo "   tag not found: $tag"
      continue
    fi
  fi
  ( cd "$dir" && { npm install --no-audit --no-fund --silent >/dev/null 2>&1 || npm ci --no-audit --no-fund --silent >/dev/null 2>&1 || true; } )
  log="$WORK/$name.test.log"
  # The package's own test command, run with the shim in PATH so the runner
  # (mocha/jest/…) executes under 3va while npm/node stay real.
  testscript="$( cd "$dir" && "$REAL_NODE" -p 'require("./package.json").scripts && require("./package.json").scripts.test || ""' 2>/dev/null || true)"
  if [ -z "$testscript" ]; then
    printf '%s\t%s\tSKIP\tno test script\n' "$name" "$report" >> "$RESULTS"
    echo "   no test script"
    continue
  fi
  set +e
  ( cd "$dir" && PATH="$WORK/bin:$dir/node_modules/.bin:$PATH" timeout 900 sh -c "$testscript" ) >"$log" 2>&1
  st3=$?
  ( cd "$dir" && PATH="$dir/node_modules/.bin:$PATH" timeout 900 sh -c "$testscript" ) >"$log.node" 2>&1
  stn=$?
  set -e
  tail_line="$(grep -Eo '[0-9]+ (passing|failed|pending)' "$log" 2>/dev/null | tail -1 | tr -s ' ' || true)"
  printf '%s\t%s\t3va=%s (%s)\tnode=%s\n' "$name" "$report" "$st3" "${tail_line:-no summary}" "$stn" >> "$RESULTS"
  echo "   3va=$st3 ${tail_line:-} | node=$stn"
done

echo "results: $RESULTS"
