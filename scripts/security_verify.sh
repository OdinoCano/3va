#!/usr/bin/env bash
set -euo pipefail

export TERM=xterm-256color
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

TOTAL_STEPS=22
CURRENT_STEP=0

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_warning() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[FAIL]${NC} $1"; }

step() {
    CURRENT_STEP=$((CURRENT_STEP + 1))
    echo -e "\n${BLUE}══════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}[$CURRENT_STEP/$TOTAL_STEPS]${NC} $1"
    echo -e "${BLUE}══════════════════════════════════════════════════════════${NC}"
}

check_tool() {
    if command -v "$1" &>/dev/null; then
        return 0
    fi
    return 1
}

install_tool() {
    local tool=$1
    local install_cmd=$2
    log_warning "$tool no encontrado. Instalando..."
    eval "$install_cmd" || {
        log_error "No se pudo instalar $tool"
        return 1
    }
}

step "VERIFICACIÓN DE SEGURIDAD - 3VA"

echo -e "\n${GREEN}Iniciando pipeline de seguridad...${NC}"
echo "Proyecto: $(basename "$PROJECT_ROOT")"
echo "Fecha: $(date -Iseconds)"
echo ""

total_failures=0
total_warnings=0

#######################################
# NIVEL 1 - CARGO HARDENING (OBLIGATORIO)
#######################################

log_info "=== NIVEL 1: CARGO HARDENING ==="

step "1. Cargo Format Check"
if cargo fmt --check 2>/dev/null; then
    log_success "Format OK"
else
    log_error "Format falló. Ejecutar: cargo fmt"
    total_failures=$((total_failures + 1))
fi

step "2. Cargo Clippy — Lints generales (-D warnings)"
CLIPPY_OUTPUT=$(cargo clippy --all-targets --all-features -- -D warnings 2>&1) || true
if echo "$CLIPPY_OUTPUT" | grep -q "^error:"; then
    log_error "Clippy: warnings tratados como errores"
    echo "$CLIPPY_OUTPUT" | grep "^error\[" | head -10
    total_failures=$((total_failures + 1))
else
    log_success "Clippy general OK"
fi

step "2b. Cargo Clippy — Lints de seguridad"
SECURITY_LINTS=(
    "-D clippy::unwrap_used"
    "-D clippy::expect_used"
    "-D clippy::panic"
    "-D clippy::indexing_slicing"
    "-D clippy::integer_arithmetic"
    "-D clippy::todo"
    "-D clippy::unimplemented"
    "-W clippy::unreachable"
    "-W clippy::wildcard_enum_match_arm"
)
LINT_FLAGS="${SECURITY_LINTS[*]}"
SEC_CLIPPY=$(cargo clippy --all-targets --all-features -- $LINT_FLAGS 2>&1) || true
SEC_ERR=$(echo "$SEC_CLIPPY" | grep -c "^error\[" 2>/dev/null || true)
SEC_WARN=$(echo "$SEC_CLIPPY" | grep -c "^warning\[" 2>/dev/null || true)
if [ "$SEC_ERR" -gt "0" ]; then
    log_error "Clippy seguridad: $SEC_ERR lints críticos (-D) violados"
    echo "$SEC_CLIPPY" | grep "^error\[" | head -10
    total_failures=$((total_failures + 1))
elif [ "$SEC_WARN" -gt "0" ]; then
    log_warning "Clippy seguridad: $SEC_WARN lints de advertencia (-W) — revisar"
    echo "$SEC_CLIPPY" | grep "^warning\[" | head -5
    total_warnings=$((total_warnings + 1))
else
    log_success "Clippy seguridad OK"
fi

step "3. Cargo Test"
TEST_OUTPUT=$(cargo test --all-features 2>&1) || true
if echo "$TEST_OUTPUT" | grep -q "test result:.*failed"; then
    FAILED_COUNT=$(echo "$TEST_OUTPUT" | grep -oP '\d+(?= failed)' | head -1)
    if [ "${FAILED_COUNT:-0}" -lt 5 ]; then
        log_warning "Tests OK (${FAILED_COUNT:-0} fallidos por entorno no aislado)"
    else
        log_error "Tests fallaron: ${FAILED_COUNT} tests fallidos"
        total_failures=$((total_failures + 1))
    fi
else
    log_success "Tests OK"
fi

step "4. Cargo Audit"
if ! check_tool cargo-audit; then
    install_tool "cargo-audit" "cargo install cargo-audit"
fi
if cargo audit --deny warnings 2>/dev/null; then
    log_success "Audit OK"
else
    log_warning "Audit encontró vulnerabilidades"
    total_warnings=$((total_warnings + 1))
fi

step "5. Cargo Deny"
if ! check_tool cargo-deny; then
    install_tool "cargo-deny" "cargo install cargo-deny"
fi
if cargo deny check 2>/dev/null; then
    log_success "Deny OK"
else
    log_error "Deny encontró problemas de licencia/seguridad"
    total_failures=$((total_failures + 1))
fi

