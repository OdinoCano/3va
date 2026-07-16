# Benchmarks

The numbers in the main README's comparison table are a claim. This directory
is how you check it — clone the repo, run one script, get your own numbers on
your own hardware.

## Running it

```bash
cargo install hyperfine oha   # one-time
cargo build --release -p vvva_cli
bash bench/run.sh
```

By default it looks for `../target/release/3va` relative to this directory.
Point it at a different binary with `BIN_3VA=/path/to/3va bash bench/run.sh`.
Node and Bun are benchmarked too if they're on `PATH`; if not, those rows are
skipped rather than faked.

## What each number means

- **Startup** — `hyperfine`, 30+ runs, 5 warmup, running `hello.js`.
- **Install (warm)** — `hyperfine` re-running `3va install` when the
  dependency is already satisfied (a no-op resolution check, not a fresh
  download). This is a different measurement than the npm/pnpm/Bun
  install-speed table in the main README, which compares cold-vs-warm
  install across tools and isn't scripted here yet — see
  [`../docs/12-roadmap/06-pm-feature-parity.md`](../docs/12-roadmap/06-pm-feature-parity.md)
  if you want to script that one too.
- **HTTP throughput** — `oha`, 100,000 requests at 1,000 concurrent
  connections against `server.js` (a minimal `http.createServer`/`Bun.serve`
  handler, same shape for every runtime so the comparison is the runtime,
  not three different servers).

## Why `3va.config.json` exists in this directory

3va's HTTP server firewalls by default: 100 req/s and 50 simultaneous
connections per source IP (`crates/firewall/src/lib.rs`'s `FirewallConfig`
default). A single-machine load test at 1,000 concurrent connections comes
from *one* IP, so against the default config it mostly gets rejected with
`403` — not a bug, that's exactly what the firewall is for. The config in
this directory raises those limits so the throughput number reflects the
server's actual request-handling capacity instead of its DDoS protection
kicking in on a benchmark that looks, from the server's point of view,
indistinguishable from an attack.

If you want the *default-config* number instead — i.e. "what does a
believable attacker actually get through" — delete or rename
`3va.config.json` before running the script and expect most requests to
return `403`. Both numbers are real; they answer different questions.

## Reference run

Measured 2026-07-15 on:

- AMD Ryzen 9 7950X (16 cores / 32 threads), 30 GiB RAM, Linux 6.17
- 3va: this repo, release build (`cargo build --release`, fat LTO,
  `rustc 1.97.0-nightly`)
- Node.js 24.17.0
- Bun 1.3.14

| Runtime | Startup (mean) | HTTP throughput (c=1000) | Memory, idle → post-load | Install, warm |
|---|---|---|---|---|
| Node.js 24.17 | 14.8 ms | 69,274 req/s | 45.0 MB → 81.2 MB | — |
| Bun 1.3.14 | 7.0 ms | 124,879 req/s | 34.4 MB → 44.6 MB | — |
| **3va** | 30.1 ms | 12,999 req/s¹ | 31.9 MB → **92.7 MB**² | 12.0 ms |

¹ With `3va.config.json`'s opened-up firewall limits — see above.  
² Was 255.1 MB before the fix described below — a real regression was found and fixed as part of producing this benchmark suite, not a permanent characteristic of the runtime.

Run-to-run variance on throughput was double-digit percent even on an
otherwise idle machine (single-run figures above, not averaged across
repeats) — treat the ranking as reliable and the exact req/s as a snapshot,
not a guarantee.

### Memory under load: root cause and fix

The first version of this reference run showed 3va's memory growing ~8.5×
from idle to 1,000 concurrent connections (30 MB → 255 MB), against ~1.8×
for Node and ~1.3× for Bun. That was investigated rather than left as a
caveat:

1. **Isolated concurrency from cumulative load.** Re-running with a fresh
   server per concurrency level (100/500/1000/2000) showed near-identical
   post-load RSS regardless of concurrency — ruling out a connection-count-
   driven cause (e.g. a per-connection buffer or an unbounded backlog
   between the accept loop and JS).
2. **Confirmed a linear per-request cost instead.** Fixed concurrency
   (c=50), increasing cumulative request count (10k/30k/60k/100k) against
   the *same* long-lived process: RSS grew ~3.5–4 KB per request, forever,
   independent of concurrency, and never recovered even across idle gaps
   between rounds.
3. **Ruled out the obvious native-side suspects.** `crates/js/src/builtins/http_server.rs`'s
   `conns`/`ready` maps (candidate: connections piling up faster than JS
   drains them) are correctly removed on every successful respond path —
   confirmed by reading `http_server.rs:589-616`, and consistent with this
   benchmark's 99.7–100% success rate. Not the driver here.
4. **Tried mimalloc as the global allocator** (a very common fix for
   exactly this RSS-never-shrinks symptom, since glibc's malloc rarely
   returns freed pages to the OS) — **no measurable difference**. This
   ruled out Rust-heap fragmentation as the cause: `#[global_allocator]`
   only affects Rust-level allocations (`Vec`, `String`, `Box`, ...); V8
   manages its own C++ heap via its own page allocator, entirely outside
   Rust's allocator hook.
5. **Root cause: V8's heap was never told to shrink.** `crates/js/src/lib.rs`
   had no call to V8's `low_memory_notification()` anywhere — the only API
   that prompts V8 to actually try to free memory back to the OS. Every
   request generates real (but collectible) V8 garbage — parsed headers,
   JSON strings, Promise/closure objects — and without that hint, V8's
   heap grows to its burst high-water mark and stays there.
6. **Fix:** `run_event_loop` (`crates/js/src/lib.rs`) now calls
   `isolate.low_memory_notification()` on a 5-second throttle
   (`LOW_MEMORY_HINT_INTERVAL`) — frequent enough to reclaim memory during
   sustained load, not so frequent that a full GC pause on every event-loop
   tick would hurt throughput. Verified: post-load RSS dropped from 255 MB
   to 93–180 MB across repeated runs (a 30–64% reduction depending on run),
   with throughput unchanged (16,100 req/s @ c=1000, 99.94% success,
   measured before and after) — the fix costs nothing observable and keeps
   most of the win.

The remaining ~3× idle-to-loaded growth (versus Node's ~1.8×) wasn't chased
further — mimalloc is still wired in as the global allocator (a reasonable
default for a busy server generally, even though it didn't fix this
specific bug), and a shorter hint interval or a request-count-based trigger
instead of a time-based one are the next things to try if this needs to go
lower.

## CI

[`../.github/workflows/benchmark.yml`](../.github/workflows/benchmark.yml)
runs this script on `workflow_dispatch` and on release tags, publishing the
result table to the workflow's job summary. GitHub-hosted runners have
noisier, weaker hardware than a dedicated machine, so treat CI's absolute
numbers as regression signal against *previous CI runs*, not as the
headline figures — the reference run above is what should back README claims.
