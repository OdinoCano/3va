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
| `--allow-read=<path>` | `path` (repetible) | Concede permiso de lectura a la ruta indicada. Se puede especificar varias veces. |
| `--allow-write=<path>` | `path` (repetible) | Concede permiso de escritura a la ruta indicada. Se puede especificar varias veces. |
| `--allow-net=<host>` | `string` (repetible) | Concede acceso de red al host indicado. Se puede especificar varias veces. |
| `--allow-env` | `bool` | Permite acceso a variables de entorno del proceso. |
| `--allow-child-process` | `bool` | Permite lanzar procesos hijos. |
| `--interactive` | `bool` | Activa el prompt interactivo de permisos en tiempo de ejecución. |

**Comportamiento:**
1. Carga y valida el archivo de entrada.
2. Inicializa `PermissionState` con los permisos concedidos.
3. Transpila TypeScript automáticamente si la extensión es `.ts`.
4. Ejecuta el archivo en el motor QuickJS.
5. Corre el event loop hasta completar timers, microtasks y callbacks pendientes.

**Ejemplos:**
```bash
# Ejecución sin permisos
3va run app.ts

# Permiso de lectura a un directorio
3va run app.ts --allow-read=/app/data

# Permiso de red
3va run app.ts --allow-net=api.example.com

# Múltiples permisos
3va run app.ts --allow-read=/config --allow-read=/data --allow-net=api.example.com --allow-env

# Modo interactivo (el runtime solicita permisos al usuario conforme los necesita)
3va run app.ts --interactive
```

---

## 2.3 Comandos de Package Manager

### 2.3.1 `install`

Instala un paquete desde un registry. Requiere `--allow-net` con el host del registry. Nunca ejecuta scripts post-install.

**Firma:**
```
3va install [<PACKAGE>[@<VERSION>]] --allow-net=<registry-host>
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `PACKAGE[@VERSION]` | `string` (opcional) | Paquete a instalar con versión opcional. Si se omite el comando completo, instala dependencias del `package.json`. |

**Opciones:**
| Opción | Tipo | Descripción |
|--------|------|-------------|
| `--allow-net=<host>` | `string` | **Requerido.** Host del registry. Determina qué registry se utiliza. |

**El registry se deriva del host:**
| `--allow-net` | Registry usado |
|---------------|---------------|
| `registry.npmjs.org` | npm |
| `registry.yarnpkg.com` | Yarn |
| `jsr.io` | JSR (solo paquetes con scope `@scope/name`) |
| Cualquier otro host | Registry npm-compatible personalizado |

> No existe un flag `--registry` separado — el registry queda determinado exclusivamente por el host autorizado en `--allow-net`, coherente con el modelo de capacidades de 3va.

**Resolución de versión:**
- Si no se especifica versión, se usa `dist-tags.latest`.
- Si la versión solicitada no existe, se muestran las 5 versiones más cercanas por distancia semver.

**Tras una instalación exitosa:**
- Actualiza `package.json` con la dependencia.
- Escribe o actualiza `3va-lock.json` con la versión exacta y el registry de origen.

**Ejemplos:**
```bash
# Desde npm (última versión)
3va install axios --allow-net=registry.npmjs.org

# Desde npm (versión exacta)
3va install axios@1.7.2 --allow-net=registry.npmjs.org

# Desde Yarn
3va install react --allow-net=registry.yarnpkg.com

# Desde JSR (requiere @scope/name)
3va install @std/path --allow-net=jsr.io
3va install @std/path@0.196.0 --allow-net=jsr.io

# Sin --allow-net: error explicativo con el comando correcto
3va install axios
# ✗ Network access denied.
#   3va install axios --allow-net=registry.npmjs.org
```

---

### 2.3.2 `reinstall`

Fuerza la reinstalación de un paquete aunque ya esté instalado. Útil para reparar una instalación corrupta o cambiar de versión.

**Firma:**
```
3va reinstall <PACKAGE>[@<VERSION>] --allow-net=<registry-host>
```

**Ejemplos:**
```bash
3va reinstall axios --allow-net=registry.npmjs.org
3va reinstall axios@1.6.0 --allow-net=registry.npmjs.org
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
| `--allow-net=<hosts>` | `string` | Hosts autorizados, separados por comas. Deben cubrir todos los registries requeridos. |

