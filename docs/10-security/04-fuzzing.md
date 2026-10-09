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

### Parser, protocol and transpiler targets

Fourteen targets exist in total. The three above check hard security invariants; these eleven mostly require that arbitrary input never panics, aborts or trips a sanitizer, and add determinism or round-trip `assert!`s where the logic allows it.

| Target | Surface | What is checked |
|--------|---------|-----------------|
| `fuzz_http_request` | `vvva_js::builtins::http_server::parse_request` (the real HTTP/1 request parser) | No panic on arbitrary bytes. It drives the framing code that rejects request smuggling, but the target itself asserts nothing about the verdict; the rejection cases are covered by unit tests in `http_server.rs` |
| `fuzz_pm_tarball` | `ContentStore::store_tarball` (the real install-time extractor) | Path traversal, absolute paths, symlinks, hardlinks and decompression bombs (size and entry-count caps) return an error; never a panic, never a write outside the store |
| `fuzz_pm_package_json` | `parse_npmrc`, `resolve_registry`, `Semver`, `SemverRange`, `DependencyGraph` | No panics on arbitrary `.npmrc` / version text |
| `fuzz_imap` | `parse_mailbox_list` | No panics; consistent parse |
| `fuzz_mqtt` | `encode_remaining_length` and a decoder | Encode/decode round trip |
| `fuzz_ftp`, `fuzz_irc`, `fuzz_pop3` | FTP reply, IRC line and POP3 line parsers | No panics; deterministic. These protocols live in embedded JavaScript with no pure Rust function, so the target fuzzes a **copy** of the logic and must be kept in sync by hand |
| `fuzz_js_transpiler` | `strip_inline_flow_types`, `static_esm_to_cjs` | Deterministic output; no panics |
| `fuzz_js_esm_resolver` | `source_is_esm`, `extract_cjs_named_exports` | Invariants on the ESM/CJS classification; no panics (also a copy of the logic) |
| `fuzz_js_import_meta` | `replace_import_meta`, `has_top_level_await`, `looks_like_jsx` | Deterministic; the replacement text never leaks out of a string literal; output length stays bounded (also a copy of the logic) |

Seeds for the eight newer targets (HTTP, tarball, `package.json`, IMAP, MQTT, FTP, IRC, POP3) are tracked in `fuzz/seeds/<target>/`; the other targets have none yet. The working corpus in `fuzz/corpus/` is git-ignored.

---

## 4.3 Running All Targets

```bash
# Run each target indefinitely (Ctrl-C to stop). Pass the host triple: without
# --target the instrumented build can fail to link and the failure is easy to miss.
cargo fuzz run fuzz_target_1 --target x86_64-unknown-linux-gnu
cargo fuzz run fuzz_permission_sandbox --target x86_64-unknown-linux-gnu
cargo fuzz run fuzz_pm_resolver --target x86_64-unknown-linux-gnu
cargo fuzz run fuzz_http_request --target x86_64-unknown-linux-gnu -- -max_total_time=60

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

Seed corpus entries can be placed in `fuzz/corpus/<target-name>/` before running; libFuzzer replays them before starting mutation. Seeds worth keeping go in `fuzz/seeds/<target-name>/`, which is tracked by git (`fuzz/corpus/` is not).

### Scheduled runs

- **CI smoke run** (`ci.yml`, job *Fuzz*): builds all targets and runs three of them for 30 s each on every pull request.
- **Nightly run** (`.github/workflows/fuzz-nightly.yml`, daily and on demand): every target, 15 minutes each by default (10–30 min via `workflow_dispatch`), seeded from `fuzz/seeds/`, with `-rss_limit_mb=4096`. Crash inputs are uploaded as artifacts and the job fails if any target crashes. GitHub does not notify you when a scheduled run fails, so check the *Actions* tab or enable failure notifications for the workflow.

---

## 4.5 Invariant Failures = Bugs

Every target treats a panic, abort, sanitizer report or failed `assert!` as a bug. When libFuzzer triggers one:

1. The failing input is saved to `fuzz/artifacts/<target-name>/crash-<hash>`
2. Reproduce manually: `cargo fuzz run <target> fuzz/artifacts/<target-name>/crash-<hash>`
3. File a security report following `SECURITY.md §3`.

---

*Implemented with `libfuzzer-sys 0.4`. Targets in `fuzz/fuzz_targets/`. Requires nightly toolchain.*
