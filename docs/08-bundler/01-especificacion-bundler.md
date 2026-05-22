# 01 - BUNDLER SPECIFICATION

## 1.1 Overview

3va's bundler transpiles and packages TypeScript, JavaScript, and JSX code into a single file ready for distribution. It performs dead code elimination (tree shaking) via AST analysis with OXC.

## 1.2 Features

| Feature | Description |
|----------------|-------------|
| TypeScript Transpilation | Type stripping in pure Rust; no `tsc` required |
| Tree shaking | Removal of unused exports based on OXC analysis |
| Code splitting | Division into chunks for dynamic imports (`--split`) |
| Minification | Whitespace and comment removal (`--minify`) |
| Source maps | `.map` file emission for debugging (`--source-map`) |

## 1.3 Usage

```bash
# Bundle an entry file (default output: dist/bundle.js)
3va bundle index.ts

# Specify output file
3va bundle index.ts -o output/app.js

# Enable code splitting for dynamic imports
3va bundle index.ts --split

# Minify output
3va bundle index.ts --minify

# Emit source map (generates output/app.js.map)
3va bundle index.ts -o output/app.js --source-map

# Combine options
3va bundle index.ts -o dist/app.js --split --minify --source-map
```

Flags not implemented: no `--out-dir`, `--format` or `--target`.

## 1.4 CLI Options

| Flag | Description | Default |
|------|-------------|---------|
| `<input>` | Entry file (required) | — |
| `-o <output>` | Output file | `dist/bundle.js` |
| `--split` | Enables code splitting for dynamic imports | disabled |
| `--minify` | Removes whitespace and comments | disabled |
| `--source-map` | Emits `<output>.map` alongside the bundle | disabled |

## 1.5 Pipeline Architecture

```
Entry file
       │
       ▼
TypeScript Transpiler
(pure Rust type stripper)
       │
       ▼
OXC Parser
(generates AST)
       │
       ▼
Tree Shaker
(removes unreferenced exports)
       │
       ▼
Code Splitter  ◄── only with --split
(chunks for dynamic imports)
       │
       ▼
Minifier  ◄── only with --minify
(removes whitespace and comments)
       │
       ▼
Output: bundle.js  [+ bundle.js.map with --source-map]
```

## 1.6 Watch Mode (internal use)

The bundler exposes `start_watch_mode(input, output, options)`, a blocking function that rebuilds the bundle whenever it detects changes in `.js`, `.ts`, `.jsx` or `.tsx` files. Applies a debounce of **300 ms**.

This mode is used internally by `3va dev` and is not intended to be invoked directly from the bundler CLI.

---

*Bundler implemented in `crates/bundler/src/` (`lib.rs`, `generator.rs`, `tree_shaker.rs`, `resolver.rs`).*