**Comportamiento:**
1. Lee `3va-lock.json` y el campo `registry` de cada dependencia.
2. Agrupa los paquetes a actualizar por registry.
3. Verifica que `--allow-net` incluya todos los hosts requeridos.
4. Si falta algún host, muestra el comando exacto que el usuario debe ejecutar.
5. Actualiza cada paquete desde su registry original.

**Si falta `--allow-net`:**
```
✗ Update requires network access to:

    registry.npmjs.org        (axios, express)
    jsr.io                    (@std/path)

Run: 3va update --allow-net=registry.npmjs.org,jsr.io
```

**Ejemplos:**
```bash
# Actualizar todos los paquetes
3va update --allow-net=registry.npmjs.org,jsr.io

# Actualizar un paquete específico
3va update axios --allow-net=registry.npmjs.org

# Actualizar varios paquetes de distintos registries
3va update axios @std/path --allow-net=registry.npmjs.org,jsr.io
```

---

## 2.4 Comandos de Testing

### 2.4.1 `test`

Ejecuta la suite de pruebas del proyecto.

**Firma:**
```
3va test [<PATHS>...] [OPTIONS]
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `PATHS` | `path[]` (opcional) | Archivos o directorios donde buscar tests. Por defecto, busca recursivamente en `.` |

**Opciones:**
| Opción | Abreviatura | Descripción |
|--------|-------------|-------------|
| `--watch` | | Ejecuta los tests en modo observador: se re-ejecutan automáticamente al detectar cambios en archivos. |
| `--coverage` | | Genera informe de cobertura de líneas y ramas al finalizar. |
| `--update-snapshots` | `-u` | Sobreescribe los snapshots existentes con los valores actuales. |

**Descubrimiento automático:**

El runner detecta archivos con las siguientes extensiones:
- `*.test.js`
- `*.test.ts`
- `*.spec.js`
- `*.spec.ts`

**Ejemplos:**
```bash
# Ejecutar todos los tests del proyecto
3va test

# Ejecutar tests en un directorio concreto
3va test tests/

# Ejecutar un archivo específico
3va test tests/auth.test.ts

# Modo observador
3va test --watch

# Con cobertura
3va test --coverage

# Actualizar snapshots desactualizados
3va test --update-snapshots
3va test -u

# Combinar flags
3va test tests/ --coverage --watch
```

---

## 2.5 Comandos de Build

### 2.5.1 `bundle`

Empaqueta una aplicación desde un punto de entrada único, resolviendo imports y aplicando tree-shaking.

**Firma:**
```
3va bundle <INPUT> [-o <OUTPUT>] [OPTIONS]
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| `INPUT` | `path` (requerido) | Archivo de entrada (punto de entrada de la aplicación) |

**Opciones:**
| Opción | Abreviatura | Default | Descripción |
|--------|-------------|---------|-------------|
| `--output <path>` | `-o` | `dist/bundle.js` | Ruta del archivo bundle generado |
| `--split` | | | Genera chunks separados (code splitting) |
| `--minify` | | | Minifica el código de salida |
| `--source-map` | | | Genera archivo `.map` junto al bundle |

**Ejemplos:**
```bash
# Bundle básico (salida en dist/bundle.js)
3va bundle src/index.ts

# Bundle con salida personalizada
3va bundle src/index.ts -o dist/app.js

# Bundle para producción
3va bundle src/index.ts --minify --source-map

# Bundle con code splitting
3va bundle src/index.ts --split -o dist/
```

---

## 2.6 Comandos de Desarrollo

### 2.6.1 `dev`

