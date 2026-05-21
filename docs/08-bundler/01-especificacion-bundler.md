# 01 - ESPECIFICACIÓN DEL BUNDLER

## 1.1 Visión General

El bundler de 3va transpila y empaqueta código TypeScript, JavaScript y JSX en un único archivo listo para distribución. Realiza eliminación de código muerto (tree shaking) mediante análisis AST con OXC.

## 1.2 Características

| Característica | Descripción |
|----------------|-------------|
| Transpilación TypeScript | Eliminación de tipos en Rust puro; no requiere `tsc` |
| Tree shaking | Eliminación de exportaciones no utilizadas basada en análisis OXC |
| Code splitting | División en chunks para importaciones dinámicas (`--split`) |
| Minificación | Eliminación de espacios en blanco y comentarios (`--minify`) |
| Source maps | Emisión de archivo `.map` para depuración (`--source-map`) |

## 1.3 Uso

```bash
# Empaquetar un archivo de entrada (salida por defecto: dist/bundle.js)
3va bundle index.ts

# Especificar archivo de salida
3va bundle index.ts -o salida/app.js

# Habilitar code splitting para importaciones dinámicas
3va bundle index.ts --split

# Minificar la salida
3va bundle index.ts --minify

# Emitir source map (genera salida/app.js.map)
3va bundle index.ts -o salida/app.js --source-map

# Combinar opciones
3va bundle index.ts -o dist/app.js --split --minify --source-map
```

Flags no implementados: no existe `--out-dir`, `--format` ni `--target`.

## 1.4 Opciones de CLI

| Flag | Descripción | Valor por defecto |
|------|-------------|-------------------|
| `<input>` | Archivo de entrada (obligatorio) | — |
| `-o <output>` | Archivo de salida | `dist/bundle.js` |
| `--split` | Activa code splitting para importaciones dinámicas | desactivado |
| `--minify` | Elimina espacios en blanco y comentarios | desactivado |
| `--source-map` | Emite `<output>.map` junto al bundle | desactivado |

## 1.5 Arquitectura del Pipeline

```
Archivo de entrada
       │
       ▼
Transpilador TypeScript
(eliminador de tipos en Rust puro)
       │
       ▼
Parser OXC
(genera AST)
       │
       ▼
Tree Shaker
(elimina exportaciones no referenciadas)
       │
       ▼
Code Splitter  ◄── solo con --split
(chunks para importaciones dinámicas)
       │
       ▼
Minificador  ◄── solo con --minify
(elimina espacios y comentarios)
       │
       ▼
Salida: bundle.js  [+ bundle.js.map con --source-map]
```

## 1.6 Modo Watch (uso interno)

El bundler expone `start_watch_mode(input, output, options)`, una función que bloquea el hilo y reconstruye el bundle cada vez que detecta cambios en archivos `.js`, `.ts`, `.jsx` o `.tsx`. Aplica un debounce de **300 ms**.

Este modo es utilizado internamente por `3va dev` y no está pensado para invocarse directamente desde la CLI del bundler.

---

*Bundler implementado en `crates/bundler/src/` (`bundler.rs`, `tree_shaker.rs`, `code_splitter.rs`, `minifier.rs`).*