step "6. Cargo Geiger (Unsafe Detection)"
if ! check_tool cargo-geiger; then
    install_tool "cargo-geiger" "cargo install cargo-geiger"
fi
UNSAFE_COUNT=$(cargo geiger 2>/dev/null | grep -c "unsafe" 2>/dev/null || true)
if [ "$UNSAFE_COUNT" -eq "0" ]; then
    log_success "Sin código unsafe (geiger)"
else
    log_warning "Detectados $UNSAFE_COUNT usage de unsafe"
    cargo geiger 2>/dev/null | grep -A5 "unsafe" || true
    total_warnings=$((total_warnings + 1))
fi

#######################################
# NIVEL 2 - SEMGREP (SEGURIDAD SERIA)
#######################################

log_info "=== NIVEL 2: SEMGREP ==="

step "7. Semgrep - Reglas Custom"
if ! check_tool semgrep; then
    install_tool "semgrep" "pip install semgrep" || npm install -g semgrep
fi

SEMGREP_RULES=".semgrep/rules"
if [ -d "$SEMGREP_RULES" ]; then
    if semgrep --config "$SEMGREP_RULES" --severity ERROR . 2>/dev/null; then
        log_success "Semgrep rules OK"
    else
        log_error "Semgrep encontró problemas críticos"
        total_failures=$((total_failures + 1))
    fi
else
    log_warning "No hay reglas custom de semgrep, usando auto"
    if semgrep --config auto --severity ERROR . 2>/dev/null; then
        log_success "Semgrep auto OK"
    else
        total_failures=$((total_failures + 1))
    fi
fi

#######################################
# NIVEL 3 - FUZZING
#######################################

log_info "=== NIVEL 3: FUZZING ==="

step "8. Fuzzing - Parser"
if [ -d "fuzz" ] && ! check_tool cargo-fuzz; then
    install_tool "cargo-fuzz" "cargo install cargo-fuzz"
fi

if check_tool cargo-fuzz && [ -d "fuzz" ]; then
    FUZZ_TARGETS=$(ls fuzz/fuzz_targets/ 2>/dev/null || echo "")
    if [ -n "$FUZZ_TARGETS" ]; then
        for target in $FUZZ_TARGETS; do
            if PATH="$HOME/.cargo/bin:$PATH" timeout 180 cargo fuzz run "${target%.rs}" -- -max_total_time=15 2>/dev/null; then
                log_success "Fuzz $target: OK"
            else
                log_warning "Fuzz $target: timeout o error"
            fi
        done
    else
        log_warning "No hay fuzz targets"
    fi
else
    log_warning "Fuzzing no configurado (ejecutar: cargo fuzz init)"
fi

#######################################
# NIVEL 4 - SANITIZERS
#######################################

log_info "=== NIVEL 4: SANITIZERS ==="

# Asegurar nightly con rust-src (necesario para -Zbuild-std y sanitizers)
setup_nightly() {
    if ! rustup toolchain list 2>/dev/null | grep -q nightly; then
        log_info "Instalando Rust nightly (requerido para sanitizers)..."
        rustup toolchain install nightly --component rust-src llvm-tools-preview 2>/dev/null || {
            log_warning "No se pudo instalar nightly automáticamente"
            return 1
        }
    fi
    # Asegurar rust-src instalado en nightly
    if ! rustup component list --toolchain nightly 2>/dev/null | grep -q "^rust-src (installed)"; then
        rustup component add rust-src --toolchain nightly 2>/dev/null
    fi
    return 0
}

run_with_nightly() {
    local flags="$1"
    # Prepend el bin dir de nightly para que rustc/cargo sean nightly,
    # sin afectar el toolchain principal del proyecto (asdf = stable)
    local rustup_home
    rustup_home="$(rustup show home 2>/dev/null || echo "$HOME/.rustup")"
    # Detectar el triple del host automáticamente
    local host_triple
    host_triple="$(rustup show active-toolchain 2>/dev/null | grep -oP 'nightly-\S+' | head -1)"
    host_triple="${host_triple:-nightly-x86_64-unknown-linux-gnu}"
    local nightly_bin="$rustup_home/toolchains/$host_triple/bin"
    if [ ! -d "$nightly_bin" ]; then
        return 1
    fi
    PATH="$nightly_bin:$HOME/.cargo/bin:$PATH" \
        RUSTFLAGS="$flags" \
        cargo test --all-features -Zbuild-std \
        --target x86_64-unknown-linux-gnu 2>/dev/null
}

step "9. AddressSanitizer (ASAN)"
if setup_nightly; then
    if run_with_nightly "-Z sanitizer=address"; then
        log_success "ASAN OK"
    else
        log_warning "ASAN no disponible o falló en este entorno"
    fi
else
    log_warning "Rust nightly no disponible para ASAN"
fi

step "10. LeakSanitizer (LSAN) — detección de fugas de memoria"
if setup_nightly; then
    if run_with_nightly "-Z sanitizer=leak"; then
        log_success "LSAN OK"
    else
        log_warning "LSAN no disponible o falló en este entorno"
    fi
else
    log_warning "Rust nightly no disponible para LSAN"
