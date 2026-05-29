# vvva_bundler

Module bundler for 3va — resolves imports, tree-shakes unused exports, and emits single-file or code-split JavaScript bundles.

## Key types

- **`Bundler`** — main bundler; call `add_entry()` then `bundle()`
- **`BundlerOptions`** — configure output format, sourcemaps, minification
- **`CodeGenerator`** / **`CodeSplitter`** — lower-level code emission
- **`TreeShaker`** — dead-export elimination via import graph analysis

## TypeScript / TSX

`.ts` and `.tsx` files are transpiled to JavaScript using the oxc toolchain (full AST parse → transform → codegen). Type annotations, interfaces, and generics are stripped correctly — not via line-based heuristics.

## Docs

`docs/08-bundler/`
