#!/usr/bin/env bash
set -euo pipefail

THREEVA_BIN="${THREEVA_BIN:-3va}"
PACKAGES=(express fastify koa axios lodash dayjs zod jsonwebtoken bcryptjs winston chalk dotenv cors helmet body-parser multer uuid nanoid ioredis mongoose next @nestjs/core @tauri-apps/cli react-native)
TOTAL=${#PACKAGES[@]}
PASS=0
FAIL=0
RESULTS=()

declare -A PKG_PERMS=(
    ["express"]="--allow-net=localhost:0 --allow-read=."
    ["fastify"]="--allow-net=localhost:0 --allow-read=."
    ["koa"]="--allow-net=localhost:0 --allow-read=."
    ["axios"]="--allow-net=localhost:0 --allow-read=."
    ["lodash"]="--allow-read=."
    ["dayjs"]="--allow-read=."
    ["zod"]="--allow-read=."
    ["jsonwebtoken"]="--allow-read=."
    ["bcryptjs"]="--allow-read=."
    ["winston"]="--allow-read=. --allow-write=."
    ["chalk"]="--allow-read=."
    ["dotenv"]="--allow-read=."
    ["cors"]="--allow-read=."
    ["helmet"]="--allow-read=."
    ["body-parser"]="--allow-read=."
    ["multer"]="--allow-read=. --allow-write=."
    ["uuid"]="--allow-read=."
    ["nanoid"]="--allow-read=."
    ["ioredis"]="--allow-net=localhost:0 --allow-read=."
    ["mongoose"]="--allow-net=localhost:0 --allow-read=."
    ["next"]="--allow-net=localhost:0 --allow-read=. --allow-write=."
    ["@nestjs/core"]="--allow-net=localhost:0 --allow-read=. --allow-write=."
    ["@tauri-apps/cli"]="--allow-read=. --allow-write=."
    ["react-native"]="--allow-read=. --allow-write=."
)

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

cd "$TMPDIR"
echo '{"name":"compat-check","version":"1.0.0","private":true}' > package.json

echo "=== 3va Compatibility Check ==="
echo "Installing packages..."

npm install --save "${PACKAGES[@]}" 2>&1 | tail -3

echo ""
echo "Running smoke tests..."
echo "─────────────────────────────────────"

for pkg in "${PACKAGES[@]}"; do
    TEST_FILE="$TMPDIR/test_${pkg//\//_}.js"
    OUT_FILE="$TMPDIR/out_${pkg//\//_}.txt"
    cat > "$TEST_FILE" <<'JSEOF'
try {
    var mod = require(process.argv[2]);
    if (mod === null || mod === undefined) {
        process.stdout.write('NULL_EXPORT\n');
        process.exit(1);
    }
    process.stdout.write('OK\n');
    process.exit(0);
} catch(e) {
    process.stdout.write('ERROR:' + e.message + '\n');
    process.exit(1);
}
JSEOF

    PERMS="${PKG_PERMS[$pkg]:-}"
    if [ -n "$PERMS" ]; then
        $THREEVA_BIN run $PERMS "$TEST_FILE" -- "$pkg" >"$OUT_FILE" 2>&1 &
    else
        $THREEVA_BIN run "$TEST_FILE" -- "$pkg" >"$OUT_FILE" 2>&1 &
    fi
    PID=$!
    sleep 3
    kill -9 $PID 2>/dev/null || true
    wait $PID 2>/dev/null || true
    OUTPUT=$(cat "$OUT_FILE" 2>/dev/null || true)

    if echo "$OUTPUT" | grep -q '^OK$'; then
        RESULTS+=("✓ $pkg")
        PASS=$((PASS + 1))
    else
        ERR=$(echo "$OUTPUT" | grep '^ERROR:' | head -1 | sed 's/^ERROR://')
        [ -z "$ERR" ] && ERR="unknown error"
        RESULTS+=("✗ $pkg FAILED: $ERR")
        FAIL=$((FAIL + 1))
    fi
done

echo ""
for r in "${RESULTS[@]}"; do
    echo "  $r"
done

echo "─────────────────────────────────────"
PCT=$((PASS * 100 / TOTAL))
echo "Compatibility: $PASS/$TOTAL ($PCT%)"
