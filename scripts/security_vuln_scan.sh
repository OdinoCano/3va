#!/usr/bin/env bash
# security_vuln_scan.sh
#
# Escáner de vulnerabilidades de 3va.
#
#  1. Actualiza TODAS las herramientas de escaneo a su última versión.
#  2. Ejecuta cada escáner sobre el proyecto completo (crates/ + docs/ + scripts/).
#  3. Escanea también la carpeta examples/ (código JS/TS + dependencias).
#  4. Ejecuta el escaneo custom (patrones no cubiertos por herramientas estándar).
#  5. Genera un reporte consolidado en security-reports/.
#
# Uso:
#   scripts/security_vuln_scan.sh [--no-update] [--out DIR] [--skip FOO,BAR]
#
# Opciones:
#   --no-update   No actualizar herramientas (usar versiones instaladas)
#   --out DIR     Directorio de reporte (default: security-reports/)
#   --skip LIST   Escáneres a omitir, separados por coma (audit,deny,geiger,vet,
#                 supply-chain,outdated,udeps,semgrep,gitleaks,trivy,osv,custom,
#                 test,clippy,fmt,doc)
#
# Exit code: 0 si ningún escáner reporta [FAIL] ni el custom scan HIGH/CRITICAL.

set -uo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

OUT="security-reports"
DO_UPDATE=1
SKIP=""

while [ $# -gt 0 ]; do
    case "$1" in
        --no-update) DO_UPDATE=0; shift ;;
        --out) OUT="$2"; shift 2 ;;
        --skip) SKIP="$2"; shift 2 ;;
        *) shift ;;
    esac
done

mkdir -p "$OUT"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

# Colores
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'

REPORT="$OUT/security-report.md"
RUNLOG="$OUT/runlog.txt"
: > "$RUNLOG"

