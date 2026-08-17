#!/usr/bin/env bash
# Scans real-world JS/TS sources vendored under .compatibility/ (node, v8, bun,
# pnpm, vite) and asks 3va to parse+transpile+load each file. A file "passes"
# if 3va gets past compilation (runtime errors are expected — these files are
# internal modules run out of context — a *compile* error means the
# transpiler/parser/builtins choked on real-world syntax or a missing module).
set -uo pipefail

THREEVA_BIN="${THREEVA_BIN:-$(dirname "$0")/../target/release/3va}"
SAMPLE="${SAMPLE:-150}"
TIMEOUT="${TIMEOUT:-3}"

declare -A SOURCES=(
    ["node-lib"]=".compatibility/node/lib"
    ["v8-mjsunit"]=".compatibility/v8/test/mjsunit"
    ["bun-test"]=".compatibility/bun/test"
    ["pnpm-src"]=".compatibility/pnpm"
    ["vite-src"]=".compatibility/vite/packages"
)

TOTAL=0
TOTAL_PASS=0
declare -A RESULTS

for name in "${!SOURCES[@]}"; do
    dir="${SOURCES[$name]}"
    [ -d "$dir" ] || continue

    mapfile -t files < <(find "$dir" \( -name "*.js" -o -name "*.mjs" -o -name "*.cjs" -o -name "*.ts" \) \
        -not -path "*/node_modules/*" -not -path "*/dist/*" -not -path "*/.git/*" \
        -not -name "*.min.js" -not -name "*.d.ts" | shuf -n "$SAMPLE" --random-source=<(yes 42))

    pass=0
    total=${#files[@]}
    for f in "${files[@]}"; do
        out=$(timeout "$TIMEOUT" "$THREEVA_BIN" run --no-prompt --allow-read=. "$f" 2>&1)
        if ! echo "$out" | grep -q "^Error: compile error"; then
            pass=$((pass + 1))
        fi
    done

    RESULTS[$name]="$pass/$total"
    TOTAL=$((TOTAL + total))
    TOTAL_PASS=$((TOTAL_PASS + pass))
done

echo "=== 3va Compatibility Scan vs .compatibility/ ==="
for name in "${!RESULTS[@]}"; do
    r="${RESULTS[$name]}"
    p=${r%/*}; t=${r#*/}
    pct=$([ "$t" -gt 0 ] && echo $((p * 100 / t)) || echo 0)
    printf "  %-12s %s (%s%%)\n" "$name" "$r" "$pct"
done
echo "─────────────────────────────────────"
PCT=$([ "$TOTAL" -gt 0 ] && echo $((TOTAL_PASS * 100 / TOTAL)) || echo 0)
echo "Overall: $TOTAL_PASS/$TOTAL ($PCT%)"
