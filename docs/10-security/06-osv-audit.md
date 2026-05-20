# 06 - AUDITORÍA DE VULNERABILIDADES (OSV)

## 6.1 Visión General

`3va audit` detecta vulnerabilidades conocidas en las dependencias instaladas consultando la [Open Source Vulnerabilities API (OSV)](https://osv.dev). OSV agrega datos de NVD, GitHub Advisory Database (GHSA), RustSec, PyPI Advisory y otros, por lo que una sola consulta cubre múltiples fuentes autoritativas.

El auditor opera en dos fases secuenciales:

| Fase | Módulo | Qué detecta |
|------|--------|-------------|
| 1 | `MalwareScanner` | Patrones de malware en el código extraído de `node_modules/` |
| 2 | `auditor::run_audit` | CVEs, GHSAs y advisories conocidos para cada `paquete@versión` |

---

## 6.2 Arquitectura del Auditor OSV

### 6.2.1 Flujo de datos

```
3va-lock.json
     │
     ▼
Lista de (nombre, versión)
     │
     ├── Hit de caché (~/.cache/3va/audit/) ──► resultado en memoria
     │
     └── Miss de caché
              │
              ▼
     OSV Batch API (POST /v1/querybatch)
     hasta 100 paquetes por petición
              │
              ▼
     Guardar en caché + resultado en memoria
              │
              ▼
     Parsear severidad CVSS v3 / etiqueta GHSA
              │
              ▼
     AuditReport { findings, critical_count, high_count, ... }
```

### 6.2.2 Elección de arquitectura: API + caché vs base de datos local

Se eligió **API-first con caché por paquete** frente a descargar la base de datos OSV completa (~600 MB comprimida) por las siguientes razones:

- Los datos siempre son los más recientes sin ningún paso manual.
- No requiere daemon ni scheduler para mantener la DB actualizada.
- Solo se descarga información de los paquetes realmente instalados.
- La caché por `paquete@versión` es granular: una nueva instalación solo fetcha lo nuevo.

---

## 6.3 API OSV: Consulta Batch

**Endpoint:** `POST https://api.osv.dev/v1/querybatch`

**Petición:**
```json
{
  "queries": [
    {
      "version": "4.17.20",
      "package": { "name": "lodash", "ecosystem": "npm" }
    },
    {
      "version": "1.7.9",
      "package": { "name": "axios", "ecosystem": "npm" }
    }
  ]
}
```

**Respuesta:**
```json
{
  "results": [
    {
      "vulns": [
        {
          "id": "GHSA-35jh-r3h4-6jhm",
          "summary": "Prototype Pollution in lodash",
          "severity": [
            { "type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:H/A:N" }
          ],
          "affected": [{
            "ranges": [{ "type": "SEMVER", "events": [{"introduced":"0"},{"fixed":"4.17.21"}] }],
            "database_specific": { "severity": "HIGH" }
          }],
          "references": [
            { "type": "ADVISORY", "url": "https://github.com/advisories/GHSA-35jh-r3h4-6jhm" }
          ]
        }
      ]
    },
    { "vulns": [] }
  ]
}
```

El array `results` tiene la misma longitud y orden que `queries`, lo que permite correlacionar resultados en O(1).

---

## 6.4 Caché Local

### 6.4.1 Ubicación

```
~/.cache/3va/audit/
```

### 6.4.2 Formato de cada entrada

Archivo: `<pkg_sanitizado>@<version>.json`

- Los paquetes con scope se sanitizan: `@scope/name` → `scope__name@1.0.0.json`

```json
{
  "fetched_at_unix": 1716235200,
  "vulns": [ ... ]
}
```

### 6.4.3 TTL y refresco

| Situación | Comportamiento |
|-----------|---------------|
| Entrada < 24 h de antigüedad | Usada directamente (0 peticiones a OSV) |
| Entrada ≥ 24 h | Re-fetch automático en background del comando |
| `--update-cache` pasado | TTL ignorado, todos los paquetes re-fetched |
| Red no disponible y caché existe | Caché stale usada con advertencia al usuario |
| Red no disponible y sin caché | Paquete omitido del análisis (warning visible) |

El comando **nunca falla con error** por problemas de conectividad.

---

## 6.5 Cálculo de Severidad

La severidad se determina en orden de preferencia:

1. **CVSS v3.1 vector** — se calcula la base score según la fórmula NVD completa.
2. **CVSS v2 numeric score** — para advisories más antiguos.
3. **`database_specific.severity`** — etiqueta string de GitHub Advisory (`CRITICAL`, `HIGH`, `MODERATE`, `LOW`).
4. **`affected[].database_specific.severity`** — mismo campo a nivel de paquete afectado.

### Umbrales CVSS v3

| Score | Severidad |
|-------|-----------|
| 9.0 – 10.0 | **CRITICAL** |
| 7.0 – 8.9  | **HIGH** |
| 4.0 – 6.9  | **MEDIUM** |
| 0.1 – 3.9  | **LOW** |
| 0.0        | UNKNOWN |

---

## 6.6 Manejo de Errores de Red y Rate Limiting

```
petición → HTTP 429 → esperar 5s → reintentar una vez
                                         │
                              ┌──────────┴──────────┐
                           éxito                  fallo
                              │                      │
                         guardar caché         caché stale
                                                (con warning)
```

- Un solo retry automático tras HTTP 429 (rate limit).
- Todos los errores de red (timeout, DNS, TLS) son recuperables: se usa la caché stale.
- Los errores se reportan como warnings, nunca como errores fatales.

---

## 6.7 Privacidad

Solo se envía a la API OSV:
- Nombre del paquete
- Versión exacta
- Ecosistema (`"npm"`)

**No se envía:** rutas de archivos, contenido del código, nombre del proyecto, variables de entorno ni ningún otro metadato del sistema.

---

## 6.8 Uso en CI/CD

```yaml
# GitHub Actions — bloquear merge si hay HIGH/CRITICAL
- name: Security audit
  run: 3va audit --deny
```

```bash
# Pipeline local
3va audit --deny && echo "OK" || exit 1
```

El flag `--deny` hace que el comando salga con código ≠ 0 si y solo si se encuentra al menos una vulnerabilidad CRITICAL o HIGH. Las vulnerabilidades MEDIUM y LOW producen una advertencia pero no bloquean el pipeline.

---

## 6.9 Relación con el Scanner de Malware

Las dos fases son complementarias, no redundantes:

| | Malware Scanner (Fase 1) | Auditor OSV (Fase 2) |
|---|---|---|
| **Fuente de verdad** | Heurísticas + patrones propios | Base de datos pública OSV |
| **Qué detecta** | Código malicioso no reportado, obfuscación, exfiltración | CVEs y advisories conocidos y publicados |
| **Requiere red** | No | Sí (con caché offline) |
| **Falsos positivos** | Posibles (heurístico) | Bajos (datos autoritativos) |
| **Cobertura** | 0-day y malware nuevo | Vulnerabilidades catalogadas |