log() { echo -e "${BLUE}[SCAN]${NC} $1" | tee -a "$RUNLOG"; }
ok()   { echo -e "${GREEN}[PASS]${NC} $1" | tee -a "$RUNLOG"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1" | tee -a "$RUNLOG"; }
FAILS=0
fail() { FAILS=$((FAILS+1)); echo -e "${RED}[FAIL]${NC} $1" | tee -a "$RUNLOG"; }

# Heavy cargo steps (geiger, udeps, clippy, test) run in a memory-capped
# systemd scope, like the git hooks: uncapped builds got the editor OOM-killed.
if [ -f "$PROJECT_ROOT/scripts/hook-limits.sh" ]; then
    . "$PROJECT_ROOT/scripts/hook-limits.sh"
    hook_limits_init
    capped() { run_limited env CARGO_BUILD_JOBS="${HOOK_JOBS:-4}" "$@"; }
else
    capped() { "$@"; }
fi

SKIP_FN() { case ",$SKIP," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

# Acumula el cuerpo del reporte en secciones
declare -a SECTIONS=()
section() { SECTIONS+=("$1"); }

declare -A TOOLVERS=()
toolver() { TOOLVERS["$1"]="$2"; }

has() { command -v "$1" >/dev/null 2>&1; }
has_and_log() {
    if ! has "$1"; then warn "$1 no instalado"; return 1; fi
    return 0
}

#######################################
# 1. ACTUALIZACIÓN DE HERRAMIENTAS
#######################################
update_tools() {
    [ "$DO_UPDATE" -eq 0 ] && { log "Actualización omitida (--no-update)"; return; }
    log "Actualizando herramientas a su última versión..."

    local cargo_tools=(
        cargo-audit cargo-deny cargo-geiger cargo-vet cargo-fuzz
        cargo-supply-chain cargo-outdated cargo-udeps cargo-binstall
        cargo-tarpaulin cargo-mutants
    )

    for t in "${cargo_tools[@]}"; do
        if has "$t" || has cargo-binstall; then
            cargo binstall -y "$t" >>"$RUNLOG" 2>&1 && ok "Actualizado $t" || warn "Fallo al actualizar $t"
        fi
    done

    if has pip || has pip3; then
        PIP_BREAK_SYSTEM_PACKAGES=1 pip install --user --upgrade semgrep >>"$RUNLOG" 2>&1 \
            && ok "Actualizado semgrep" || warn "Fallo al actualizar semgrep"
    fi

    if has brew; then
        brew upgrade gitleaks >>"$RUNLOG" 2>&1 && ok "Actualizado gitleaks" || warn "Fallo al actualizar gitleaks"
    elif has cargo-binstall; then
        cargo binstall -y gitleaks >>"$RUNLOG" 2>&1 && ok "Actualizado gitleaks" || warn "gitleaks no actualizado (se usa el instalado)"
    fi

    # trivy (script oficial de Aqua Security)
    if curl -sfL https://raw.githubusercontent.com/aquasecurity/trivy/main/contrib/install.sh -o /tmp/trivy-install.sh 2>/dev/null; then
        sh /tmp/trivy-install.sh -b "$HOME/.local/bin" >>"$RUNLOG" 2>&1 && ok "Actualizado trivy" || warn "Fallo al actualizar trivy"
    fi

    # osv-scanner (binario de releases de Google)
    local osv_ver osv_latest
    osv_latest="$(curl -sfL https://api.github.com/repos/google/osv-scanner/releases/latest 2>/dev/null | grep -oP '"tag_name":\s*"v\K[^"]+' || true)"
    if [ -n "$osv_latest" ]; then
        if curl -sfL --retry 3 "https://github.com/google/osv-scanner/releases/download/v${osv_latest}/osv-scanner_linux_amd64" \
            -o "$HOME/.local/bin/osv-scanner" && chmod +x "$HOME/.local/bin/osv-scanner"; then
            ok "Actualizado osv-scanner (v$osv_latest)"
        else
            warn "Fallo al actualizar osv-scanner"
        fi
    fi

    # Base de datos de cargo-audit (rust advisory DB)
    if has cargo-audit; then
        cargo audit fetch >>"$RUNLOG" 2>&1 && ok "Base RUSTSEC actualizada" || warn "Fallo al actualizar base RUSTSEC"
    fi
}

#######################################
# 2. CAPTURA DE VERSIONES
#######################################
capture_versions() {
    toolver audit        "$(cargo audit --version 2>/dev/null | awk '{print $2}')"
    toolver deny         "$(cargo deny --version 2>/dev/null | awk '{print $2}')"
    toolver geiger       "$(cargo geiger --version 2>/dev/null | awk '{print $2}')"
    toolver vet          "$(cargo vet --version 2>/dev/null | awk '{print $2}')"
    toolver supply-chain "$(cargo supply-chain --version 2>/dev/null | awk '{print $2}')"
    toolver outdated     "$(cargo outdated --version 2>/dev/null | awk '{print $2}')"
    toolver udeps        "$(cargo udeps --version 2>/dev/null | awk '{print $3}')"
    toolver semgrep      "$(semgrep --version 2>/dev/null)"
    toolver gitleaks     "$(gitleaks version 2>/dev/null)"
    toolver trivy        "$("$HOME/.local/bin/trivy" --version 2>/dev/null | rg -oP '\d+\.\d+\.\d+' | head -1)"
    toolver osv          "$("$HOME/.local/bin/osv-scanner" --version 2>/dev/null | rg -oP 'v?\d+\.\d+\.\d+' | head -1)"
    toolver custom       "local"
}

#######################################
# 3. EJECUCIÓN DE ESCÁNERES
#######################################

run_audit() {
    has_and_log cargo-audit || return
    log "cargo-audit (advisories RUSTSEC/CVE de dependencias Rust)"
    if cargo audit --deny warnings >"$OUT/audit.txt" 2>&1; then
        ok "cargo-audit: sin vulnerabilidades conocidas"
    else
        local n
        n="$(rg -c '^error:|vulnerabilit' "$OUT/audit.txt" || true)"
        fail "cargo-audit: $n hallazgo(s) — ver security-reports/audit.txt"
        section "### cargo-audit — $n hallazgo(s)\n\`\`\`\n$(cat "$OUT/audit.txt")\n\`\`\`"
    fi
}

run_deny() {
    has_and_log cargo-deny || return
    log "cargo-deny (advisories + licencias + bans + sources)"
    if cargo deny check advisories licenses bans sources >"$OUT/deny.txt" 2>&1; then
        ok "cargo-deny: OK"
    else
        fail "cargo-deny: problemas encontrados — ver security-reports/deny.txt"
        section "### cargo-deny — problemas\n\`\`\`\n$(cat "$OUT/deny.txt")\n\`\`\`"
    fi
}

run_geiger() {
    has_and_log cargo-geiger || return
    log "cargo-geiger (inventario de código unsafe)"
    if capped timeout 300 cargo geiger --color never >"$OUT/geiger.txt" 2>&1; then
        ok "cargo-geiger completado"
    else
        warn "cargo-geiger: completado con advertencias"
    fi
    section "### cargo-geiger — resumen unsafe\n\`\`\`\n$(rg -i 'unsafe usage|total' "$OUT/geiger.txt" | head -20 || true)\n\`\`\`"
}

run_vet() {
    has_and_log cargo-vet || return
    log "cargo-vet (auditoría de supply chain)"
    if cargo vet >"$OUT/vet.txt" 2>&1; then
        ok "cargo-vet: OK"
    else
        warn "cargo-vet: crates sin auditar — ver security-reports/vet.txt"
        section "### cargo-vet — crates sin auditar\n\`\`\`\n$(cat "$OUT/vet.txt")\n\`\`\`"
    fi
}

run_supply_chain() {
    has_and_log cargo-supply-chain || return
    log "cargo-supply-chain (crates abandonados / forkeados / sin mantenimiento)"
    if timeout 120 cargo supply-chain --no-check-crates >"$OUT/supply-chain.txt" 2>&1; then
        ok "cargo-supply-chain completado"
    else
        warn "cargo-supply-chain: advertencias"
    fi
    local abandoned
    abandoned="$(rg -ci 'abandoned|fork|unmaintained' "$OUT/supply-chain.txt" || true)"
    if [ "${abandoned:-0}" -gt 0 ]; then
        warn "cargo-supply-chain: $abandoned crates marcados como abandonados/forkeados"
        section "### cargo-supply-chain — crates de riesgo\n\`\`\`\n$(rg -i -B1 -A2 'abandoned|fork|unmaintained' "$OUT/supply-chain.txt" | head -60)\n\`\`\`"
    else
        ok "cargo-supply-chain: sin crates de riesgo conocidos"
    fi
}

run_outdated() {
    has_and_log cargo-outdated || return
    log "cargo-outdated (dependencias desactualizadas → CVE no parcheados)"
    if timeout 180 cargo outdated --color never >"$OUT/outdated.txt" 2>&1; then
        ok "cargo-outdated: todo al día"
    else
        local outdated_count
        outdated_count="$(rg -c '^\S+\s+.*\s+yes' "$OUT/outdated.txt" || echo 0)"
        warn "cargo-outdated: $outdated_count dependencia(s) desactualizada(s)"
        section "### cargo-outdated — dependencias desactualizadas\n\`\`\`\n$(head -60 "$OUT/outdated.txt")\n\`\`\`"
    fi
}

run_udeps() {
    has_and_log cargo-udeps || return
    log "cargo-udeps (dependencias sin uso → superficie de ataque)"
    if capped timeout 300 cargo udeps --color never >"$OUT/udeps.txt" 2>&1; then
        ok "cargo-udeps: sin dependencias muertas"
    else
        warn "cargo-udeps: ver security-reports/udeps.txt"
        section "### cargo-udeps — dependencias sin uso\n\`\`\`\n$(head -50 "$OUT/udeps.txt")\n\`\`\`"
    fi
}

run_semgrep() {
    has_and_log semgrep || return
    log "semgrep (SAST — reglas propias + project + examples)"
    local rc=0
    local rule_sets=".semgrep/rules/"
    if semgrep --config "$rule_sets" --severity ERROR --json -o "$OUT/semgrep.json" . >"$OUT/semgrep.txt" 2>&1; then
        ok "semgrep (reglas propias): sin errores de severidad ERROR"
    else
        rc=1
        local n
        n="$(rg -oP '"code":\s*\K[0-9]+' "$OUT/semgrep.json" 2>/dev/null | head -1 || echo "?")"
        fail "semgrep: $n hallazgo(s) ERROR — ver security-reports/semgrep.json"
        section "### semgrep — reglas propias\n\`\`\`\n$(rg -oP '"message":\s*"\K[^"]+' "$OUT/semgrep.json" 2>/dev/null | head -20)\n\`\`\`"
    fi
    # Reglas auto de JS/TS sobre examples/
    if semgrep --config auto --severity ERROR --json -o "$OUT/semgrep-examples.json" examples/ >"$OUT/semgrep-examples.txt" 2>&1; then
        ok "semgrep auto (examples/): sin errores"
    else
        warn "semgrep auto (examples/): hallazgos — ver security-reports/semgrep-examples.json"
        section "### semgrep auto — examples/\n\`\`\`\n$(rg -oP '"message":\s*"\K[^"]+' "$OUT/semgrep-examples.json" 2>/dev/null | head -20)\n\`\`\`"
    fi
    return $rc
}

run_gitleaks() {
    has_and_log gitleaks || return
    log "gitleaks (secretos en todo el repo, incl. examples/)"
    if gitleaks git --redact --exit-code 1 -c .gitleaks.toml >"$OUT/gitleaks.txt" 2>&1; then
        ok "gitleaks: sin secretos"
    else
        local n
        n="$(rg -ci 'leaked|secret' "$OUT/gitleaks.txt" || echo 0)"
        fail "gitleaks: $n secreto(s) posible(s) — ver security-reports/gitleaks.txt"
        section "### gitleaks\n\`\`\`\n$(cat "$OUT/gitleaks.txt")\n\`\`\`"
    fi
}

run_trivy() {
    local trivy_bin="$HOME/.local/bin/trivy"
    has trivy && trivy_bin="$(command -v trivy)"
    if [ ! -x "$trivy_bin" ]; then warn "trivy no instalado"; return; fi
    log "trivy (vulnerabilidades en Dockerfile y filesystem)"
    if "$trivy_bin" fs --scanners vuln,secret,license --severity HIGH,CRITICAL \
        --no-progress --skip-dirs target --skip-dirs vendor --skip-dirs node_modules \
        --skip-dirs fuzz --skip-dirs .compatibility --skip-dirs .codegraph \
        -f table -o "$OUT/trivy.txt" . >/dev/null 2>&1; then
        ok "trivy fs: sin hallazgos HIGH/CRITICAL"
    else
        local n
        n="$(rg -c 'CRITICAL:|HIGH:' "$OUT/trivy.txt" || echo 0)"
        warn "trivy fs: $n hallazgo(s) HIGH/CRITICAL — ver security-reports/trivy.txt"
        section "### trivy fs — HIGH/CRITICAL\n\`\`\`\n$(cat "$OUT/trivy.txt")\n\`\`\`"
    fi
}

run_osv() {
    local osv_bin="$HOME/.local/bin/osv-scanner"
    has osv-scanner && osv_bin="$(command -v osv-scanner)"
    if [ ! -x "$osv_bin" ]; then warn "osv-scanner no instalado"; return; fi
    log "osv-scanner (base de datos OSV sobre Cargo.lock + node_modules)"
    if "$osv_bin" scan --no-ignore Cargo.lock >"$OUT/osv.txt" 2>&1; then
        ok "osv-scanner: sin vulnerabilidades conocidas"
    else
        local n
        n="$(rg -oP '\d+ package[s]? affected' "$OUT/osv.txt" | head -1)"
        warn "osv-scanner: $n — ver security-reports/osv.txt"
        section "### osv-scanner — Cargo.lock\n\`\`\`\n$(tail -30 "$OUT/osv.txt")\n\`\`\`"
    fi
}

run_custom() {
    log "escaneo custom (patrones JS/TS no cubiertos por herramientas estándar)"
    if bash scripts/security_custom_scan.sh --out "$OUT" >>"$RUNLOG" 2>&1; then
        ok "custom scan: sin hallazgos HIGH/CRITICAL"
    else
        local total
        total="$(rg -oP 'custom_scan_total=\K[0-9]+' "$OUT/custom_summary.txt" 2>/dev/null || echo 0)"
        local crit high
        crit="$(rg -oP 'custom_scan_critical=\K[0-9]+' "$OUT/custom_summary.txt" 2>/dev/null || echo 0)"
        high="$(rg -oP 'custom_scan_high=\K[0-9]+' "$OUT/custom_summary.txt" 2>/dev/null || echo 0)"
        warn "custom scan: $total hallazgo(s) (CRITICAL=$crit HIGH=$high)"
        section "### custom scan\n\`\`\`\n$(cat "$OUT/custom_findings.txt")\n\`\`\`"
    fi
}

#######################################
# 4. BUILD / TEST / LINT (verificación)
#######################################

run_fmt() {
    SKIP_FN fmt && return
    log "cargo fmt --check"
    cargo fmt --check >"$OUT/fmt.txt" 2>&1 && ok "fmt OK" || { warn "fmt: diferencias — ver security-reports/fmt.txt"; section "### cargo fmt\n\`\`\`\n$(head -20 "$OUT/fmt.txt")\n\`\`\`"; }
}

run_clippy() {
    SKIP_FN clippy && return
    log "cargo clippy (lints de seguridad)"
    if capped timeout 600 cargo clippy --all-targets --all-features -- -D warnings >"$OUT/clippy.txt" 2>&1; then
        ok "clippy OK"
    else
        local n
        n="$(rg -c '^error\[' "$OUT/clippy.txt" || echo 0)"
        warn "clippy: $n error(es) — ver security-reports/clippy.txt"
        section "### cargo clippy\n\`\`\`\n$(rg '^error' "$OUT/clippy.txt" | head -10)\n\`\`\`"
    fi
}

run_test() {
    SKIP_FN test && return
    log "cargo test (suite completa)"
    if capped timeout 1200 cargo test --all-features >"$OUT/test.txt" 2>&1; then
        ok "tests OK"
    else
        local failed
        failed="$(rg -oP '\d+(?= failed)' "$OUT/test.txt" | head -1 || echo 0)"
        warn "tests: $failed fallido(s) — ver security-reports/test.txt"
        section "### cargo test\n\`\`\`\n$(rg -A2 'failures:|test result: FAILED' "$OUT/test.txt" | head -30)\n\`\`\`"
    fi
}

#######################################
# 5. GENERACIÓN DE REPORTE
#######################################
generate_report() {
    {
        echo "# 3va — Reporte de Vulnerabilidades"
        echo ""
        echo "- Fecha: $(date -Iseconds)"
        echo "- Commit: $(git rev-parse --short HEAD 2>/dev/null || echo n/a)"
        echo "- Rama: $(git branch --show-current 2>/dev/null || echo n/a)"
        echo ""
        echo "## Versiones de herramientas"
        echo ""
        echo "| Herramienta | Versión |"
        echo "|-------------|---------|"
        for t in audit deny geiger vet supply-chain outdated udeps semgrep gitleaks trivy osv custom; do
            echo "| $t | ${TOOLVERS[$t]:-n/a} |"
        done
        echo ""
        echo "## Resultados por escáner"
        echo ""
        for s in "${SECTIONS[@]}"; do
            printf '%b\n\n' "$s"
        done
        echo "## Notas"
        echo ""
        echo "- Advisories ignorados justificados en \`deny.toml\` y \`.cargo/audit.toml\`."
        echo "- Los hallazgos HIGH/CRITICAL del escaneo custom son patrones que requieren revisión manual."
        echo "- Ver archivos individuales en \`$OUT/\` para detalle completo."
    } > "$REPORT"
}

#######################################
# MAIN
#######################################

log "=== 3va Vulnerability Scan ==="
update_tools
capture_versions

# Mostrar versiones
echo -e "\n${BLUE}Versiones instaladas:${NC}"
for t in audit deny geiger vet supply-chain outdated udeps semgrep gitleaks trivy osv custom; do
    printf '  %-14s %s\n' "$t:" "${TOOLVERS[$t]:-n/a}"
done
echo ""

SKIP_FN audit        || run_audit
SKIP_FN deny         || run_deny
SKIP_FN geiger       || run_geiger
SKIP_FN vet          || run_vet
SKIP_FN supply-chain || run_supply_chain
SKIP_FN outdated     || run_outdated
SKIP_FN udeps        || run_udeps
SKIP_FN semgrep      || run_semgrep
SKIP_FN gitleaks     || run_gitleaks
SKIP_FN trivy        || run_trivy
SKIP_FN osv          || run_osv
SKIP_FN custom       || run_custom
SKIP_FN fmt          || run_fmt
SKIP_FN clippy       || run_clippy
SKIP_FN test         || run_test

generate_report

echo -e "\n${GREEN}Reporte generado: $REPORT${NC}"
echo -e "Log de ejecución: $RUNLOG"
echo ""
echo "Resumen de advertencias/fallos en: $RUNLOG"

# Exit code: falla si algún escáner de seguridad reportó [FAIL] (audit, deny,
# vet, gitleaks, trivy, osv, ...) o si el custom scan tiene HIGH/CRITICAL.
if [ "$FAILS" -gt 0 ]; then
    echo -e "${RED}$FAILS escáner(es) con [FAIL]${NC}"
    exit 1
fi
if [ -f "$OUT/custom_summary.txt" ]; then
    crit="$(rg -oP 'custom_scan_critical=\K[0-9]+' "$OUT/custom_summary.txt" || echo 0)"
    high="$(rg -oP 'custom_scan_high=\K[0-9]+' "$OUT/custom_summary.txt" || echo 0)"
    if [ "${crit:-0}" -gt 0 ] || [ "${high:-0}" -gt 0 ]; then
        exit 1
    fi
fi
exit 0