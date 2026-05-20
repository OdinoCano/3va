# 02 - COMANDOS DISPONIBLES

## 2.1 Catálogo de Comandos

Este documento describe todos los comandos disponibles en la CLI de 3va.

---

## 2.2 Comandos de Ejecución

### 2.2.1 `run`

Ejecuta un archivo JavaScript o TypeScript en un entorno sandboxed. Los permisos son denegados por defecto.

**Firma:**
```
3va run [OPTIONS] <FILE>
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `FILE` | `path` (requerido) | Ruta al archivo `.js` o `.ts` a ejecutar |

**Opciones:**
| Opción | Tipo | Descripción |
|--------|------|-------------|
| `--allow-read=<paths>` | `path[]` | Rutas con permiso de lectura |
| `--allow-write=<paths>` | `path[]` | Rutas con permiso de escritura |
| `--allow-net=<hosts>` | `string[]` | Hosts con permiso de red |
| `--allow-env` | `bool` | Acceso a variables de entorno |
| `--allow-child-process` | `bool` | Permiso para lanzar procesos hijos |

**Comportamiento:**
1. Carga y valida el archivo de entrada.
2. Inicializa `PermissionState` con los permisos concedidos.
3. Transpila TypeScript automáticamente si la extensión es `.ts`.
4. Ejecuta el archivo en el motor QuickJS.
5. Corre el event loop hasta completar timers y callbacks pendientes.

**Ejemplos:**
```bash
3va run app.ts
3va run app.ts --allow-read=/app/data
3va run app.ts --allow-net=api.example.com
3va run app.ts --allow-read=/config --allow-net=api.example.com --allow-env
```

---

## 2.3 Comandos de Package Manager

### 2.3.1 `install`

Instala un paquete desde un registry. Requiere `--allow-net` con el host del registry.

**Firma:**
```
3va install [<PACKAGE>[@<VERSION>]] --allow-net=<registry-host>
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `PACKAGE[@VERSION]` | `string` (opcional) | Paquete a instalar. Si se omite, instala dependencias del `package.json`. |

**Opciones:**
| Opción | Tipo | Descripción |
|--------|------|-------------|
| `--allow-net=<host>` | `string` | **Requerido.** Host del registry. Define qué registry se usa. |

**El registry se deriva del host:**
| `--allow-net` | Registry usado |
|---------------|---------------|
| `registry.npmjs.org` | npm |
| `registry.yarnpkg.com` | Yarn |
| `jsr.io` | JSR (solo paquetes con scope) |
| Cualquier otro host | Registry npm-compatible custom |

**Ejemplos:**
```bash
# Desde npm
3va install axios --allow-net=registry.npmjs.org
3va install axios@1.7.2 --allow-net=registry.npmjs.org

# Desde Yarn
3va install react --allow-net=registry.yarnpkg.com

# Desde JSR (requiere @scope/name)
3va install @std/path --allow-net=jsr.io
3va install @std/path@0.196.0 --allow-net=jsr.io

# Sin --allow-net: error explicativo
3va install axios
# ✗ Network access denied.
#   3va install axios --allow-net=registry.npmjs.org
```

---

### 2.3.2 `reinstall`

Fuerza la reinstalación de un paquete aunque ya esté instalado.

**Firma:**
```
3va reinstall <PACKAGE>[@<VERSION>] --allow-net=<registry-host>
```

**Ejemplos:**
```bash
3va reinstall axios --allow-net=registry.npmjs.org
3va reinstall @std/path@0.196.0 --allow-net=jsr.io
```

---

### 2.3.3 `update`

Actualiza paquetes instalados a su última versión, respetando el registry de origen registrado en `3va-lock.json`.

**Firma:**
```
3va update [<PACKAGE>...] --allow-net=<hosts>
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `PACKAGE` | `string[]` (opcional) | Paquetes a actualizar. Si se omite, actualiza todos. |

**Opciones:**
| Opción | Tipo | Descripción |
|--------|------|-------------|
| `--allow-net=<hosts>` | `string` | Hosts autorizados. Deben cubrir todos los registries necesarios. Puede ser una lista separada por comas. |

**Comportamiento:**
1. Lee `3va-lock.json` y el campo `registry` de cada dependencia.
2. Agrupa paquetes por registry.
3. Verifica que `--allow-net` incluya todos los hosts necesarios.
4. Si falta algún host, muestra el comando exacto a ejecutar.
5. Actualiza cada paquete desde su registry original.

**Si falta `--allow-net`:**
```bash
3va update
# ✗ Update requires network access to:
#
#     registry.npmjs.org        (axios, express)
#     jsr.io                    (@std/path)
#
# Run: 3va update --allow-net=registry.npmjs.org,jsr.io
```

**Ejemplos:**
```bash
# Actualizar todo
3va update --allow-net=registry.npmjs.org,jsr.io

# Actualizar un paquete específico
3va update axios --allow-net=registry.npmjs.org

