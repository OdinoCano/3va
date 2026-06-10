# Changelog — 2026-06-03

Expo / React Native package support + ESM→CJS engine fixes.
Covered by `crates/js/tests/pipeline.rs` and `crates/js/tests/framework_compat.rs`.

---

## ESM→CJS Converter — 7 correctness fixes

All fixes are in `crates/js/src/builtins/modules.rs` (`__esmToCjs` function).

### Circular dependency stack overflow

**Problem:** `require()` cached the module result only *after* evaluating it. A mutual import (A→B→A) caused `require(A)` to be called a second time while A was still loading, entering infinite recursion.

**Fix:** Pre-cache `module.exports` (initially `{}`) before eval:
```js
globalThis.__requireCache[resolvedPath] = globalThis.module.exports; // pre-cache
// ... eval ...
globalThis.__requireCache[resolvedPath] = result;                    // final value
```
Circular requires now receive the partially-filled object, matching Node.js behavior.

---

### `export default X` set `.default` on the wrong object

**Problem:** `module.exports.default = module.exports = X` relies on the JS engine evaluating the LHS reference *before* executing the RHS assignment. The LHS captured the old `module.exports = {}`, so `.default` was set on the discarded empty object rather than the new `X`. Result: `require('./Module').default === undefined`.

**Fix:** Two separate statements:
```js
module.exports = X;
// deferred:
try { var __ed = module.exports; __ed.default = __ed; module.exports = __ed; } catch(__e) {}
```

---

### `export const { a, b } = X` not handled

**Problem:** The converter only matched `export const/let/var IDENTIFIER`. Destructuring patterns like `export const { pickScale } = AssetSourceResolver` fell through unchanged, leaving `export` in the eval'd source.

**Fix:** New patterns before the plain identifier rule:
```
export const { a, b } = X  →  const { a, b } = X; module.exports.a = a; module.exports.b = b;
export const [a, b] = X    →  const [a, b] = X;   module.exports.a = a; module.exports.b = b;
```

---

### `export var X;` (TypeScript enum IIFE) exported `undefined`

**Problem:** TypeScript compiles `enum Foo {}` to:
```js
export var Foo;
(function(Foo) { Foo["A"] = "a"; })(Foo || (Foo = {}));
```
The converter exported `Foo` immediately — before the IIFE ran — so callers received `undefined`. Accessing `Foo.A` then threw "cannot read property 'A' of undefined".

**Fix:** Declarations without `=` (no initializer) are now deferred to end-of-file, after all IIFEs have executed.

---

### `export {}` caused "unsupported keyword: export"

**Problem:** OXC emits `export {};` at the top of re-export-only files (e.g. `src/index.ts` with only `export { X } from './X'`) to mark them as ES modules. This statement wasn't matched by any converter rule and remained in the eval'd source.

**Fix:** `export {}` (empty braces, with or without `from`) is now a no-op.

---

### Dynamic `import()` unsupported in eval context

**Problem:** QuickJS's `import()` uses the ES module loader, which is not configured for CJS-wrapped eval. Any `import(specifier)` expression threw at runtime.

**Fix:** `import(` is replaced with `__importAsync(` at source load time. The polyfill wraps synchronous `require()` in a resolved Promise:
```js
globalThis.__importAsync = function(specifier) {
    var dir = globalThis.__dirname;
    return new Promise(function(resolve, reject) {
        try {
            var mod = globalThis.require(specifier);
            resolve(mod && mod.__esModule ? mod : Object.assign({ default: mod }, mod));
        } catch(e) { reject(e); }
    });
};
```

---

### Deferred exports threw on read-only properties

**Problem:** `module.exports.setCustomSourceTransformer = fn` failed when `Object.defineProperty` had already created a getter-only property with the same name on the exported object.

**Fix:** All deferred export assignments are wrapped in `try{}catch(__de){}`.

---

## TypeScript transpiler fix

`SemanticBuilder::with_enum_eval(true)` added in `crates/js/src/transpiler.rs`. Without it, OXC 0.132 panicked when transforming files containing `const enum` declarations (e.g. `expo-file-system`'s internal types).

---

## Platform-aware extension resolution

`resolve_file_path` in `crates/js/src/builtins/modules.rs` now tries `.web.*` extensions before `.js/.ts`:

```
setUpJsLogger.fx  →  setUpJsLogger.fx.web.ts   (empty stub, was: setUpJsLogger.fx.ts which needs native)
requireNativeModule  →  requireNativeModule.web.ts  (no react-native dep, was: .ts which imports TurboModuleRegistry)
ExpoFileSystem    →  ExpoFileSystem.web.ts      (web stub with FileSystemFile class)
ExpoFontLoader    →  ExpoFontLoader.web.js      (server-safe, was: .js which calls registerWebModule)
ExponentConstants →  ExponentConstants.web.js   (web shim, was: .js which calls NativeModules via react-native)
polyfill/index    →  polyfill/index.web.ts      (installExpoGlobalPolyfill, was: index.ts = noop)
```

Index file probing follows the same order: `index.web.ts` > `index.ts`.

---

## New polyfills

### `react-native`

Pre-cached in `__requireCache['react-native']`. Provides: `Platform` (OS=`'web'`), `NativeModules`, `TurboModuleRegistry`, `PixelRatio`, `Dimensions`, `StyleSheet`, `Animated`, `Linking`, `Alert`, `Keyboard`, `DeviceEventEmitter`, `NativeEventEmitter`, `InteractionManager`, `LayoutAnimation`, `Vibration`, `Share`, `BackHandler`, `I18nManager`, `UIManager`, `AccessibilityInfo`, and all standard component stubs (`View`, `Text`, `Image`, `ScrollView`, `FlatList`, etc.).

### `@react-native/assets-registry`

Pre-cached under both the package name and the `registry` subpath. `registerAsset(meta)` returns a 1-based ID; `getAssetByID(id)` retrieves the registered metadata.

### `NativeModules` proxy redesigned

Previously the proxy auto-created and cached sub-proxies for every property access, making `NativeModules.EXDevLauncher` truthy. `expo-constants` then called `JSON.parse(NativeModules.EXDevLauncher.manifestString)` where `manifestString` was the truthy function-proxy, throwing "unexpected token: 'function'".

Now the proxy returns `undefined` for unregistered module names. Registered modules (ExpoConstants, ExpoFileSystem, etc.) still return a callable proxy.

### `expo-modules-core` polyfill extended

Added: `NativeModule`, `SharedObject`, `SharedRef` (constructors extendable by `class X extends NativeModule`), `registerWebModule(factory)`, `Platform`, `uuid`.

`requireOptionalNativeModule` now always returns `null` — correct for a web/server environment. Returning the module proxy caused truthy checks on proxy properties to pass, breaking `expo-constants`'s manifest loading code.

### `process.env.EXPO_OS = 'web'`

Set at runtime initialization. Expo packages use this to branch to server-safe code paths. Without it:
- `ExpoFontLoader.web.js` evaluated `isServer = false` and called `registerWebModule()` which expected a DOM.
- `setUpJsLogger.fx.ts` attempted to set up native JS logger hooks for iOS/Android.