Inicia el servidor de desarrollo con recarga en caliente (HMR) y servicio de archivos estáticos.

**Firma:**
```
3va dev [OPTIONS]
```

**Opciones:**
| Opción | Default | Descripción |
|--------|---------|-------------|
| `--port <N>` | `3000` | Puerto en el que escucha el servidor |
| `--host <H>` | `127.0.0.1` | Dirección de red a la que se enlaza |
| `--open` | | Abre el navegador automáticamente al iniciar |
| `--public-dir <D>` | `public/` | Directorio de archivos estáticos a servir |

**Comportamiento:**
1. Realiza una compilación inicial al arrancar.
2. Vigila cambios en archivos `.js`, `.ts`, `.jsx`, `.tsx` con un debounce de 300 ms.
3. Al detectar cambios, hace rebuild y notifica a los clientes conectados via HMR.
4. Sirve archivos estáticos desde `--public-dir` con los MIME types correctos.
5. Rutas no encontradas devuelven `public/index.html` (fallback SPA).
6. Si no existe `public/index.html`, sirve una página de desarrollo integrada.

**HMR (Hot Module Replacement):**
- El endpoint SSE es `/__hmr`.
- 3va inyecta automáticamente el script cliente HMR justo antes de `</body>` en todos los archivos HTML servidos.
- Los clientes conectados reciben notificaciones de rebuild sin necesidad de recargar manualmente.

**Ejemplos:**
```bash
# Servidor de desarrollo con configuración por defecto
3va dev

# Puerto y host personalizados
3va dev --port 8080 --host 0.0.0.0

# Abrir el navegador automáticamente
3va dev --open

# Directorio público alternativo
3va dev --public-dir www/

# Configuración completa
3va dev --port 4000 --host 0.0.0.0 --open --public-dir static/
```

---

## 2.7 Comandos de Diagnóstico

### 2.7.1 `audit`

Audita las dependencias instaladas en hasta **tres fases**. Las tres fases se ejecutan independientemente: un error en una fase no cancela las siguientes.

**Firma:**
```
3va audit [OPTIONS]
```

**Opciones:**
| Opción | Descripción |
|--------|-------------|
| `--deny` | Sale con código de error ≠ 0 si se detecta algún hallazgo de severidad **CRITICAL** o **HIGH**. Recomendado como gate en pipelines CI/CD. |
| `--update-cache` | Ignora la caché local (TTL 24 h) y descarga datos frescos de la API OSV. |
| `--secrets` | Activa la Fase 3: detección de secretos hardcodeados en el código de dependencias. |
| `--json` | Emite la salida en formato JSON machine-readable en lugar del formato humano. |

**Fases de auditoría:**

**Fase 1 — Análisis estático de malware**
Escanea el código extraído en `node_modules/` buscando patrones maliciosos conocidos:
- Exfiltración de datos (envío de variables de entorno, credenciales del sistema)
- Inyección de comandos en scripts de instalación
- Ofuscación sospechosa (`eval`, `Function`, cadenas codificadas en base64)
- Mineros de criptomonedas (fingerprints de stratum/pool)

**Fase 2 — Vulnerabilidades conocidas (OSV)**
- Consulta `https://api.osv.dev/v1/querybatch` en lotes de hasta 100 paquetes.
- Lee `3va-lock.json` para obtener versiones exactas; si no existe, recorre `node_modules/` como fallback.
- Cachea resultados en `~/.cache/3va/audit/` durante 24 horas.
- Si la API no está disponible, usa la caché stale y avisa al usuario — nunca falla por problemas de conectividad.
- Solo se transmite `{ name, version, ecosystem }` a OSV. No se envía código fuente ni rutas de archivos.

**Fase 3 — Detección de secretos (requiere `--secrets`)**
Escanea dependencias buscando secretos hardcodeados mediante `SecretsScanner`:
- Claves de AWS (`AKIA...`)
- Tokens de GitHub (`ghp_`, `ghs_`, `gho_`)
- Claves privadas PEM (`-----BEGIN ... PRIVATE KEY-----`)
- Tokens JWT (`eyJ...`)
- Claves de API de Stripe (`sk_live_...`)
- Otros patrones de secretos comunes

