# 01 - ESPECIFICACIÓN DEL BUNDLER

## 1.1 Visión General

El bundler de 3va transpila y empaqueta código TypeScript, JavaScript y JSX para distribución, con análisis de seguridad integrado.

## 1.2 Características

| Caracteristica | Descripcion |
|----------------|-------------|
| Transpilación | TypeScript, JSX, TSX |
| Tree shaking | Eliminación de código muerto |
| Code splitting | División en chunks |
| Minificación | Compression de salida |
| Source maps | Mapas de debug |
| Security scan | Análisis de vulnerabilidades |

## 1.3 Uso

```bash
# Build básico
3va build index.ts

# Output a directorio
3va build index.ts --out-dir ./dist

# Minificar
3va build index.ts --minify

# Formatos
3va build index.ts --format=esm
3va build index.ts --format=cjs
3va build index.ts --format=iife

# Target
3va build index.ts --target=node
3va build index.ts --target=browser

# Con source maps
3va build index.ts --source-map
```

## 1.4 Arquitectura

```
┌──────────────┐
│    Entry     │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│    Parse     │ ───► AST
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Transform  │ ───► TSX►JS, imports►requires
└──────┬───────┘
       │
       ▼
┌──────────────┐
│    Analyze   │ ───► Dep graph, tree shaking
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Generate   │ ───► Output bundle
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Security   │ ───► Scan de vulnerabilidades
└──────────────┘
```

---

*Bundler conforme a Rollup y esbuild spec.*