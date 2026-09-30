#!/usr/bin/env bash
# security_custom_scan.sh
#
# Escaneo custom de vulnerabilidades NO cubiertas por las herramientas
# estándar (cargo-audit, semgrep, gitleaks, trivy, osv-scanner):
#
#   1. Patrones peligrosos en código JS/TS (repo + examples/)
#      - eval() / new Function() / indirect Function construction
#      - child_process: exec/execSync/spawn/execFile/fork
#      - Command/shell injection (interpolación de variables en exec)
#      - XSS sinks: innerHTML, outerHTML, document.write, dangerouslySetInnerHTML
#      - Path traversal: fs + input de usuario sin sanitizar
#      - Prototype pollution: __proto__, constructor.prototype, merge helpers
#      - Cripto inseguro: Math.random() para secretos, md5/sha1
#      - Header/CRLF injection: "\r\n" en respuestas HTTP
#   2. Secretos hardcodeados (fuera de fixtures y docs permitidas)
#   3. Scripts peligrosos en package.json (postinstall/preinstall con shell)
#   4. Permisos 3va excesivos en config (allow-net "*", allow-all)
#   5. Dependencias JS desactualizadas/vulnerables (npm audit + osv-scanner)
#
# Uso:
#   scripts/security_custom_scan.sh [--out DIR]
#
# Escribe:
#   $OUT/custom_findings.txt  -> hallazgos en texto plano
#   $OUT/custom_summary.txt   -> conteo por categoría
#   $OUT/custom_report.md     -> reporte markdown
#
# Exit code: 0 si no hay hallazgos CRITICAL/HIGH, 1 en caso contrario.

set -uo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

OUT="security-reports"
if [ "${1:-}" = "--out" ]; then
    OUT="$2"
fi
mkdir -p "$OUT"

FINDINGS="$OUT/custom_findings.txt"
SUMMARY="$OUT/custom_summary.txt"
REPORT="$OUT/custom_report.md"

: > "$FINDINGS"
: > "$SUMMARY"

declare -A COUNTS
COUNTS[CRITICAL]=0
COUNTS[HIGH]=0
COUNTS[MEDIUM]=0
COUNTS[LOW]=0
COUNTS[DEMO]=0

# Categorías de PATRONES DE CÓDIGO que en examples/ son deliberadas:
# demuestran que 3va bloquea la capacidad (deny-by-default), no son un bug.
# Las categorías de config/dependencias (over-broad-permission,
# npm-dependency, osv-node-modules, malicious-install-script) SI son reales
# incluso dentro de examples/ y no se marcan como demo.
DEMO_CATEGORIES='child-process|command-injection|xss-sink|dangerous-eval|path-traversal|prototype-pollution|weak-crypto|crlf-injection|hardcoded-secret'

add_finding() {
    local sev="$1" cat="$2" file="$3" line="$4" msg="$5"
    if [[ "$file" =~ ^\.?/?examples/ ]] && [[ "$cat" =~ ^(${DEMO_CATEGORIES})$ ]]; then
        COUNTS[DEMO]=$(( ${COUNTS[DEMO]} + 1 ))
        printf '[DEMO] [%s] %s:%s %s (deliberado: demo deny-by-default de 3va)\n' \
            "$cat" "$file" "$line" "$msg" >> "$FINDINGS"
        return
    fi
    COUNTS[$sev]=$(( ${COUNTS[$sev]} + 1 ))
    printf '[%s] [%s] %s:%s %s\n' "$sev" "$cat" "$file" "$line" "$msg" >> "$FINDINGS"
}

scan_pattern() {
    local sev="$1" cat="$2" pattern="$3" desc="$4"
    local include="${5:-}"
    while IFS=: read -r file line rest; do
        [ -z "$file" ] && continue
        add_finding "$sev" "$cat" "$file" "$line" "$desc"
    done < <(rg -n --no-heading -g '*.js' -g '*.ts' -g '*.jsx' -g '*.tsx' -g '*.mjs' -g '*.cjs' \
        --glob "!**/node_modules/**" --glob "!**/target/**" --glob "!**/dist/**" --glob "!**/vendor/**" \
        --glob "!**/.compatibility/**" --glob "!**/security-reports/**" \
        "$pattern" $include 2>/dev/null || true)
}