**Severidad OSV:**
| Rango CVSS v3 | Etiqueta |
|---------------|---------|
| ≥ 9.0 | CRITICAL |
| ≥ 7.0 | HIGH |
| ≥ 4.0 | MEDIUM |
| < 4.0 | LOW |

La severidad se determina en este orden: CVSS v3 vector → CVSS v2 score → campo `database_specific.severity` (formato GitHub Advisory).

**Formato JSON (`--json`):**
```json
{
  "passed": true,
  "phases": {
    "malware": {
      "findings": []
    },
    "osv": {
      "packages_scanned": 12,
      "vulnerable": 0,
      "findings": []
    },
    "secrets": {
      "findings": []
    }
  }
}
```
> Cuando se usa `--json`, todo el output human-readable se suprime y solo se emite el JSON a stdout.

**Caché local:**
```
~/.cache/3va/audit/
  lodash@4.17.20.json
  axios@1.7.9.json
  @scope__pkg@2.0.0.json    ← @ y / sanitizados en el nombre de archivo
```

**Ejemplos:**
```bash
# Auditoría estándar (malware + CVEs)
3va audit

# CI/CD: falla si hay HIGH o CRITICAL
3va audit --deny

# Incluir detección de secretos
3va audit --secrets

# Forzar datos frescos de OSV
3va audit --update-cache

# Salida JSON (para integración con otras herramientas)
3va audit --json

# Auditoría completa con salida JSON para CI
3va audit --secrets --deny --json
```

---

### 2.7.2 `sandbox`

Abre un REPL interactivo de JavaScript aislado en sandbox.

**Firma:**
```
3va sandbox
```

**Comportamiento:**
- En TTY: abre el REPL interactivo con soporte multi-línea. El detector de brackets rastrean paréntesis, corchetes y llaves para determinar cuándo una expresión está completa.
- En pipe / CI (stdin no-TTY): sale inmediatamente sin bloquear el proceso.
- Los objetos se muestran en formato JSON estilo Node.js.
- Las sentencias que producen `undefined` lo muestran explícitamente.

**Comandos de sesión REPL:**
| Comando | Descripción |
|---------|-------------|
| `.help` | Muestra la lista de comandos disponibles |
| `.exit` | Sale del REPL |
| `.clear` | Limpia el contexto de la sesión actual |
| `.allow-read <path>` | Concede permiso de lectura a la ruta indicada en la sesión |
| `.allow-net <host>` | Concede acceso de red al host indicado en la sesión |
| `.permissions` | Lista todos los permisos actualmente concedidos en la sesión |

**Ejemplo de sesión:**
```
3va sandbox
> 1 + 1
2
> const obj = { a: 1, b: [2, 3] }
undefined
> obj
{ "a": 1, "b": [2, 3] }
> .allow-net api.example.com
Granted: net → api.example.com
> .permissions
Granted permissions:
  net: api.example.com
> .exit
```

---

### 2.7.3 `doctor`

Verifica la salud del runtime y del entorno del sistema.

**Firma:**
```
3va doctor
```

Comprueba la instalación del binario, la configuración del entorno, los archivos de bloqueo y otros requisitos del sistema. Útil para diagnosticar problemas de instalación o configuración.

---

## 2.8 Opción Global

### `--accessible`

Activa el modo accesible: deshabilita colores, animaciones y caracteres especiales en la salida. Conforme a EN 301 549 para lectores de pantalla y terminales Braille.

Se puede combinar con cualquier subcomando:

```bash
3va --accessible run app.ts
3va --accessible install axios --allow-net=registry.npmjs.org
3va --accessible audit --deny
3va --accessible test --coverage
```

---

*Comandos conformes a IEEE 829 y diseño de CLI seguro por defecto.*
