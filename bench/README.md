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

The numbers backing the main README's comparison table now come from
[`.github/workflows/benchmark.yml`](../.github/workflows/benchmark.yml)
itself, not a hand-run on a maintainer's machine — see the "CI" section
below for why. Latest published run:
[actions/runs/32180248410](https://github.com/OdinoCano/3va/actions/runs/32180248410)
(2026-08-18, commit `2917866`), on a GitHub-hosted `ubuntu-latest` runner
(4 vCPU, 15 GiB RAM, Linux 6.17-azure):

- 3va: this repo, release build (`cargo build --release`)
- Node.js 24.19.0
- Bun 1.3.14

| Runtime | Startup (mean) | HTTP throughput (c=1000) | Memory, idle → post-load | Install, warm |
|---|---|---|---|---|
| Node.js 24.19 | 28.2 ms | 19,056 req/s | 48.0 MB → 77.2 MB | — |
| Bun 1.3.14 | 15.4 ms | 54,112 req/s | 41.8 MB → 50.7 MB | — |
| **3va** | 25.8 ms | 6,899 req/s¹ | 50.8 MB → 87.0 MB | 24.8 ms |

¹ With `3va.config.json`'s opened-up firewall limits — see above.

GitHub's shared runners are weaker and noisier than dedicated hardware —
absolute numbers here run meaningfully lower than a quiet dedicated box for
all three runtimes (CPU-bound work like HTTP throughput scales down with
fewer/slower cores; startup latency picks up scheduler noise from
neighboring jobs) — but the workflow re-measures all three runtimes
together on every run, so the *ranking* and relative gaps are the
trustworthy part. Treat single-run absolute figures as a snapshot; rerun
the workflow (`workflow_dispatch`) for a fresh one, or diff against past
runs in the Actions tab for a trend instead of one point in time.

### Memory under load: root cause and fix

*Historical investigation, from a 2026-07-15 run on dedicated hardware
(AMD Ryzen 9 7950X) predating the switch to CI-sourced reference numbers
above — the specific MB figures below won't match the current CI table
(different machine, different point in time), but the root cause and fix
they describe are still exactly what's running today.*

The first version of that run showed 3va's memory growing ~8.5×
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
   `isolate.low_memory_notification()` on a 1-second throttle
   (`LOW_MEMORY_HINT_INTERVAL`) — frequent enough to reclaim memory during
   sustained load, not so frequent that a full GC pause on every event-loop
   tick would hurt throughput. Verified: post-load RSS dropped from 255 MB
   to 93–180 MB across repeated runs (a 30–64% reduction depending on run),
   with throughput unchanged (16,100 req/s @ c=1000, 99.94% success,
   measured before and after) — the fix costs nothing observable and keeps
   most of the win. (The interval was later tightened from 5 s to 1 s: the
   5 s hint never fired during a fast HTTP burst once keep-alive removed the
   per-request TCP handshake cost — the whole 100k-request load finished in
   ~2 s — so the heap hit its burst high-water mark before the first hint.
   At 1 s the hint fires at least once mid-burst, which held post-load RSS
   near the pre-keep-alive figure instead of spiking to the burst watermark.
   The exact post-load number is measured by CI.)

The remaining idle-to-loaded growth (versus Node's ~1.8×) wasn't chased
further — mimalloc is still wired in as the global allocator (a reasonable
default for a busy server generally, even though it didn't fix this
specific bug), and a request-count-based trigger instead of a time-based one
is the next thing to try if this needs to go lower.

## CI

[`../.github/workflows/benchmark.yml`](../.github/workflows/benchmark.yml)
runs this script on `workflow_dispatch`, on release tags, and weekly,
publishing the result table to the workflow's job summary. **As of 2.5.0,
this is where the main README's comparison table numbers come from** —
deliberately, not a reversal of the earlier "dedicated hardware only"
policy by accident: a number anyone can regenerate by clicking "Run
workflow" (or that reruns automatically on a schedule) is more credible
than one only reproducible on a maintainer's specific machine, even though
the dedicated-hardware run above has cleaner absolute figures. Re-run it
yourself, compare against the [linked run](https://github.com/OdinoCano/3va/actions/runs/32180248410)
above, or diff any two runs in the Actions tab for a regression check.
