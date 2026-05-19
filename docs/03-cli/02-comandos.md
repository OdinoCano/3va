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

Audita las dependencias instaladas.

```bash
3va audit
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
