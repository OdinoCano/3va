# 01 - INTERFAZ DE LÍNEA DE COMANDOS

## 1.1 Descripción General

La interfaz de línea de comandos (CLI) de 3va constituye el punto de entrada primario para todas las operaciones del sistema. Implementada mediante la librería `clap` de Rust, proporciona una experiencia de usuario consistente con herramientas modernas como Bun, npm y cargo.

## 1.2 Estructura del Comando Principal

### 1.2.1 Formato de Uso
```
3va [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS] [ARGUMENTS]
```

### 1.2.2 Global Options
Las opciones globales están disponibles para todos los comandos:

| Opción | Descripción | Valor por defecto |
|--------|-------------|-------------------|
| --help, -h | Muestra ayuda | - |
| --version, -v | Muestra versión | - |
| --verbose, -V | Salida verbose | false |
| --quiet, -q | Suprime salida | false |
| --json | Salida en formato JSON | false |
| --config | Archivo de configuración | ~/.3va/config.json |

### 1.2.3 Nivel de Verbosidad
```
0: error      - Solo errores críticos
1: warn       - Advertencias y errores
2: info       - Información general (default)
3: debug      - Depuración detallada
4: trace      - Trazas completas
```

## 1.3 Subcomandos

### 1.3.1 Comando: run
Ejecuta un archivo JavaScript o TypeScript con el runtime de 3va.

```
3va run [OPTIONS] <FILE> [-- <SCRIPT_ARGS>...]
```

**Opciones específicas:**
| Opción | Descripción |
|--------|-------------|
| --inspect | Activa inspector de Chrome |
| --inspect-brk | Inspector con breakpoint inicial |
| --watch | Recarga automática en cambios |
| --env | Variables de entorno como JSON |

**Ejemplo:**
```bash
3va run app.ts --allow-read=/app --allow-net=api.example.com
```

### 1.3.2 Comando: install
Instala uno o más paquetes desde el registry.

```
3va install [OPTIONS] <PACKAGE>[@<VERSION>]...
```

**Opciones específicas:**
| Opción | Descripción |
|--------|-------------|
| --save | Añade a dependencies |
| --save-dev | Añade a devDependencies |
| --save-peer | Añade a peerDependencies |
| --global | Instalación global |
| --allow-net | Permitir acceso a red |

**Ejemplo:**
```bash
3va install axios lodash --save
```

### 1.3.3 Comando: test
Ejecuta la suite de pruebas.

```
3va test [OPTIONS] [FILES_OR_PATTERNS]...
```

**Opciones específicas:**
| Opción | Descripción |
|--------|-------------|
| --watch | Modo watch |
| --coverage | Generar coverage |
| --update-snapshots | Actualizar snapshots |
| --bail | Detener en primer fallo |
| --test-name-pattern | Filtrar por nombre |

**Ejemplo:**
```bash
3va test --coverage --bail
```

### 1.3.4 Comando: build
Empaqueta código para distribución.

```
3va build [OPTIONS] <ENTRY_FILE>
```

**Opciones específicas:**
| Opción | Descripción |
|--------|-------------|
| --out-dir | Directorio de salida |
| --format | Formato: esm, cjs, iife |
| --target | Target: node, browser, webworker |
| --minify | Minificar salida |
| --source-map | Generar source maps |

**Ejemplo:**
```bash
3va build index.ts --out-dir ./dist --minify
```

### 1.3.5 Comando: eval
Evalúa código JavaScript inline.

```
3va eval [OPTIONS] <CODE>
```

**Opciones específicas:**
| Opción | Descripción |
|--------|-------------|
| --print | Imprime el resultado |
| --json | Salida en JSON |

**Ejemplo:**
```bash
3va eval "console.log('Hello ' + 3va')"
```

## 1.4 Flags de Permisos

### 1.4.1 Sistema de Permisos

Los permisos siguen el principio de "denegar por defecto". Los flags de permisos permiten granular qué operaciones están permitidas.

### 1.4.2 Flags de Permiso

| Flag | Recurso | Descripción |
|------|---------|-------------|
| --allow-read | Sistema de archivos | Permite leer archivos |
| --allow-read= | Path específico | Permite leer un path específico |
| --allow-write | Sistema de archivos | Permite escribir archivos |
| --allow-write= | Path específico | Permite escribir en un path específico |
| --allow-net | Red | Permite conexiones de red |
| --allow-net= | Hostname/IP | Permite conectar a host específico |
| --allow-env | Entorno | Permite acceder a variables de entorno |
| --allow-child-process | Procesos | Permite crear procesos hijos |
| --allow-ffi | FFI | Permite llamadas a funciones nativas |

### 1.4.3 Flags de Denegación

| Flag | Descripción |
|------|-------------|
| --deny-read | Deniega lectura de archivos |
| --deny-write | Deniega escritura de archivos |
| --deny-net | Deniega conexiones de red |
| --deny-env | Deniega acceso a entorno |
| --deny-child-process | Deniega creación de procesos |

### 1.4.4 Ejemplos de Permisos

```bash
# Permitir solo lectura del directorio actual
3va run script.ts --allow-read=.

# Permitir acceso a API específica
3va run app.ts --allow-net=api.github.com

# Permisos completos para desarrollo
3va run dev.ts --allow-read --allow-write --allow-net --allow-env --allow-child-process

# Denegar entorno pero permitir red
3va run app.ts --deny-env --allow-net
```

## 1.5 Gestión de Errores

### 1.5.1 Códigos de Salida

| Código | Significado | Ejemplo |
|--------|-------------|---------|
| 0 | Éxito | Ejecución completada |
| 1 | Error general | Fallo desconocido |
| 2 | Error de uso | Argumentos inválidos |
| 3 | Error de configuración | Config inválida |
| 4 | Error de permisos | Permiso denegado |
| 5 | Error de módulo | Módulo no encontrado |
| 6 | Error de runtime | Error en JS |
| 7 | Error de bundle | Error en build |
| 8 | Error de test | Test fallido |
| 9 | Error de seguridad | Vulnerabilidad detectada |

### 1.5.2 Formato de Mensajes de Error

**Modo texto:**
```
Error: Permission denied: FileRead(/etc/passwd)
  --> app.ts:5:1
```

**Modo JSON:**
```json
{
  "error": "permission_denied",
  "message": "Permission denied: FileRead(/etc/passwd)",
  "location": {
    "file": "app.ts",
    "line": 5,
    "column": 1
  }
}
```

---

*Interfaz conforme a IEEE 829 y estándares de CLI.*