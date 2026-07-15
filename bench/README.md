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
| Node.js 24.17 | 14.3 ms | 73,050 req/s | 45.9 MB → 81.8 MB | — |
| Bun 1.3.14 | 6.7 ms | 133,720 req/s | 38.5 MB → 48.7 MB | — |
| **3va** | 28.0 ms | 15,817 req/s¹ | 30.3 MB → **255.1 MB** | 11.7 ms |

¹ With `3va.config.json`'s opened-up firewall limits — see above.

Run-to-run variance on throughput was double-digit percent even on an
otherwise idle machine (single-run figures above, not averaged across
repeats) — treat the ranking as reliable and the exact req/s as a snapshot,
not a guarantee.

**3va's memory under load is a real finding worth flagging, not noise.**
It was the lowest of the three at idle and the highest by far once loaded:
~8.5× growth from idle to post-load, versus ~1.8× for Node and ~1.3× for
Bun. This reproduced consistently across repeated runs (255.1 MB and 254.0
MB on back-to-back runs) — it's not a one-off spike. This script doesn't
diagnose *why* (per-connection buffer sizing, the firewall's own
connection-tracking state scaling with the raised `maxConnectionsPerIp`,
or something else); that's an open question for whoever picks it up next,
not something to paper over in the README.

## CI

[`../.github/workflows/benchmark.yml`](../.github/workflows/benchmark.yml)
runs this script on `workflow_dispatch` and on release tags, publishing the
result table to the workflow's job summary. GitHub-hosted runners have
noisier, weaker hardware than a dedicated machine, so treat CI's absolute
numbers as regression signal against *previous CI runs*, not as the
headline figures — the reference run above is what should back README claims.
