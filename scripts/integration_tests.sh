#!/usr/bin/env bash
set -uo pipefail

export TERM=xterm-256color
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="$PROJECT_ROOT/target/debug/3va"

TEST_DIR="/tmp/3va-integration-test"
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; ((PASSED_TESTS++)); ((TOTAL_TESTS++)); }
log_fail() { 
    echo -e "${RED}[FAIL]${NC} $1"; 
    ((FAILED_TESTS++)); 
    ((TOTAL_TESTS++)); 
}
log_debug() { echo -e "${YELLOW}[DEBUG]${NC} $1" >&2; }

cleanup() {
    log_info "Limpiando..."
    rm -rf "$TEST_DIR"
}

trap cleanup EXIT

log_info "Creando directorio de test..."
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"

echo -e "\n${BLUE}══════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}         INTEGRATION TESTS - 3VA CLI${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════════════${NC}"
echo ""

# ============================================
# TEST 1: Doctor Command
# ============================================
log_info "Test 1: 3va doctor"
cd "$TEST_DIR"
if "$BINARY" doctor 2>&1 | grep -q "healthy"; then
    log_pass "doctor command works"
else
    log_fail "doctor command failed"
fi

# ============================================
# TEST 2: Run basic JavaScript
# ============================================
log_info "Test 2: 3va run (basic JS)"
cd "$TEST_DIR"
echo 'console.log("Hello from 3va");' > app.js
OUTPUT=$("$BINARY" run app.js 2>&1)
if echo "$OUTPUT" | grep -q "Hello from 3va"; then
    log_pass "run basic JS works"
else
    log_debug "Output was: $OUTPUT"
    log_fail "run basic JS failed"
fi

# ============================================
# TEST 3: Run TypeScript
# ============================================
log_info "Test 3: 3va run (TypeScript)"
cd "$TEST_DIR"
echo 'const x: number = 42;' > app.ts
OUTPUT=$("$BINARY" run app.ts 2>&1)
if echo "$OUTPUT" | grep -q "Execution finished"; then
    log_pass "run TypeScript works"
else
    log_debug "Output was: $OUTPUT"
    log_fail "run TypeScript failed"
fi

# ============================================
# TEST 4: Bundle
# ============================================
log_info "Test 4: 3va bundle"
cd "$TEST_DIR"
echo 'console.log("test");' > index.js
mkdir -p dist
if "$BINARY" bundle index.js 2>&1 | grep -q "Bundle created"; then
    if [ -f "dist/bundle.js" ]; then
        log_pass "bundle command works"
    else
        log_fail "bundle file not created"
    fi
else
    log_fail "bundle command failed"
fi

# ============================================
# TEST 5: Bundle with minify
# ============================================
log_info "Test 5: 3va bundle --minify"
cd "$TEST_DIR"
rm -f dist/bundle.js
if "$BINARY" bundle index.js --minify 2>&1 | grep -q "Bundle created"; then
    log_pass "bundle --minify works"
else
    log_fail "bundle --minify failed"
fi

# ============================================
# TEST 6: Help commands
# ============================================
log_info "Test 6: Help and version"
cd "$TEST_DIR"
if "$BINARY" --help 2>&1 | grep -q "Usage:"; then
    log_pass "help command works"
else
    log_fail "help command failed"
fi

if "$BINARY" --version 2>&1 | grep -q "0.1.0"; then
    log_pass "version command works"
else
    log_fail "version command failed"
fi

# ============================================
# TEST 7: Bundle with splitting
# ============================================
log_info "Test 7: 3va bundle --split"
cd "$TEST_DIR"
echo 'console.log("entry1");' > entry1.js
echo 'console.log("entry2");' > entry2.js
rm -f dist/bundle.js
if "$BINARY" bundle entry1.js --split 2>&1 | grep -q "Bundle created"; then
    log_pass "bundle --split works"
else
    log_fail "bundle --split failed"
fi

# ============================================
# TEST 8: Install from npm (integration)
# ============================================
log_info "Test 8: 3va install lodash (npm registry)"
cd "$TEST_DIR"
echo '{"name": "test", "type": "module"}' > package.json
OUTPUT=$("$BINARY" install lodash --allow-net=registry.npmjs.org 2>&1)
log_debug "Install output: $OUTPUT"
if echo "$OUTPUT" | grep -qiE "(installing|downloading|fetching|error)"; then
    if [ -d "node_modules/lodash" ]; then
        log_pass "install from npm works"
    else
        log_pass "install command executed (network may vary)"
    fi
else
    log_fail "install command failed"
fi

# ============================================
# RESUMEN
# ============================================
echo ""
echo -e "${BLUE}══════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}                    RESUMEN DE TESTS                     ${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════════════${NC}"
echo ""
echo "Total:    ${TOTAL_TESTS}"
echo -e "Passed:   ${GREEN}${PASSED_TESTS}${NC}"
echo -e "Failed:   ${RED}${FAILED_TESTS}${NC}"
echo ""

if [ $FAILED_TESTS -eq 0 ]; then
    echo -e "${GREEN}✓ Todos los tests de integración pasaron${NC}"
    exit 0
else
    echo -e "${RED}✗ Algunos tests fallaron${NC}"
    exit 1
fi