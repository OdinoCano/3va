# Changelog — 2026-06-01

Security and correctness fixes identified by code-review (10 findings).
All changes are in `crates/`; no public API surface was removed.

---

## CRÍTICO

### `__pqTlsConnect` blocked the JS event loop (`crates/js/src/builtins/tcp.rs`)

**Problem:** The `__pqTlsConnect` JS binding performed a full TCP connect +
TLS handshake + ML-KEM-768 key exchange synchronously on the JS event loop
thread.  Any call to this function on a slow or unresponsive host would freeze
all timers, microtasks, and other I/O for the duration of the network round trips.

**Fix:** Extracted all blocking I/O into `pq_tls_connect_blocking()` — a
standalone function that runs inside `tokio::task::spawn_blocking`.  The JS
binding is now registered as `Async`, consistent with `__netAcceptAsync` and
other async networking primitives.

**Bonus fix:** The server-sent ciphertext bytes were previously passed through
`hex::encode` then `MlKemCiphertext::from_hex`, an unnecessary round-trip.
A new `MlKemCiphertext::from_bytes(&[u8])` method was added to `vvva_crypto`
and used directly.

---

## ALTO

### `SemverRange` silently rejected common npm range forms (`crates/pm/src/semver.rs`)

**Problem:** The removal of the old `Range(String) => true` catch-all caused
any version string not recognised by the parser — `"latest"`, `"1.x"`,
`"1.2"`, `">=1.0.0 <2.0.0"` — to return `None`, silently failing package
resolution with no error or diagnostic.

**Fix:**
- **Dist-tags** (`latest`, `next`, `beta`, …) → `SemverRange::Any`
- **X-ranges** (`1`, `1.x`, `1.2`, `1.2.x`) → `Caret` or `Tilde` equivalent
- **Compound ranges** (`>=1.0.0 <2.0.0`) → new `And(Box<SemverRange>, Box<SemverRange>)` variant
- Added 8 new tests covering all new forms.

### Dependency resolution silently ignored version conflicts (`crates/pm/src/resolver.rs`)

**Problem:** When a package was required by multiple dependents with
incompatible version ranges, the resolver silently used the first-seen version
with no warning.  Downstream code received a version that may not satisfy all
constraints.

**Fix:** When the already-resolved version does not satisfy an incoming
constraint, a structured `tracing::warn!` is emitted:

```
WARN version conflict: resolved version does not satisfy this constraint
  package=foo resolved=1.5.0 required=^2.0.0
```

### Dependency resolution order was non-deterministic (`crates/pm/src/resolver.rs`)

**Problem:** The initial resolution stack was populated from a `HashMap`
iterator whose order is randomised per run by Rust's default hasher.  Identical
inputs could produce different lockfiles across invocations.

**Fix:** The initial stack and every batch of transitive dependencies are now
sorted descending by package name before being pushed, so `pop()` always
processes packages in ascending alphabetical order regardless of HashMap seed.

---

## MEDIO

### `Content-Length` header forwarded to JS did not reflect the 100 MiB cap (`crates/js/src/builtins/http_server.rs`)

**Problem:** `parse_request` capped the `content_length` allocation at
`100 * 1024 * 1024` bytes, but pushed the original raw header value string into
the `headers` vec that is serialized to JS.  A handler reading
`req.headers['content-length']` would see the original inflated value (e.g.
`"209715201"`) even though only 100 MiB was actually read from the socket.

**Fix:** When `content-length` is present, the header entry pushed to the vec
now uses `content_length.to_string()` (the capped value) rather than the raw
header string.  Added two integration tests: one verifying the header value
matches the body for normal requests, one verifying the server survives an
oversized Content-Length + early connection close without panicking.

### `find_best_match` silently dropped nodes with unparseable version strings (`crates/pm/src/resolver.rs`)

**Problem:** Nodes whose stored version string did not parse as a `Semver` were
dropped silently by the `filter` predicate.

**Fix:** Added a `tracing::warn!` inside the filter so unparseable versions
surface in logs with the package name and raw version string.

---

## BAJO

### `collect_installed` lockfile path was hardcoded to CWD (`crates/pm/src/lib.rs`)

**Problem:** `PathBuf::from("3va-lock.json")` resolved against whatever the
process CWD was at call time, not relative to the `node_modules` argument's
parent.  Workspace audits with a non-CWD `node_modules` path would read the
wrong lockfile.

**Fix:** Path is now derived as `node_modules.parent().unwrap_or(".").join("3va-lock.json")`.

### `audit_packages_silent` reported `"unknown"` for all versions when no lockfile existed (`crates/pm/src/lib.rs`)

**Problem:** The fallback filesystem scan set version to `"unknown"` for every
package, making CVE lookups and version-gated heuristics unreliable.

**Fix:** New `read_package_version(pkg_dir)` reads `package.json` for the
`"version"` field, falling back to `"unknown"` only when the file is absent or
unparseable.

### Resolver made one HTTP request per package with no parallelism (`crates/pm/src/resolver.rs`)

**Problem:** Packages were resolved sequentially — one network round-trip at a
time.  A project with N dependencies took N × RTT to resolve.

**Fix:** Uncached packages are batched and fetched concurrently via
`tokio::spawn`, then re-queued for processing.  The cache hit path is unaffected.

### `MlKemCiphertext::from_bytes` added (`crates/crypto/src/kem.rs`)

New constructor to decode a ciphertext from raw bytes without a hex round-trip.
`from_hex` is now implemented in terms of `from_bytes`.  Three new tests added.

---

## Documentation updated

| File | Change |
|------|--------|
| `docs/07-pm/02-resolucion.md` | Rewritten to match real resolver: parallel fetching, determinism, conflict detection, full semver range table |
| `docs/10-security/05-post-quantum.md` | Hybrid PQ-TLS moved from "Planned" to "Done"; `from_bytes` API documented; JS API examples added |
