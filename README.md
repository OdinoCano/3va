# 3va (Veni, Vidi, Vici, Abiit)

> *He came, he saw, he conquered, he left.*

**3va** is a modern, secure-by-default, WASM-first JavaScript and TypeScript runtime written in Rust. The name is a tribute to the philosophy and impact of Satoshi Nakamoto—creating a monumental paradigm shift and leaving it to the world.

## Philosophy

The Javascript ecosystem is fundamentally broken from a supply chain security perspective. `3va` reimagines the runtime from the ground up, moving closer philosophically to QubesOS, WASI, and the Chrome Sandbox than to traditional runtimes like Node.js. 

Key principles:
- **Secure-by-Default**: No automatic access to the filesystem, network, environment variables, or child processes.
- **Permission-Based (Capabilities)**: Granular permissions must be explicitly granted via CLI flags (e.g., `--allow-net=api.example.com`).
- **Strict Package Management**: The built-in package manager refuses to execute post-install scripts by default. Dependencies are treated as untrusted and sandboxed.
- **WASM-First**: Built for the future of serverless, edge compute, and isolated WebAssembly components.
- **Post-Quantum Ready**: Designed to integrate modern cryptography (e.g., hybrid TLS, PQC signatures) natively.

## Getting Started (Development)

The runtime is built using Rust and Cargo.

### Prerequisites
- [Rust toolchain](https://rustup.rs/) (edition 2021+)

### Build
```bash
# Clone the repository
git clone https://github.com/yourusername/3va.git
cd 3va

# Build the runtime
cargo build --release

### Running the binary

After building, you can run `3va` in three ways:

1. **Direct path** (recommended for quick testing):
   ```bash
   ./target/release/3va run app.ts
   ```

2. **Add to PATH temporarily** (for the current terminal session):
   ```bash
   export PATH="$PWD/target/release:$PATH"
   3va run app.ts
   ```

3. **Install globally** (requires `sudo`, persists across sessions):
   ```bash
   sudo cp target/release/3va /usr/local/bin/
   3va run app.ts
   ```

### Usage (CLI Preview)

```bash
# Run a file securely (no permissions by default)
3va run app.ts

# Run with explicit capabilities
3va run app.ts \
  --allow-read=/app/config \
  --allow-net=api.example.com \
  --deny-env \
  --deny-child-process

# Install dependencies (strictly sandboxed, no post-install execution)
# The --allow-net host defines which registry to use — no separate --registry flag needed
3va install axios --allow-net=registry.npmjs.org
3va install @std/path --allow-net=jsr.io
3va install react --allow-net=registry.yarnpkg.com
```

## Architecture

`3va` is organized as a Cargo workspace with distinct, specialized crates:
- `vvva_core`: The Tokio-driven async event loop and scheduler.
- `vvva_cli`: The `clap`-based CLI entrypoint.
- `vvva_permissions`: The strict capability-based authorization engine.
- `vvva_js`: The JavaScript engine integration (currently using QuickJS via `rquickjs`).
- `vvva_pm`: The secure package manager.

## License

This project is licensed under the [MIT License](LICENSE).
