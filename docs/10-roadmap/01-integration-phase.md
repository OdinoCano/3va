# 10 - ROADMAP AND WORK TRACKING

## 10.1 Current Status (Completed)
The "Integration Phase" (Wiring Phase) has been successfully completed. The following components are connected and functional:

### ✅ CLI Connected
- `3va run` → Reads files and executes with `JsEngine::eval()`
- `3va bundle` → Invokes `vvva_bundler::bundle_file()`
- `3va test` → Invokes `vvva_test::run_tests()`
- `3va install` → Invokes `vvva_pm::install_package()`

### ✅ JS Engine Integrated
- **Injected built-ins**: `console`, `timers`, `buffer`, `process`, `fetch`, `fs`
- **Expanded Console**: log, warn, error, info, debug
- **Permissions connected**: The engine receives `PermissionState` and uses it internally

### ✅ Permission System
- `AuditLogger` implemented in `permissions/src/audit.rs`
- `fs` and `fetch` APIs with permission verification
- Enforcers integrated in the builtins

### ✅ Package Security (NIS2/eIDAS)
- `MalwareScanner` implemented in `pm/src/malware_scanner.rs`
- `SignatureVerifier` implemented in `pm/src/signature_verifier.rs`
- Detection of: fork bombs, recursive deletes, curl|wget|sh, crypto mining, backdoors

### ✅ TimerWheel
- Integration with `setTimeout`/`setInterval` in `js/src/builtins/timers.rs`
- TimerId publicly exposed for external use

---

## 10.2 Stabilization Phase (Completed)

### ✅ TypeScript Transpiler
- `js/src/transpiler.rs` — type stripper in pure Rust (no extra deps)
- Removes: `interface`, `type`, `declare`, `import type`, `export type`
- Removes inline annotations: `const x: string` → `const x`
- Removes `as TypeName`, access modifiers, `!` non-null, generics
- Integrated in `JsEngine::eval_file()` — automatic transpilation for `.ts`/`.tsx`
- 16 transpilation tests, all passing

### ✅ Async Runtime (Event Loop)
- `TimerManager` reimplemented in `js/src/builtins/timers.rs` with `thread_local!`
- Rust natives: `__nativeSetTimeout`, `__nativeSetInterval`, `__nativeClearTimer`
- Real JS wrappers: `setTimeout`/`setInterval` register callbacks and call natives
- `__fireTimer(id)` executes the callback when the timer expires
- `JsEngine::run_event_loop()` — drains all pending timers after `eval_file()`
- CLI: `3va run` now executes the event loop automatically after execution

### ✅ Module System (CommonJS require)
- `js/src/builtins/modules.rs` — `require()` backed by Rust
- Module cache via `global.__requireCache`
- CJS wrapper: `(function(exports, module, __filename, __dirname) { ... })`
- Relative path resolution with `.js` and `.ts` extensions
- Permission verification (`Capability::FileRead`) before reading files
- Automatic `.ts` transpilation in `require()`

### ✅ Lockfile and Cache
- `install_package()` reads/creates `package.json`, resolves deps and generates `3va-lock.json`
- Format compatible with npm lockfile v3 with 3va security extensions

---

## 10.3 Next Steps

1. **ESM import/export** — use `rquickjs::Module` for native ES module loading
2. **Real async/await** — integrate QuickJS Promises with Tokio for non-blocking I/O
3. **REPL / Interactive Sandbox** — implement `3va sandbox`
4. **Dev server** — implement `3va dev` with hot-reload
