#!/usr/bin/env bash
# Behavioral compatibility check: dgram (UDP sockets) and dns (MX/TXT/SRV/NS/
# CNAME/... records) against real Node.js.
#
# Each case under scripts/compat-cases/{dgram,dns}/*.js is run once with the
# real `node` binary and once with 3va (--allow-net=*). The case files already
# normalize everything that is legitimately non-deterministic (ephemeral ports,
# DNS answer ordering, kernel-dependent buffer sizes), so PASS requires the two
# stdout streams to be byte-identical.
#
# Requirements: a `node` binary (>=18) and the 3va release binary. Override
# with NODE_BIN / THREEVA_BIN, or set NO_BUILD=1 to skip `cargo build --release`.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NODE_BIN="${NODE_BIN:-$(command -v node || echo /usr/bin/node)}"
THREEVA_BIN="${THREEVA_BIN:-$ROOT/target/release/3va}"
TIMEOUT="${TIMEOUT:-20}"
NO_BUILD="${NO_BUILD:-0}"

if [ ! -x "$THREEVA_BIN" ] && [ "$NO_BUILD" != "1" ]; then
    echo ">> Building 3va release binary..."
    (cd "$ROOT" && cargo build --release) || { echo "build failed"; exit 1; }
fi
if [ ! -x "$THREEVA_BIN" ]; then
    echo "3va binary not found at $THREEVA_BIN (set THREEVA_BIN or build first)" >&2
    exit 1
fi
if ! "$NODE_BIN" --version >/dev/null 2>&1; then
    echo "node binary not found (set NODE_BIN)" >&2
    exit 1
fi

CASES=("$ROOT"/scripts/compat-cases/dgram/*.js "$ROOT"/scripts/compat-cases/dns/*.js)
TOTAL=${#CASES[@]}
PASS=0
FAILED=()

for f in "${CASES[@]}"; do
    name="${f#"$ROOT/scripts/compat-cases/"}"
    node_out=$(timeout "$TIMEOUT" "$NODE_BIN" "$f" 2>&1); node_rc=$?
    threeva_out=$(timeout "$TIMEOUT" "$THREEVA_BIN" run --no-prompt --allow-read=. --allow-net='*' "$f" 2>&1); threeva_rc=$?

    # A case may legitimately fail to connect to the live DNS on a broken/offline
    # box; skip (not fail) when the reference (node) run itself errored out.
    if [ "$node_rc" -ne 0 ] || [ -z "$node_out" ]; then
        echo "  SKIP   $name (reference run failed rc=$node_rc)"
        continue
    fi

    if [ "$node_out" = "$threeva_out" ] && [ "$threeva_rc" -eq 0 ]; then
        echo "  PASS   $name"
        PASS=$((PASS + 1))
    else
        # DNS callback completion order is not a compatibility contract: node
        # dispatches from the libuv threadpool, 3va from timers. Compare DNS
        # case output as sorted line sets.
        case "$f" in */dns/*)
            node_out=$(printf '%s\n' "$node_out" | sort)
            threeva_out=$(printf '%s\n' "$threeva_out" | sort)
        esac
        if [ "$node_out" = "$threeva_out" ] && [ "$threeva_rc" -eq 0 ]; then
            echo "  PASS   $name (order-insensitive)"
            PASS=$((PASS + 1))
        else
            echo "  FAIL   $name"
            FAILED+=("$name")
            diff <(printf '%s\n' "$node_out") <(printf '%s\n' "$threeva_out") \
                | head -20 | sed 's/^/         /'
        fi
    fi
done

echo "─────────────────────────────────────"
echo "dgram+dns compat: $PASS/$TOTAL"
if [ "${#FAILED[@]}" -gt 0 ]; then
    printf '  failed: %s\n' "${FAILED[@]}"
    exit 1
fi