# Actualizar varios de distintos registries
3va update axios @std/path --allow-net=registry.npmjs.org,jsr.io
```

---

## 2.4 Comandos de Testing

### 2.4.1 `test`

Ejecuta la suite de pruebas del proyecto.

**Firma:**
```
3va test [<PATHS>...]
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `PATHS` | `path[]` (opcional) | Archivos o directorios. Por defecto, busca en `.` |

**Extensiones detectadas:** `.test.js`, `.test.ts`, `.spec.js`, `.spec.ts`

**Ejemplos:**
```bash
3va test
3va test tests/
3va test tests/auth.test.ts
```

---

## 2.5 Comandos de Build

### 2.5.1 `bundle`

Empaqueta una aplicación desde un punto de entrada.

**Firma:**
```
3va bundle <INPUT> [--output <OUTPUT>]
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `INPUT` | `string` (requerido) | Archivo de entrada |

**Opciones:**
| Opción | Tipo | Default | Descripción |
|--------|------|---------|-------------|
| `--output` / `-o` | `string` | `dist/bundle.js` | Ruta del bundle generado |

**Ejemplos:**
```bash
3va bundle src/index.ts
3va bundle src/index.ts --output dist/app.js
```

---

## 2.6 Comandos de Diagnóstico e Información

### 2.6.1 `audit`

Audita las dependencias instaladas en **dos fases**:

1. **Análisis estático de malware** — escanea el código extraído en `node_modules/` buscando patrones maliciosos conocidos (exfiltración de datos, inyección de comandos, etc.).
2. **Vulnerabilidades conocidas (OSV)** — consulta la [OSV Batch API](https://osv.dev) por cada paquete@versión en `3va-lock.json` y reporta CVEs, GHSAs y entradas de RustSec/NVD.

**Firma:**
```
3va audit [--deny] [--update-cache]
```

**Opciones:**
| Opción | Descripción |
|--------|-------------|
| `--deny` | Sale con código de error ≠ 0 si se detecta alguna vulnerabilidad de severidad **CRITICAL** o **HIGH**. Recomendado en pipelines CI/CD. |
| `--update-cache` | Ignora la caché local (TTL 24 h) y descarga datos frescos de la API OSV para todos los paquetes. |

**Comportamiento:**
- Lee `3va-lock.json` para obtener la lista de paquetes instalados con sus versiones exactas.
- Si `3va-lock.json` no existe, recorre `node_modules/` como fallback.
- Consulta `https://api.osv.dev/v1/querybatch` en lotes de hasta 100 paquetes por petición HTTP.
- Cachea cada resultado en `~/.cache/3va/audit/<pkg>@<version>.json` durante 24 horas.
- Si la API no está disponible, usa la caché stale y avisa al usuario — el comando nunca falla por problemas de conectividad.
- Solo envía `{ name, version, ecosystem }` a OSV. No se transmiten rutas de archivos ni código fuente.

**Severidad OSV:**
| Rango CVSS v3 | Etiqueta |
|---------------|---------|
| 9.0 – 10.0 | CRITICAL |
| 7.0 – 8.9 | HIGH |
| 4.0 – 6.9 | MEDIUM |
| 0.1 – 3.9 | LOW |

La severidad se calcula en este orden de preferencia: CVSS v3 vector → CVSS v2 score → etiqueta `database_specific.severity` (formato GitHub Advisory).

**Caché local:**
```
~/.cache/3va/audit/
  lodash@4.17.20.json
  axios@1.7.9.json
  @scope__pkg@2.0.0.json    ← @ y / sanitizados
```

**Ejemplos:**
```bash
# Auditoría completa (malware + CVEs)
3va audit

# CI/CD: falla si hay HIGH o CRITICAL
3va audit --deny

# Forzar datos frescos de OSV
3va audit --update-cache

# Ejemplo de salida con vulnerabilidad
# === Phase 2: Known Vulnerabilities (OSV) ===
#
#   HIGH lodash@4.17.20 — 1 issue(s)
#     [HIGH] GHSA-35jh-r3h4-6jhm — Prototype Pollution in lodash
#            Fix: upgrade to 4.17.21
#            See: https://github.com/advisories/GHSA-35jh-r3h4-6jhm
#
#   Packages scanned      : 12
#   Packages with vulns   : 1
#   Total vulnerabilities : 1
#   High                  : 1
#
# ! 1 CRITICAL/HIGH issue(s) found. Review the findings above.
# Use '3va audit --deny' in CI/CD pipelines to enforce a hard block.
```

### 2.6.2 `doctor`

Verifica la salud del runtime y del entorno.

```bash
3va doctor
```

### 2.6.3 `sandbox`

Abre un sandbox interactivo aislado.

```bash
3va sandbox
```

### 2.6.4 `dev`

Inicia el servidor de desarrollo.

```bash
3va dev
```

---

## 2.7 Opción Global

### `--accessible`

Activa el modo accesible (sin colores ni animaciones) para lectores de pantalla y terminales Braille. Conforme a EN 301 549.

```bash
3va --accessible run app.ts
3va --accessible install axios --allow-net=registry.npmjs.org
```

---

*Comandos conformes a IEEE 829 y diseño de CLI seguro por defecto.*
