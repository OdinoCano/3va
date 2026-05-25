# 04 - TREE SHAKING

## 4.1 Overview

Tree shaking removes unused exports from library modules before emitting the bundle. 3va's implementation is AST-based, powered by the Oxc parser, and runs as a three-pass pipeline inside `Bundler::bundle()`.

Entry-point modules are **never shaken** — their exports are the public API and must all be preserved regardless of whether any other bundled module imports them.

---

## 4.2 Pipeline

`Bundler::bundle()` performs three sequential passes over the module set:

```
Pass 1 — Process
  For each module path: read file, strip TypeScript types if needed → raw JS source

Pass 2 — Analyze named imports
  For each module source, call TreeShaker::analyze_named_imports()
  Collect:  module_path → { exported names imported by other modules }
  Store in: Bundler.used_exports

Pass 3 — Shake & emit
  For each module:
    • If module is an entry point  → emit unchanged (all exports kept)
    • Else if used_exports[name] exists → TreeShaker::shake(name, code, used)
    • Else                         → emit unchanged (no import info → conservative)
  Add final code to CodeGenerator
```

---

## 4.3 Entry Points

A module registered via `Bundler::add_entry()` is automatically marked as an entry point in the `TreeShaker`. Entry-point modules bypass tree shaking entirely:

```rust
let mut bundler = Bundler::new(root);
bundler.add_entry("src/index.ts")?;  // ← index.ts is an entry point; exports are preserved
let code = bundler.bundle()?;
```

The entry-point list is maintained by `TreeShaker::add_entry_point(&str)`, which is called internally by `add_entry`. You can also call it directly when constructing a `TreeShaker` standalone:

```rust
let mut shaker = TreeShaker::new(vec!["src/index.ts".to_string()]);
// or after construction:
shaker.add_entry_point("src/extra-entry.ts");
```

---

## 4.4 Named Import Analysis

`TreeShaker::analyze_named_imports(code: &str) -> HashMap<String, HashSet<String>>`

Parses the source with the Oxc AST and returns which named exports each module imports from each dependency:

| Import form | Recorded as |
|-------------|-------------|
| `import { foo, bar } from './utils'` | `"./utils" → {"foo", "bar"}` |
| `import defaultExport from './mod'` | `"./mod" → {"default"}` |
| `import * as ns from './lib'` | `"./lib" → {"*"}` |

A `"*"` entry means the full module surface is required; that module will not be shaken even if it is not an entry point.

---

## 4.5 Shake Semantics

`TreeShaker::shake(module_name: &str, module_code: &str, used_exports: &HashSet<String>) -> String`

- If `module_name` is a registered entry point → returns `module_code` unchanged.
- If `used_exports` is empty → returns `module_code` unchanged (conservative — no information means no removal).
- Otherwise: removes any `export function`, `export const`, `export class`, or `export { ... }` declaration whose exported name is **not** in `used_exports`.

Non-export statements (function bodies, imports, expressions) are never removed.

---

## 4.6 Example

```javascript
// utils.js
export function used() { return 1; }
export function dropped() { return 2; }

// main.js  ← entry point
import { used } from "./utils.js";
console.log(used());
```

After bundling with `main.js` as entry:
- `main.js` is an entry point → emitted unchanged
- `used_exports["./utils.js"] = {"used"}` (from Pass 2 analysis of `main.js`)
- `utils.js` is shaken: `dropped()` is removed

```javascript
// utils.js after shake
export function used() { return 1; }
```

---

## 4.7 Current Limitation — Path Resolution

The key used in `used_exports` is the **raw import string** as it appears in source (`"./utils"`, `"lodash"`). The `ModuleResolver` field in `Bundler` is reserved for future normalization of these paths to absolute canonical paths, enabling cross-module tree shaking across complex dependency graphs. Until then, tree shaking works reliably when import paths exactly match the entry names passed to `add_entry()`.

---

*Implemented in `crates/bundler/src/tree_shaker.rs` (`TreeShaker`, `DeadCodeEliminator`) and `crates/bundler/src/lib.rs` (`Bundler::bundle`).*