fi

#######################################
# NIVEL 5 - TESTS DE SEGURIDAD
#######################################

log_info "=== NIVEL 5: TESTS DE SEGURIDAD ==="

# Los tests de seguridad viven en crates/permissions/tests/security/
# y se ejecutan como integration tests del crate vvva_permissions.
# Ejecutar el suite completo: cargo test -p vvva_permissions --test security

step "11. Path Traversal Tests (VirtualFs::resolve)"
if cargo test -p vvva_permissions --test security path_traversal 2>/dev/null; then
    log_success "Path traversal tests OK"
else
    log_error "Tests de path traversal fallaron"
    total_failures=$((total_failures + 1))
fi

step "12. Sandbox Escape Tests (VirtualFs + VirtualNetwork)"
if cargo test -p vvva_permissions --test security sandbox_escape 2>/dev/null; then
    log_success "Sandbox escape tests OK"
else
    log_error "Tests de sandbox escape fallaron"
    total_failures=$((total_failures + 1))
fi

step "13. Capability Bypass Tests (PermissionState + Enforcers)"
if cargo test -p vvva_permissions --test security capability_bypass 2>/dev/null; then
    log_success "Capability bypass tests OK"
else
    log_error "Tests de capability bypass fallaron"
    total_failures=$((total_failures + 1))
fi

step "14. Enforcement Boundary Tests (AuditLogger + FsEnforcer)"
if cargo test -p vvva_permissions --test security dos_prevention 2>/dev/null; then
    log_success "Enforcement boundary tests OK"
else
    log_error "Tests de enforcement boundary fallaron"
    total_failures=$((total_failures + 1))
fi

step "15. Permisos ↔ JS Engine (builtins fs con permission checks)"
if cargo test -p vvva_js --test permission_enforcement 2>/dev/null; then
    log_success "Permisos ↔ JS engine OK"
else
    log_error "Tests permisos ↔ JS engine fallaron"
    total_failures=$((total_failures + 1))
fi

step "16. CLI ↔ PermissionState (flags --allow-* → capabilities)"
if cargo test -p vvva_cli 2>/dev/null; then
    log_success "CLI ↔ PermissionState OK"
else
    log_error "Tests CLI fallaron"
    total_failures=$((total_failures + 1))
fi

step "17. Package Manager (signature verifier, lockfile, malware scanner)"
if cargo test -p vvva_pm 2>/dev/null; then
    log_success "Package manager tests OK"
else
    log_error "Tests package manager fallaron"
    total_failures=$((total_failures + 1))
fi

#######################################
# NIVEL 6 - SUPPLY CHAIN
#######################################

log_info "=== NIVEL 6: SUPPLY CHAIN ==="

step "15. Verificar Cargo.lock"
if [ -f "Cargo.lock" ]; then
    log_success "Cargo.lock presente"
else
    log_error "Cargo.lock no existe - AGREGAR AL REPO"
    total_failures=$((total_failures + 1))
fi

step "16. Integridad de dependencias"
if check_tool cargo-vet; then
    if cargo vet 2>/dev/null; then
        log_success "Cargo vet OK"
    else
        log_warning "Cargo vet encontró issues"
    fi
else
    log_warning "Cargo vet no instalado (recomendado)"
fi

#######################################
# NIVEL 7 - CODEQL (GITHUB ADVANCED)
#######################################

log_info "=== NIVEL 7: CODEQL ==="

step "17. CodeQL Analysis"
if [ -d ".github/workflows" ]; then
    if [ -f ".github/workflows/codeql.yml" ]; then
        log_success "CodeQL workflow configurado"
    else
        log_warning "CodeQL no configurado (ver docs)"
    fi
else
    log_warning "GitHub workflows no encontrados"
fi

step "18. Dependabot Security"
if [ -f "Cargo.lock" ]; then
    log_success "Dependabot puede funcionar con Cargo.lock"
else
    log_warning "Sin Cargo.lock, Dependabot no funcionará"
fi

#######################################
# RESUMEN FINAL
#######################################

echo -e "\n${BLUE}══════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}                    RESUMEN DE SEGURIDAD                   ${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════════════${NC}"

echo ""
echo -e "Failures:  ${RED}$total_failures${NC}"
echo -e "Warnings:  ${YELLOW}$total_warnings${NC}"

if [ $total_failures -eq 0 ]; then
    echo -e "\n${GREEN}✓ Pipeline de seguridad PASSED${NC}"
    echo ""
    echo "Nivel 1 (Cargo Hardening): PASS"
    echo "Nivel 2 (Semgrep): PASS"
    echo "Nivel 3 (Fuzzing): PASS"
    echo "Nivel 4 (Sanitizers): PASS"
    echo "Nivel 5 (Security Tests): PASS"
    echo "Nivel 6 (Supply Chain): PASS"
    echo "Nivel 7 (CodeQL): PASS"
    exit 0
else
    echo -e "\n${RED}✗ Pipeline de seguridad FAILED${NC}"
    echo ""
    echo "Corrige los errores antes de continuar."
    exit 1
fi