# 03 - TREE SHAKING

## 3.1 Dead Code Elimination

Tree shaking removes unused code from the final bundle.

## 3.2 Algorithm

### 3.2.1 Export Analysis

```
1. Parse all modules
2. Build dependency graph
3. Identify used exports
4. Mark imports as "used"
5. Recursively mark dependencies
6. Remove unmarked code
```

### 3.2.2 Side Effects

```javascript
// This code has side effects
// cannot be removed
import { something } from "module";

// Mark as non-removable
/* @__PURE__ */ something();
```

## 3.3 Configuration

| Option | Description |
|--------|-------------|
| treeShaking: true | Enabled by default |
| sideEffects: false | Everything is pure |
| sideEffects: ["*.css"] | Only CSS has side effects |

## 3.4 Example

```javascript
// module.js
export function used() { return 1; }
export function unused() { return 2; }

// main.js
import { used } from "./module.js";
console.log(used());

// Output - unused() removed
console.log(1);
```

---

*Tree shaking conforming to Rollup spec.*
