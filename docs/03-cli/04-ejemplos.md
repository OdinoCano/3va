# 04 - EJEMPLOS DE USO

## 4.1 Ejecución de Scripts

### 4.1.1 Ejecución Básica

```bash
# JavaScript
3va run hello.js

# TypeScript (transpilación automática)
3va run app.ts
```

### 4.1.2 Con Permisos Granulares

Los permisos son denegados por defecto. Se conceden explícitamente:

```bash
# Acceso de lectura a un directorio específico
3va run app.ts --allow-read=/app/data

# Acceso de red a un host específico
3va run app.ts --allow-net=api.example.com

# Combinación de permisos
3va run app.ts --allow-read=/app/config --allow-net=api.example.com --allow-env

# Acceso de escritura
3va run app.ts --allow-write=/tmp/output
```

---

## 4.2 Package Manager

### 4.2.1 Conceptos Clave

**El host en `--allow-net` define el registry.** No existe un flag `--registry` separado.

| Comando | Registry usado |
|---------|---------------|
| `--allow-net=registry.npmjs.org` | npm |
| `--allow-net=registry.yarnpkg.com` | Yarn |
| `--allow-net=jsr.io` | JSR |

**Sin `--allow-net` la red está denegada:**
```bash
3va install axios
# ✗ Network access denied.
#   3va install axios --allow-net=registry.npmjs.org
#   3va install axios --allow-net=registry.yarnpkg.com
#   3va install axios --allow-net=jsr.io
```

### 4.2.2 Instalación desde npm

```bash
# Última versión
3va install axios --allow-net=registry.npmjs.org

# Versión específica
3va install axios@1.7.2 --allow-net=registry.npmjs.org

# Versión no existente → muestra alternativas
3va install axios@99.0.0 --allow-net=registry.npmjs.org
# ✗ Version axios@99.0.0 not found in registry.
#
#   Versions available near 99.0.0:
#     axios@1.7.9
#     axios@1.7.8
#     axios@1.7.7
#     axios@1.7.6
#     axios@1.7.5
```

### 4.2.3 Instalación desde Yarn

```bash
3va install axios --allow-net=registry.yarnpkg.com
3va install react@18.3.1 --allow-net=registry.yarnpkg.com
```

### 4.2.4 Instalación desde JSR

JSR solo acepta paquetes con scope (`@scope/name`):

```bash
# Correcto — paquete con scope
3va install @std/path --allow-net=jsr.io
3va install @std/path@0.196.0 --allow-net=jsr.io

# Error — paquete sin scope no válido en JSR
3va install axios --allow-net=jsr.io
# ✗ JSR only supports scoped packages (e.g. @scope/name)
```

### 4.2.5 Proyecto Multi-Registry

En un mismo proyecto pueden convivir dependencias de distintos registries:

```bash
# axios desde npm, react desde Yarn, @std/path desde JSR
3va install axios --allow-net=registry.npmjs.org
3va install react --allow-net=registry.yarnpkg.com
3va install @std/path --allow-net=jsr.io
```

El lockfile `3va-lock.json` registra el origen de cada uno:

```json
{
  "dependencies": {
    "axios":     { "version": "1.7.2",   "registry": "registry.npmjs.org" },
    "react":     { "version": "18.3.1",  "registry": "registry.yarnpkg.com" },
    "@std/path": { "version": "0.196.0", "registry": "jsr.io" }
  }
}
```

### 4.2.6 Reinstalación

```bash
3va reinstall axios --allow-net=registry.npmjs.org
3va reinstall @std/path --allow-net=jsr.io
```

### 4.2.7 Actualización

`update` respeta el registry registrado en el lockfile para cada paquete:

```bash
# Sin --allow-net: el CLI informa qué hosts se necesitan
3va update
# ✗ Update requires network access to:
#
#     registry.npmjs.org        (axios)
#     registry.yarnpkg.com      (react)
#     jsr.io                    (@std/path)
#
# Run: 3va update --allow-net=registry.npmjs.org,registry.yarnpkg.com,jsr.io

# Actualizar todo
3va update --allow-net=registry.npmjs.org,registry.yarnpkg.com,jsr.io

# Actualizar un solo paquete
3va update axios --allow-net=registry.npmjs.org

# Actualizar paquetes específicos de distintos registries
3va update axios @std/path --allow-net=registry.npmjs.org,jsr.io
```

**Migrar un paquete a otro registry** (acción explícita, queda registrada en el lockfile):

```bash
# axios pasará a actualizarse desde Yarn en el futuro
3va install axios --allow-net=registry.yarnpkg.com
```

---

## 4.3 Testing

```bash
# Todos los tests en el directorio actual
3va test

# Directorio específico
3va test tests/

# Archivo específico
3va test tests/auth.test.ts
```

---

## 4.4 Bundler

```bash
# Bundle con output por defecto (dist/bundle.js)
3va bundle src/index.ts

# Output personalizado
3va bundle src/index.ts --output dist/app.js

# Ejecutar el bundle resultante
3va run dist/bundle.js --allow-net=api.example.com
```

---

## 4.5 Accesibilidad

Desactiva colores y animaciones para lectores de pantalla y terminales Braille (EN 301 549):

```bash
3va --accessible run app.ts
3va --accessible install axios --allow-net=registry.npmjs.org
3va --accessible update --allow-net=registry.npmjs.org,jsr.io
```

---

## 4.6 Scripts en `package.json`

```json
{
  "scripts": {
    "start":   "3va run src/index.ts --allow-net=api.mycompany.com",
    "build":   "3va bundle src/index.ts --output dist/app.js",
    "test":    "3va test",
    "install": "3va install --allow-net=registry.npmjs.org",
    "update":  "3va update --allow-net=registry.npmjs.org,jsr.io"
  }
}
```

---

*Ejemplos conformes a IEEE 829 y al modelo de capacidades de 3va.*
