# 01 - ESPECIFICACIÓN DEL PACKAGE MANAGER

## 1.1 Visión General

El Package Manager (PM) de 3va es un gestor de paquetes compatible con npm que prioriza la seguridad, implementando verificación de paquetes, sandboxing y análisis de vulnerabilidades.

## 1.2 Características Principales

### 1.2.1 Comparación con npm

| Característica | npm | 3va PM |
|----------------|-----|--------|
| Instalación | ~30s | <1s (cached) |
| post-install scripts | Por defecto | Deshabilitado por defecto |
| Verificación de firma | Opcional | Obligatoria |
| Análisis de malware | No | Sí |
| Auditoría de seguridad | manual | automática |
| Sandboxing | mínimo | completo |

### 1.2.2 Objetivos de Diseño

1. **Velocidad**: Instalar paquetes hasta 30x más rápido que npm
2. **Seguridad**: Verificación obligatoria y análisis de vulnerabilidades
3. **Compatibilidad**: Compatible con package.json de npm
4. **Sandboxing**: Aislamiento de paquetes por defecto
5. **Auditoría**: Registro de todas las operaciones

## 1.3 Arquitectura del PM

### 1.3.1 Componentes

```
┌─────────────────────────────────────────────────────────────────┐
│                      CLI Interface                             │
│                   (3va install, 3va remove)                    │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Package Manager Core                        │
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐       │
│  │   Resolver   │  │   Fetcher    │  │  Installer   │       │
│  └───────────────┘  └───────────────┘  └───────────────┘       │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
              ┌───────────────────┼───────────────────┐
              ▼                   ▼                   ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│  Verifier       │    │   Sandbox       │    │   Audit Log     │
│  (Signatures)   │    │   (Packages)   │    │   (Security)    │
└─────────────────┘    └─────────────────┘    └─────────────────┘
              │                   │                   │
              ▼                   ▼                   ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│  npm Registry   │    │  node_modules   │    │  lockfile       │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

## 1.4 Subcomandos

### 1.4.1 install

```bash
# Instalar paquete
3va install <package>

# Con versión específica
3va install react@18.2.0

# Con alias
3va install lodash-alias@npm:lodash@4.17.21

# Guardar en dependencies
3va install lodash --save

# Guardar en devDependencies
3va install jest --save-dev

# Guardar en peerDependencies
3va install react --save-peer

# Instalación global
3va install typescript --global
```

### 1.4.2 remove

```bash
# Desinstalar paquete
3va remove <package>

# Desinstalar múltiples
3va remove lodash axios
```

### 1.4.3 update

```bash
# Actualizar todos los paquetes
3va update

# Actualizar paquete específico
3va update lodash

# A versión latest
3va update lodash --latest
```

### 1.4.4 list

```bash
# Listar dependencias
3va list

# Profundidad específica
3va list --depth=2

# Solo producción
3va list --prod

# Solo desarrollo
3va list --dev

# Formato JSON
3va list --json
```

### 1.4.5 audit

```bash
# Ejecutar auditoría de seguridad
3va audit

# Mostrar vulnerabilidades
3va audit

# Corregir vulnerabilidades
3va audit fix
```

### 1.4.6 prune

```bash
# Eliminar dependencias no usadas
3va prune
```

## 1.5 Formato de package.json

### 1.5.1 Soportado

```json
{
    "name": "my-package",
    "version": "1.0.0",
    "description": "My package",
    "main": "dist/index.js",
    "type": "module",
    "exports": {
        ".": "./dist/index.js",
        "./feature": "./dist/feature.js"
    },
    "scripts": {
        "build": "3va build src/index.ts",
        "test": "3va test"
    },
    "dependencies": {
        "lodash": "^4.17.21"
    },
    "devDependencies": {
        "jest": "^29.0.0"
    },
    "peerDependencies": {
        "react": ">=17"
    },
    "engines": {
        "3va": ">=1.0.0",
        "node": ">=16"
    }
}
```

### 1.5.2 Configuración de 3va

```json
{
    "3va": {
        "allowScripts": false,
        "verifySignatures": true,
        "sandbox": true
    }
}
```

## 1.6 Registro

### 1.6.1 Registry Predeterminado

```
https://registry.npmjs.org
```

### 1.6.2 Configuración de Registry

```bash
# Usar registry específico
3va install lodash --registry=https://registry.npmmirror.com

# Configurar en package.json
{
    "publishConfig": {
        "registry": "https://registry.mycompany.com"
    }
}

# Configuración global
3va config set registry https://registry.mycompany.com
```

## 1.7 Cumplimiento Normativo (eIDAS / NIS2)

La arquitectura del gestor de dependencias ha sido diseñada para ser intrínsecamente segura frente a ataques de cadena de suministro (Supply Chain Attacks).
- Cumple con las directrices de la directiva europea **NIS2** obligando a la verificación estática de código (Malware Scanner) y restringiendo severamente la ejecución de binarios de terceros (postinstall deshabilitado).
- Adopta mecanismos de infraestructura asimilables a **eIDAS** para la comprobación de firmas criptográficas de los paquetes descargados por el *Fetcher*.

---

*Implementado en los módulos de resolución, caché y extracción criptográfica de `crates/pm/src` (`fetcher.rs`, `resolver.rs`, `lockfile.rs`).*