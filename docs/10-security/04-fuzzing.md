# 04 - FUZZ TESTING

## 4.1 Overview

3va uses **cargo-fuzz / libFuzzer** for coverage-guided fuzzing of security-critical surfaces. All fuzz targets live in `fuzz/fuzz_targets/` and share a single fuzz workspace at `fuzz/Cargo.toml` that is kept separate from the main workspace.

Fuzzing requires a **nightly** Rust toolchain:

```bash
rustup install nightly
cargo install cargo-fuzz
```

---

## 4.2 Fuzz Targets

### `fuzz_target_1` — Bundler code generator

**File:** `fuzz/fuzz_targets/fuzz_target_1.rs`

Feeds arbitrary UTF-8 strings as JavaScript source into `CodeGenerator` in both IIFE and ESM+minify modes. Ensures the generator never panics, segfaults, or returns inconsistent output for any input.

```bash
cargo fuzz run fuzz_target_1
```

---

### `fuzz_permission_sandbox` — Permission sandbox invariants

**File:** `fuzz/fuzz_targets/fuzz_permission_sandbox.rs`

Exercises `PermissionState`, `VirtualFs`, and `VirtualNetwork` with arbitrary byte inputs. Verifies the following hard invariants on every input:

| Invariant | Description |
|-----------|-------------|
| No path-traversal escape | `VirtualFs::resolve()` result must stay inside the mount source; `/../` attacks must not produce paths outside the sandbox |
| `deny_all_fs` beats any grant | `deny_all_fs()` must block `FileRead`/`FileWrite` even when `grant("/")` is active |
| Explicit deny beats grant | `deny(cap)` always overrides `grant(cap)` for the same capability |
| `deny_all_net` beats any grant | `deny_all_net()` blocks all `Network` checks regardless of grants |
| No panics on arbitrary hosts/paths | `check()` and `is_allowed()` must never panic |

```bash
cargo fuzz run fuzz_permission_sandbox
```

---

### `fuzz_pm_resolver` — Package manager resolver stability

**File:** `fuzz/fuzz_targets/fuzz_pm_resolver.rs`

Exercises `Semver::parse`, `SemverRange::parse`, `DependencyGraph`, and `Resolver::resolve` with arbitrary inputs. Verifies:

| Invariant | Description |
|-----------|-------------|
| No panics | All parse/resolve paths handle arbitrary input without panicking |
| Parse determinism | Same input always produces identical `Semver` (tested with `assert_eq!(a.cmp(&b), Equal)`) |
| Resolver determinism | Two fresh `Resolver` instances produce identical graph sizes for the same input |
| Graph soundness | `get_node`, `resolve_version`, and `nodes()` are callable on any graph produced by `resolve()` |

Input is split at the first NUL byte (`\0`) to exercise two-package scenarios independently.

```bash
cargo fuzz run fuzz_pm_resolver
```

---

## 4.3 Running All Targets

```bash
# Run each target indefinitely (Ctrl-C to stop)
cargo fuzz run fuzz_target_1
cargo fuzz run fuzz_permission_sandbox
cargo fuzz run fuzz_pm_resolver

# Replay an existing corpus without fuzzing (-runs=0)
cargo fuzz run fuzz_permission_sandbox -- -runs=0

# Run with AddressSanitizer (recommended for CI)
cargo fuzz run fuzz_permission_sandbox -- -sanitizer=address
```

Corpus files discovered during fuzzing are saved to `fuzz/corpus/<target-name>/` automatically. Crashes are saved to `fuzz/artifacts/<target-name>/`.

---

## 4.4 Corpus Management

```bash
# Minimize corpus (remove redundant inputs)
cargo fuzz cmin fuzz_permission_sandbox

# Print coverage report for a target
cargo fuzz coverage fuzz_pm_resolver
```

Seed corpus entries can be placed in `fuzz/corpus/<target-name>/` before running; libFuzzer replays them before starting mutation.

---

## 4.5 Invariant Failures = Bugs

All three targets use `assert!` to check security invariants. When libFuzzer triggers an assertion failure:

1. The failing input is saved to `fuzz/artifacts/<target-name>/crash-<hash>`
2. Reproduce manually: `cargo fuzz run <target> fuzz/artifacts/<target-name>/crash-<hash>`
3. File a security report following `SECURITY.md §3`.

---

*Implemented with `libfuzzer-sys 0.4`. Targets in `fuzz/fuzz_targets/`. Requires nightly toolchain.*
