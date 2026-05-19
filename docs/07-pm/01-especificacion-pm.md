# 01 - ESPECIFICACIÓN DEL PACKAGE MANAGER

## 1.1 Visión General

El Package Manager (PM) de 3va es un gestor de dependencias seguro por defecto. Prioriza la seguridad de la cadena de suministro sobre la comodidad: ninguna llamada de red ocurre sin permiso explícito del usuario.

## 1.2 Filosofía de Diseño

### 1.2.1 El Registry lo define `--allow-net`

A diferencia de npm/yarn/pnpm, 3va **no tiene un flag `--registry`**. El host que el usuario autoriza en `--allow-net` *es* el registry. Esto es coherente con el modelo de capacidades del runtime:

```bash
# El host autorizado determina el registry
3va install axios --allow-net=registry.npmjs.org      # → npm
3va install axios --allow-net=registry.yarnpkg.com    # → Yarn
3va install @std/path --allow-net=jsr.io              # → JSR
```

Tener un flag `--registry` separado duplicaría la autorización y rompería el modelo de seguridad.

### 1.2.2 Red denegada por defecto

```bash
3va install axios
# ✗ Network access denied.
#   3va install axios --allow-net=registry.npmjs.org
#   3va install axios --allow-net=registry.yarnpkg.com
#   3va install axios --allow-net=jsr.io
```

### 1.2.3 Comparación con gestores tradicionales

| Característica | npm | yarn | 3va PM |
|----------------|-----|------|--------|
| Red por defecto | Sí | Sí | **No** |
| Flag de registry | `--registry` | `--registry` | `--allow-net` |
| Post-install scripts | Por defecto | Por defecto | **Deshabilitado** |
| Verificación de firma | Opcional | Opcional | Obligatoria |
| Análisis de malware | No | No | Sí |
| Multi-registry por proyecto | No | No | **Sí** |
| Origen por paquete en lockfile | No | No | **Sí** |

---

## 1.3 Registries Soportados

### 1.3.1 npm (`registry.npmjs.org`)

API compatible con npm registry. Devuelve JSON con campos `versions` y `dist-tags.latest`.

```bash
3va install axios --allow-net=registry.npmjs.org
3va install axios@1.7.2 --allow-net=registry.npmjs.org
```

### 1.3.2 Yarn (`registry.yarnpkg.com`)

Mismo protocolo que npm. El host autorizado determina que se use Yarn como origen.

```bash
3va install react --allow-net=registry.yarnpkg.com
```

### 1.3.3 JSR (`jsr.io`)

Solo acepta paquetes con scope (`@scope/name`). Usa el endpoint:
`GET https://jsr.io/api/scopes/{scope}/packages/{name}/versions`

Respuesta: `{ "items": [{ "version": "..." }] }`

```bash
3va install @std/path --allow-net=jsr.io
3va install @std/path@0.196.0 --allow-net=jsr.io

# Error: paquete sin scope no válido en JSR
3va install axios --allow-net=jsr.io
# ✗ JSR only supports scoped packages (e.g. @scope/name)
```

### 1.3.4 Registry Custom

Cualquier host que no coincida con los tres anteriores se trata como registry npm-compatible:

```bash
3va install my-pkg --allow-net=registry.mycompany.com
```

---

## 1.4 Subcomandos

### 1.4.1 `install`

```bash
3va install <package>[@<version>] --allow-net=<registry-host>
```

**Flujo:**
1. Validar nombre y versión del paquete.
2. Verificar `--allow-net` — si falta, error con sugerencia de comandos.
3. Derivar registry del host autorizado.
4. Consultar el registry (verificar existencia del paquete).
5. Resolver versión: usa `latest` si no se especifica; si la versión no existe, muestra las 5 más cercanas.
6. Verificar firma del paquete.
7. Actualizar `package.json`.
8. Regenerar `3va-lock.json` preservando registries previos y registrando el del nuevo paquete.

**Detección de paquete ya instalado:**
```bash
3va install axios --allow-net=registry.npmjs.org
# ✓ axios@1.7.2 is already installed.
#   Use 'reinstall' to force reinstall.
```

