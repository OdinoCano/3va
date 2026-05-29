# vvva_wasm

WebAssembly execution engine for 3va — runs WASI-compatible `.wasm` modules inside the same capability-based sandbox.

## Key types

- **`WasmEngine`** — loads and executes a `.wasm` binary; accepts a `PermissionState` to enforce filesystem and network boundaries inside the module

## Usage

```rust
let engine = WasmEngine::new(permissions);
engine.run("module.wasm", &["arg1"])?;
```

Wasm modules share the same permission model as JS scripts: file access, network calls, and environment reads all require explicit flags.
