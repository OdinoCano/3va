# Differential suite (`node` vs `3va`)

The same scenario runs with `node` (24) and with `3va`, and the runner fails when
the output differs. It is step 1 of the roadmap in
[`docs/09-testing/07-compatibilidad-node.md`](../../docs/09-testing/07-compatibilidad-node.md)
(§7.4) and implements rules §7.3.2 (server and client in **separate OS
processes**) and §7.3.4 (binary data compared by SHA-256).

test262 covers the ECMAScript language; this suite covers the Node API layer
(`http`, `stream`, `Buffer`, `net`, `tls`, `fetch`, the module loader) that 3va
reimplements and test262 never touches. Between 2.12.1 and 2.12.6 a production
Express client hit bugs that no existing test could see; a run of this suite
against the **2.12.0** binary fails 19 of the 25 scenarios.

## Run it

```sh
scripts/differential.sh                 # whole suite
scripts/differential.sh express4 fetch  # only matching scenarios
```

Environment:

| Variable | Meaning |
|---|---|
| `THREEVA_BIN` | 3va binary under test (default `target/release/3va`, else `3va`) |
| `NODE_BIN` | `node` binary (default `node`; Node 24) |
| `DIFFERENTIAL_FILTER` | comma-separated substrings (same as CLI args) |
| `DIFFERENTIAL_OUT` | output directory (default `target/differential`) |
| `DIFFERENTIAL_REFRESH` | `1` to force `npm ci` even when `node_modules` exists |
| `DIFFERENTIAL_KEEP` | `1` to keep previous runs' results |

The runner copies the scenarios and `lib/` into `target/differential/work`,
installs the pinned dependencies there with `npm ci` (never in the repo) and
generates a throwaway CA under `certs/`. Per-run outputs (stdout/stderr, exit
code) land in `target/differential/results/<scenario>/`; that is what you diff
when something fails.

Exit code is non-zero when a scenario differs from `node` and is not listed as a
known difference, or when a listed known difference stops reproducing.

## Layout

```
tests/differential/
  package.json / package-lock.json   pinned scenario dependencies (npm ci)
  known-differences.json             accepted differences + their reason
  lib/                               shared servers and clients
  scenarios/<name>/
    scenario.json                    manifest
    server.js  client.js             kind: client-server
    main.js / main.mjs               kind: single
```

`scenario.json`:

```json
{
  "kind": "client-server",
  "timeout_s": 30,
  "server_perms": ["--allow-net=127.0.0.1", "--allow-read=."],
  "client_perms": ["--allow-net=127.0.0.1", "--allow-read=."],
  "env": "NODE_EXTRA_CA_CERTS=certs/ca.pem",
  "main": "main.mjs",
  "combos": ["node/node", "3va/node", "node/3va", "3va/3va"]
}
```

`kind` defaults to `client-server`, the four combos default to those above
(`node` then `3va` for `kind: single`), and the permissions default to
`--allow-net=127.0.0.1 --allow-read=.`.

## Determinism rules

The comparison is **byte for byte**, so a scenario must print the same bytes on
every run and on both runtimes:

- the port is chosen by the runner and passed on `argv`, never printed;
- a server signals readiness by printing exactly `READY`;
- no timestamps, PIDs, random boundaries or hash-map iteration order in the
  output: files are identified by name/size/SHA-256, never by the multipart
  boundary, and objects are printed with an explicit key order;
- binary is compared by SHA-256 or hex, never through `String(buffer)`.

## Known differences (the ratchet)

A real difference may be accepted while it is being fixed. It is listed in
`known-differences.json` with its reason, is reported separately and does not
fail the run:

```json
{ "esm-mjs-import": { "3va": "…reason…" } }
```

Use `"*"` as the combo to cover every non-baseline combo. The **ratchet** is the
other half: if a listed difference stops reproducing (3va starts matching node),
the run **fails** until the entry is removed, so the list cannot rot. Entries
whose combo is not run, or whose scenario no longer exists, fail too.

Each entry carries the concrete finding, so it can be turned into an issue:

| Scenario | Combo | Finding |
|---|---|---|
| `esm-mjs-import` | `3va` | a relative import of a `.mjs` file fails (`require() of ES module … not supported`) |
| `esm-cyclic-import` | `3va` | import cycles do not expose live bindings (`undefined` instead of the value node sees) |
| `http-status-codes` | `3va` | `http.STATUS_CODES[415]` is missing (node: `Unsupported Media Type`) |
| `http-request-streaming` | 3va client | an incrementally-written body is framed with `Content-Length` instead of `Transfer-Encoding: chunked` |
| `tls-private-ca` | 3va client | `https`/`tls` ignore `NODE_EXTRA_CA_CERTS` (native-tls/OpenSSL path); `fetch` does honour it |

## Adding a scenario

1. `mkdir scenarios/<name>` and add `scenario.json` (and `server.js`/`client.js`
   or `main.js`).
2. Keep the printed output deterministic (see above).
3. Add dependencies to `package.json` with an **exact** version and run
   `npm install --package-lock-only` to refresh the lockfile.
4. Run `scripts/differential.sh <name>`; it must match on all combos, or the
   difference must be added to `known-differences.json` with its reason.

## CI

`.github/workflows/ci.yml` runs the suite (node 24, debug `3va`) on every push
and pull request. The runner takes ~30 s; the job's cost is dominated by the 3va
build, which the shared Rust cache covers.
