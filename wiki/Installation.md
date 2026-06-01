# Installation

## Prerequisites

- [Rust toolchain](https://rustup.rs/) (edition 2021 or later)
- `cargo` available in your PATH

## Build from source

```bash
git clone https://github.com/OdinoCano/3va.git
cd 3va
cargo build --release
```

The binary will be at `target/release/3va`.

## Install the binary

**Temporary (current session only):**
```bash
export PATH="$PWD/target/release:$PATH"
```

**Permanent:**
```bash
sudo cp target/release/3va /usr/local/bin/
```

## Verify installation

```bash
3va --version
3va doctor
```

`3va doctor` runs a system health check to verify the runtime environment is correctly set up.

---

## First run

```bash
# Run a TypeScript file (no permissions granted)
3va run app.ts

# Run with network and read access
3va run app.ts --allow-net=api.example.com --allow-read=/data
```

See [[CLI Reference]] for all available flags.
