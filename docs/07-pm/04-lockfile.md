# 04 - FORMATO DE LOCKFILE

## 4.1 Visión General

El lockfile `3va-lock.json` garantiza instalaciones reproducibles, trazabilidad de origen por paquete y auditorías de seguridad. Es generado automáticamente por `3va install` y `3va reinstall`, y es leído por `3va update`.

**Principio clave:** el lockfile es la fuente de verdad para saber qué versión de cada paquete está instalada *y desde qué registry proviene*. Esto permite que `3va update` respete los orígenes sin llamadas de red silenciosas.

---

## 4.2 Formato JSON

```json
{
  "lockfileVersion": 3,
  "name": "my-project",
  "version": "1.0.0",
  "packages": {
    "": {
      "version": "1.0.0",
      "resolved": null,
      "integrity": null,
      "dev": null
    },
    "node_modules/axios": {
      "version": "1.7.2",
      "resolved": "https://registry.npmjs.org/axios/-/axios-1.7.2.tgz",
      "integrity": "sha512-...",
      "registry": "registry.npmjs.org"
    },
    "node_modules/@std/path": {
      "version": "0.196.0",
      "resolved": null,
      "integrity": null,
      "registry": "jsr.io"
    }
  },
  "dependencies": {
    "axios": {
      "version": "1.7.2",
      "resolved": "https://registry.npmjs.org/axios/-/axios-1.7.2.tgz",
      "integrity": "sha512-...",
      "registry": "registry.npmjs.org"
    },
    "@std/path": {
      "version": "0.196.0",
      "resolved": null,
      "integrity": null,
      "registry": "jsr.io"
    },
    "follow-redirects": {
      "version": "^1.15.0",
      "resolved": null,
      "integrity": null
    }
  }
}
```

---

## 4.3 Campos Raíz

| Campo | Tipo | Descripción |
|-------|------|-------------|
| `lockfileVersion` | `number` | Versión del formato. Actualmente `3`. |
| `name` | `string` | Nombre del proyecto (de `package.json`). |
| `version` | `string` | Versión del proyecto. |
| `packages` | `object` | Mapa de entradas de paquetes incluyendo `node_modules/*`. |
| `dependencies` | `object` | Dependencias de primer nivel resueltas. |

---

## 4.4 Entrada de Paquete (`LockfilePackage`)

Aparece bajo `packages["node_modules/<name>"]`:

| Campo | Tipo | Obligatorio | Descripción |
|-------|------|-------------|-------------|
| `version` | `string` | Sí | Versión exacta instalada. |
| `resolved` | `string \| null` | No | URL completa del tarball descargado. |
| `integrity` | `string \| null` | No | Hash de integridad (sha512 base64 o sha256). |
| `dev` | `boolean \| null` | No | `true` si es dependencia de desarrollo. |
| `registry` | `string \| null` | No | Host del registry de origen (e.g., `registry.npmjs.org`, `jsr.io`). Omitido si no se conoce. |

---

## 4.5 Entrada de Dependencia (`LockfileDep`)

Aparece bajo `dependencies["<name>"]`:

| Campo | Tipo | Obligatorio | Descripción |
|-------|------|-------------|-------------|
| `version` | `string` | Sí | Versión instalada. |
| `resolved` | `string \| null` | No | URL del tarball. |
| `integrity` | `string \| null` | No | Hash de integridad. |
| `dependencies` | `object \| null` | No | Subdependencias transitivas. |
| `dev` | `boolean \| null` | No | Dependencia de desarrollo. |
| `registry` | `string \| null` | No | Host del registry de origen. Este campo es el que usa `3va update` para saber adónde conectarse. |

---

## 4.6 Campo `registry` — Seguimiento de Origen

### 4.6.1 Propósito

En proyectos grandes, distintos paquetes pueden provenir de distintos registries:

```json
"axios":    { "version": "1.7.2",   "registry": "registry.npmjs.org" },
"react":    { "version": "18.3.1",  "registry": "registry.yarnpkg.com" },
"@std/path":{ "version": "0.196.0", "registry": "jsr.io" }
```

El campo `registry` es la fuente de verdad para `3va update`: garantiza que cada paquete se actualiza desde su registry original sin cambio silencioso de origen.

### 4.6.2 Cuándo se escribe

- Se escribe en cada `3va install` o `3va reinstall` para el paquete recién instalado.
- Se preserva para los demás paquetes al regenerar el lockfile (el proceso de generación carga el lockfile anterior y copia los valores existentes antes de sobreescribir).
- **No se escribe** para dependencias transitivas inferidas por el resolver (no se contactó un registry real para ellas).

### 4.6.3 Valores reconocidos

| Valor | Registry |
|-------|----------|
| `registry.npmjs.org` | npm oficial |
| `registry.yarnpkg.com` | Yarn |
| `jsr.io` | JSR (JavaScript Registry) |
| Cualquier otro host | Registry custom derivado del host en `--allow-net` |

### 4.6.4 Migración de registry

Para migrar un paquete a otro registry, se usa `install` con el nuevo `--allow-net`. Eso sobrescribe el campo `registry` en el lockfile de forma explícita y auditada:

```bash
# axios pasará a actualizarse desde Yarn en el futuro
3va install axios --allow-net=registry.yarnpkg.com
```

---

## 4.7 Integridad y Seguridad

### 4.7.1 Hash de Integridad

```
integrity: sha512-<hash-base64>
```

Calculado con `SignatureVerifier` (SHA-256 o SHA-512 según configuración).

### 4.7.2 Verificación

```bash
# La verificación ocurre automáticamente en install/reinstall/update
3va install axios --allow-net=registry.npmjs.org
# → [SIGNATURE] Verifying axios@1.7.2 ...
```

---

## 4.8 Operaciones

### 4.8.1 Generar / Actualizar Lockfile

```bash
# Instalar y registrar origen
3va install axios --allow-net=registry.npmjs.org
# → escribe registry: "registry.npmjs.org" en 3va-lock.json

3va install @std/path --allow-net=jsr.io
# → escribe registry: "jsr.io"; preserva el registry de axios
```

### 4.8.2 Actualizar desde Lockfile

```bash
# Ver qué registries se necesitan antes de ejecutar
3va update
# ✗ Update requires network access to:
#     registry.npmjs.org   (axios)
#     jsr.io               (@std/path)
# Run: 3va update --allow-net=registry.npmjs.org,jsr.io

# Ejecutar la actualización
3va update --allow-net=registry.npmjs.org,jsr.io

# Actualizar solo un paquete
3va update axios --allow-net=registry.npmjs.org
```

### 4.8.3 Reinstalar

```bash
3va reinstall axios --allow-net=registry.npmjs.org
```

---

## 4.9 Compatibilidad

El formato es compatible con `package-lock.json` v3 de npm en los campos base (`version`, `resolved`, `integrity`, `dependencies`). El campo `registry` es una extensión propia de 3va; herramientas npm lo ignorarán.

---

*Lockfile conforme a npm lockfile spec v3 con extensiones de seguridad y trazabilidad de origen.*
*Implementado en `crates/pm/src/lockfile.rs`.*