**Versión no encontrada — sugerencia de versiones cercanas:**
```bash
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

### 1.4.2 `reinstall`

Fuerza la reinstalación aunque el paquete ya esté instalado.

```bash
3va reinstall <package>[@<version>] --allow-net=<registry-host>
```

### 1.4.3 `update`

Actualiza paquetes a su última versión, respetando el registry de origen registrado en el lockfile.

```bash
# Actualizar todos los paquetes
3va update --allow-net=<todos-los-hosts-necesarios>

# Actualizar paquetes específicos
3va update axios @std/path --allow-net=registry.npmjs.org,jsr.io
```

**Si `--allow-net` no cubre todos los registries necesarios:**
```bash
3va update
# ✗ Update requires network access to:
#
#     registry.npmjs.org        (axios, express)
#     jsr.io                    (@std/path)
#
# Run: 3va update --allow-net=registry.npmjs.org,jsr.io
```

**Flujo interno:**
1. Leer `3va-lock.json`.
2. Determinar qué paquetes actualizar (todos o los especificados).
3. Leer el campo `registry` de cada paquete en el lockfile.
4. Verificar que `--allow-net` cubre todos los registries necesarios.
5. Para cada paquete, reinstalar desde su registry original.

**Nota:** `update` nunca cambia el registry de un paquete. Para migrar a otro registry, usar `install` explícitamente.

---

## 1.5 Multi-Registry por Proyecto

Un proyecto puede tener dependencias de distintos registries simultáneamente. El lockfile registra el origen de cada una:

```json
{
  "dependencies": {
    "axios":     { "version": "1.7.2",   "registry": "registry.npmjs.org" },
    "react":     { "version": "18.3.1",  "registry": "registry.yarnpkg.com" },
    "@std/path": { "version": "0.196.0", "registry": "jsr.io" }
  }
}
```

Para actualizar este proyecto:
```bash
3va update --allow-net=registry.npmjs.org,registry.yarnpkg.com,jsr.io
```

---

## 1.6 Resolución de Versiones

### 1.6.1 Versión no especificada

Usa `dist-tags.latest` del registry (npm/Yarn) o la última entrada de `items[]` (JSR).

### 1.6.2 Versión especificada y existente

```bash
3va install axios@1.7.2 --allow-net=registry.npmjs.org
# ✓ Version axios@1.7.2 exists
```

### 1.6.3 Versión especificada y no existente

Calcula las 5 versiones más cercanas por distancia semver numérica:
`score = major × 1_000_000 + minor × 1_000 + patch`

Las sugerencias siempre siguen el formato `name@version`.

### 1.6.4 Formato de especificación de paquete

| Formato | Ejemplo | Resultado |
|---------|---------|-----------|
| Solo nombre | `axios` | Instala `latest` |
| Nombre + versión | `axios@1.7.2` | Instala versión exacta |
| Scoped | `@std/path` | Instala `latest` del scope |
| Scoped + versión | `@std/path@0.196.0` | Instala versión exacta |

---

## 1.7 Formato de `package.json`

```json
{
  "name": "my-package",
  "version": "1.0.0",
  "description": "",
  "main": "index.js",
  "type": "module",
  "dependencies": {
    "axios": "1.7.2",
    "@std/path": "0.196.0"
  }
}
```

3va escribe versiones exactas (sin `^` ni `~`) al instalar para garantizar reproducibilidad.

---

## 1.8 Seguridad

### 1.8.1 Post-install scripts

Deshabilitados por defecto. Los scripts `postinstall`, `install`, `preinstall` definidos en `package.json` de dependencias **no se ejecutan**.

### 1.8.2 Verificación de firmas

Cada paquete pasa por `SignatureVerifier` (SHA-256/SHA-512) antes de registrarse en el lockfile.

### 1.8.3 Scanner de malware

`MalwareScanner` analiza el contenido del paquete antes de instalarlo.

### 1.8.4 Cumplimiento Normativo

- **NIS2**: verificación estática de código y restricción de ejecución de binarios de terceros.
- **eIDAS**: mecanismos de verificación de firmas criptográficas de paquetes.

---

*Implementado en `crates/pm/src/` (`lib.rs`, `lockfile.rs`, `fetcher.rs`, `resolver.rs`, `signature_verifier.rs`, `malware_scanner.rs`).*