#######################################
# 1. Patrones peligrosos JS/TS
#######################################

echo "== Escaneando patrones JS/TS peligrosos ==" >&2

# 1a. eval / Function dinámico
scan_pattern HIGH "dangerous-eval" '(\beval\s*\(|\bnew\s+Function\s*\(|[^.\w]Function\s*\()' \
    "Uso de eval()/Function() dinamico — riesgo de RCE si hay input de usuario" \
    "crates examples bench scripts tests"

# 1b. child_process (no por sí mismo peligroso, pero requiere revisión)
scan_pattern MEDIUM "child-process" '\brequire\s*\(\s*["'"'"']child_process["'"'"']\s*\)' \
    "child_process usado — verificar que no reciba input sin sanitizar" \
    "crates examples bench scripts tests"

# 1c. exec/execSync con interpolación -> command injection
while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    ln="${rest%%:*}"
    body="${rest#*:}"
    if echo "$body" | rg -q '[$][{]|[+]'; then
        add_finding CRITICAL "command-injection" "$file" "$ln" \
            "exec/execSync con interpolación de variable — POSIBLE COMMAND INJECTION"
    else
        add_finding MEDIUM "command-injection" "$file" "$ln" \
            "exec/execSync con string estático (revisar igualmente)"
    fi
done < <(rg -n --no-heading -g '*.js' -g '*.ts' -g '*.jsx' -g '*.tsx' -g '*.mjs' -g '*.cjs' \
    --glob "!**/node_modules/**" --glob "!**/target/**" --glob "!**/dist/**" --glob "!**/vendor/**" \
    '\b(exec|execSync|execFile|execFileSync|spawn|spawnSync|fork)\s*\(' \
    crates examples bench scripts tests 2>/dev/null || true)

# 1d. XSS sinks (relevante para código web/bundler/server)
scan_pattern HIGH "xss-sink" '\.\s*(innerHTML|outerHTML|insertAdjacentHTML)\s*=|document\.write\s*\(|dangerouslySetInnerHTML' \
    "Sink XSS: asignación de HTML desde datos — riesgo de XSS" \
    "examples bench tests"

# 1e. Path traversal: fs con path derivado de input
scan_pattern HIGH "path-traversal" '(readFileSync|readFile|createReadStream|writeFileSync|writeFile|existsSync)\s*\(\s*[^)]*(req|params|query|body|url|input)' \
    "Acceso a filesystem con input de usuario sin validar — POSIBLE PATH TRAVERSAL" \
    "examples bench tests"

# 1f. Prototype pollution
scan_pattern HIGH "prototype-pollution" '(__proto__|constructor\.prototype|\[\s*["'"'"']__proto__["'"'"']\s*\])' \
    "Prototype pollution — asignación a __proto__" \
    "examples bench"

# 1g. Cripto inseguro en JS
scan_pattern MEDIUM "weak-crypto" '\bMath\.random\s*\(\s*\)' \
    "Math.random() usado — NO es criptográficamente seguro (usar crypto.getRandomValues)" \
    "examples bench"
scan_pattern MEDIUM "weak-crypto" '(createHash\s*\(\s*["'"'"'](md5|sha1)["'"'"']\s*\))' \
    "Hash md5/sha1 — débil para firmas/autenticación (usar sha256+)" \
    "examples bench"

# 1h. Header/CRLF injection
scan_pattern HIGH "crlf-injection" '(writeHead|setHeader)\s*\([^)]*["'"'"'][^"'"'"']*[\r\n]' \
    "Posible CRLF/header injection en respuesta HTTP" \
    "examples bench"

#######################################
# 2. Secretos hardcodeados (JS/TS fuera de fixtures)
#######################################

echo "== Escaneando secretos hardcodeados ==" >&2

SECRET_PATTERNS=(
    '(api[_-]?key|apikey)\s*[:=]\s*["'"'"'][A-Za-z0-9_\-]{12,}["'"'"']'
    'password\s*[:=]\s*["'"'"'][^"'"'"']{8,}["'"'"']'
    '(secret|token|bearer)\s*[:=]\s*["'"'"'][A-Za-z0-9_\-\.]{16,}["'"'"']'
    'AKIA[0-9A-Z]{16}'
    'ghp_[A-Za-z0-9]{36,}'
    'sk-(live|test)-[A-Za-z0-9]{20,}'
)

