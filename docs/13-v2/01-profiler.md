# 01 - CPU Profiler (`--prof`)

> **Status: ✅ Implemented** — available since v1.1.0 (post-1.0.0 patch).

---

## 1.1 Overview

3va includes a sampling-based CPU profiler. Samples are collected via `setInterval` + `new Error().stack` inside the JS runtime, so the profiler is zero-overhead when not enabled and requires no changes to user code.

**Output formats:**
- `.cpuprofile` — V8-compatible JSON; loadable in Chrome DevTools → Performance tab, [speedscope.app](https://speedscope.app), or any V8-profile viewer.
- Flamegraph SVG — generated via the `inferno` Rust crate from folded-stacks data.

**Limitation:** Sampling is event-loop-based. Tight CPU loops with no async yield points appear as a single deep frame. I/O-bound and async programs profile accurately.

---

## 1.2 CLI — `3va run --prof`

```bash
# Collect a profile (default: profile.cpuprofile, 10 ms interval)
3va run app.ts --prof

# Custom output and interval
3va run app.ts --prof --prof-out=my.cpuprofile --prof-interval=5

# Also emit a flamegraph SVG
3va run app.ts --prof --flamegraph=flame.svg
```

| Flag | Default | Description |
|------|---------|-------------|
| `--prof` | off | Enable CPU sampling profiler |
| `--prof-out=<path>` | `profile.cpuprofile` | Output path for `.cpuprofile` JSON |
| `--prof-interval=<ms>` | `10` | Sampling interval in milliseconds |
| `--flamegraph=<path>` | — | Also emit an Inferno-style SVG flamegraph |

`--prof` and `--inspect` cannot be used together.

---

## 1.3 CLI — `3va prof <file>`

Post-hoc analysis of a `.cpuprofile` file:

```bash
# Print top-20 hot functions (self %)
3va prof profile.cpuprofile

# Top-10 only
3va prof profile.cpuprofile --top 10

# Re-generate flamegraph from an existing .cpuprofile
3va prof profile.cpuprofile --format=flamegraph --out=flame.svg
```

| Flag | Default | Description |
|------|---------|-------------|
| `--top <N>` | `20` | Number of hot functions to show |
| `--format <fmt>` | `text` | `text` or `flamegraph` |
| `--out <path>` | `flamegraph.svg` | SVG output path (only with `--format=flamegraph`) |

Example output:

```
Self%  Function
-----------------------------------------
  42%  computeHash
  18%  parsePacket
  12%  (anonymous)
   8%  buildTree
   5%  serialize
```

---

## 1.4 JS API — `console.profile` / `console.profileEnd`

When `--prof` is active, `console.profile` and `console.profileEnd` annotate the profile with named labels:

```js
console.profile('my-label');
// ... code to profile ...
console.profileEnd('my-label');
```

The label is attached to every sample captured while it is active. Labels are preserved in the `.cpuprofile` JSON but are not yet rendered by Chrome DevTools (they appear in the raw JSON under each sample).

---

## 1.5 Implementation

**Sampling mechanism (`crates/js/src/profiler.rs`):**

1. At engine startup (when `--prof` is passed), `JsEngine::new_with_profiler(perms, interval_ms)` is called instead of the standard constructor.
2. The JS bootstrap (`profiler_js(interval_ms)`) is injected: it starts a `setInterval` that captures `new Error().stack` every `interval_ms` milliseconds and calls the Rust-side `__profilerPush(ts_ms, stack_str, label)` native function.
3. Frames from `profiler.rs` and anonymous profiler internals are filtered out of each stack before it is recorded.
4. After `eval_file` completes, `JsEngine::take_profiler()` calls `__profilerStop()` in JS (clears the interval) and returns the `Profiler` handle.

**`.cpuprofile` serializer:**

Builds a V8 CPU profile trie: each unique call stack shares nodes. Output fields:
- `nodes` — call frame trie with `hitCount` and `children`
- `samples` — sequence of leaf node IDs
- `timeDeltas` — time between samples in microseconds
- `startTime` / `endTime` — profile bounds in microseconds

**Flamegraph serializer:**

Converts samples to inferno folded-stacks format (`outer;inner N`) then calls `inferno::flamegraph::from_lines` to produce an SVG.

---

## 1.6 Tests

7 unit tests in `crates/js/src/profiler.rs`:

| Test | Covers |
|------|--------|
| `parse_quickjs_stack_basic` | Stack string parser — 3-frame stack, `<anonymous>` handling |
| `parse_location_full` | `url:line:col` parsing |
| `parse_location_no_col` | `url:line` (no column) |
| `to_folded_stacks_basic` | Folded stacks aggregation and count |
| `cpuprofile_is_valid_json` | `.cpuprofile` output is valid JSON with required fields |
| `analyze_cpuprofile_basic` | Post-hoc analysis returns correct top function |
| `profiler_js_contains_interval` | JS bootstrap embeds the configured interval |