for pat in "${SECRET_PATTERNS[@]}"; do
    while IFS=: read -r file line rest; do
        [ -z "$file" ] && continue
        # Omitir fixtures/documentación intencionales
        case "$file" in
            *docs/*|*.md|*tests/*|*.test.*|*.spec.*|*fuzz/*|*.snap.json) continue ;;
        esac
        add_finding CRITICAL "hardcoded-secret" "$file" "$line" \
            "Posible secreto hardcodeado (patrón: $pat)"
    done < <(rg -ni --no-heading -g '*.js' -g '*.ts' -g '*.jsx' -g '*.tsx' -g '*.mjs' -g '*.cjs' \
        --glob "!**/node_modules/**" --glob "!**/target/**" --glob "!**/dist/**" --glob "!**/vendor/**" \
        --glob "!**/.compatibility/**" \
        "$pat" crates examples bench scripts tests 2>/dev/null || true)
done

#######################################
# 3. Scripts peligrosos en package.json
#######################################

echo "== Escaneando scripts peligrosos en package.json ==" >&2

while IFS= read -r pkg; do
    [ -z "$pkg" ] && continue
    while IFS= read -r hook; do
        if rg -q '(preinstall|postinstall|prepare|prepublish)' <<< "$hook"; then
            hook_line="$(rg -n "$(printf '%s' "$hook" | sed 's/[&/]/\\&/g')" "$pkg" | head -1 | cut -d: -f1)"
            add_finding HIGH "malicious-install-script" "$pkg" "${hook_line:-1}" \
                "Hook de instalación ($hook) ejecuta código arbitrario en install"
        fi
    done < <(rg -o '"(preinstall|postinstall|prepare|prepublish|install)"\s*:\s*"[^"]*"' "$pkg" 2>/dev/null || true)
done < <(find examples bench -name package.json -not -path "*/node_modules/*" 2>/dev/null || true)

#######################################
# 4. Permisos 3va excesivos
#######################################

echo "== Escaneando permisos 3va excesivos ==" >&2

while IFS=: read -r file line rest; do
    [ -z "$file" ] && continue
    add_finding HIGH "over-broad-permission" "$file" "$line" \
        "Permiso 3va excesivo: allow-net/allow-read/allow-all con comodín '*'"
done < <(rg -n --no-heading -g 'package.json' -g '*.json' \
    --glob "!**/node_modules/**" --glob "!**/target/**" \
    '("allow-net"\s*:\s*\[?\s*["'"'"']\*["'"'"']|"allow-read"\s*:\s*\[?\s*["'"'"']\.?/?\.?\*|--allow-all)' \
    . 2>/dev/null || true)

#######################################
# 5. Dependencias JS (npm audit + osv-scanner)
#######################################

echo "== Escaneando dependencias JS ==" >&2

# 5a. npm audit sobre cada example con package.json (lockfile temporal en /tmp)
TMP_AUDIT="$(mktemp -d)"
npm_audit_report() {
    local pkg="$1"
    local dir
    dir="$(dirname "$pkg")"
    local name
    name="$(basename "$dir")"
    local tmpdir="$TMP_AUDIT/$name"
    mkdir -p "$tmpdir"
    cp "$pkg" "$tmpdir/package.json"
    if (cd "$tmpdir" && npm install --package-lock-only --ignore-scripts --no-audit --no-fund >/dev/null 2>&1); then
        if (cd "$tmpdir" && npm audit --json >/dev/null 2>&1); then
            : # sin vulnerabilidades
        else
            local json_vulns
            json_vulns="$(cd "$tmpdir" && npm audit --json 2>/dev/null || true)"
            local total
            total="$(echo "$json_vulns" | rg -oP '"total":\s*\K[0-9]+' | head -1)"
            total="${total:-0}"
            local high
            high="$(echo "$json_vulns" | rg -oP '"high":\s*\K[0-9]+' | head -1)"
            high="${high:-0}"
            if [ "$total" -gt 0 ]; then
                add_finding MEDIUM "npm-dependency" "$pkg" "1" \
                    "npm audit: $total vulnerabilidades (high=$high) en dependencias del example"
            fi
        fi
    else
        add_finding LOW "npm-dependency" "$pkg" "1" \
            "No se pudo generar lockfile para npm audit (deps posiblemente no instalables en npm)"
    fi
}

for pkg in $(find examples bench -name package.json -not -path "*/node_modules/*" 2>/dev/null); do
    npm_audit_report "$pkg"
done
rm -rf "$TMP_AUDIT"

# 5b. osv-scanner sobre node_modules presentes
if command -v osv-scanner >/dev/null 2>&1 || [ -x "$HOME/.local/bin/osv-scanner" ]; then
    OSV="$HOME/.local/bin/osv-scanner"
    command -v osv-scanner >/dev/null 2>&1 && OSV="$(command -v osv-scanner)"
    for nm in $(find . -type d -name node_modules -not -path "*/target/*" 2>/dev/null | head -5); do
        if [ -n "$(ls -A "$nm" 2>/dev/null)" ]; then
            out="$("$OSV" scan --no-ignore "$nm" 2>&1 || true)"
            if echo "$out" | rg -q 'vulnerabilit'; then
                total="$(echo "$out" | rg -oP '\d+ package[s]? affected' | head -1)"
                add_finding MEDIUM "osv-node-modules" "$nm" "1" \
                    "osv-scanner: $total en dependencias node_modules"
            fi
        fi
    done
fi

#######################################
# Resumen
#######################################

total=$(( ${COUNTS[CRITICAL]} + ${COUNTS[HIGH]} + ${COUNTS[MEDIUM]} + ${COUNTS[LOW]} ))

{
    echo "custom_scan_total=$total"
    echo "custom_scan_critical=${COUNTS[CRITICAL]}"
    echo "custom_scan_high=${COUNTS[HIGH]}"
    echo "custom_scan_medium=${COUNTS[MEDIUM]}"
    echo "custom_scan_low=${COUNTS[LOW]}"
    echo "custom_scan_demo=${COUNTS[DEMO]}"
} > "$SUMMARY"

cat > "$REPORT" <<EOF
# Custom Security Scan (patrones no cubiertos por herramientas estándar)

Fecha: $(date -Iseconds)
Total hallazgos reales: $total (CRITICAL=${COUNTS[CRITICAL]} HIGH=${COUNTS[HIGH]} MEDIUM=${COUNTS[MEDIUM]} LOW=${COUNTS[LOW]})
Hallazgos DEMO (examples/ deliberados, no bloquean): ${COUNTS[DEMO]}

> Los hallazgos marcados como DEMO viven en \`examples/\` y son deliberados:
> demuestran el modelo deny-by-default de 3va (el runtime los bloquea).
> No son vulnerabilidades del runtime ni bugs de los ejemplos.

## Categorías detectadas
| Severidad | Categoría | Descripción |
|-----------|-----------|-------------|
| CRITICAL | command-injection | exec/execSync/spawn con interpolación de variables |
| CRITICAL | hardcoded-secret | Secretos (API keys, tokens, passwords) en código |
| HIGH | dangerous-eval | eval()/new Function() dinámico |
| HIGH | xss-sink | innerHTML/document.write/dangerouslySetInnerHTML |
| HIGH | path-traversal | fs con input de usuario sin validar |
| HIGH | prototype-pollution | Asignación a __proto__ |
| HIGH | crlf-injection | CRLF en headers HTTP |
| HIGH | malicious-install-script | Hooks preinstall/postinstall en package.json |
| HIGH | over-broad-permission | Permisos 3va con comodín |
| MEDIUM | child-process | child_process usado |
| MEDIUM | weak-crypto | Math.random() / md5/sha1 |
| MEDIUM | npm-dependency | Vulnerabilidades npm en examples |
| MEDIUM | osv-node-modules | Vulnerabilidades OSV en node_modules |
| LOW | npm-dependency | Lockfile no generable |
| DEMO | (todas las de código en examples/) | Patrón deliberado del demo deny-by-default |

## Hallazgos
\`\`\`
$(cat "$FINDINGS")
\`\`\`
EOF

echo "Custom scan completado: $total reales + ${COUNTS[DEMO]} DEMO" >&2
exit $(( ${COUNTS[CRITICAL]} > 0 || ${COUNTS[HIGH]} > 0 ? 1 : 0 ))