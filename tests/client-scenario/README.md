# Client scenario — release gate

A pass/fail scenario that reproduces a production client and runs with the
**released** 3va binary. Step 5 of the roadmap in
[`docs/09-testing/07-compatibilidad-node.md`](../../docs/09-testing/07-compatibilidad-node.md)
(§7.4): if it fails, the release does not ship.

## What it exercises

An Express 4 app (`app/`) that a real client would run:

- `express.json()`, `express-session` login (`POST /login`, `GET /me`);
- a multer upload (`POST /upload`) whose bytes are downloaded back and compared
  by SHA-256 (`GET /download/:id`);
- an outgoing **axios** call to a mock upstream (`GET /upstream`);
- **pg against PostgreSQL 16 with SCRAM-SHA-256** (the `pg` pool);
- **top-level await** in `db.js`: the pool connects and seeds the table *before*
  the app starts serving (the shape of a real `db/pg.js`);
- everything behind an **nginx** reverse proxy; `3va.config.json` sets
  `firewall.trustedProxies`, and `GET /ip` returns the address 3va resolved
  from `X-Forwarded-For` (nginx pins `203.0.113.7` for that route).

The app runs under 3va inside `debian:bookworm-slim` (glibc 2.36, the oldest
supported floor; `nodejs`/`npm` are only used at image-build time to run
`npm ci`). A node client (`client/`) drives the whole flow through nginx and
exits non-zero if any check fails.

## Run it

```sh
THREEVA_BIN=/path/to/3va scripts/client-scenario.sh
```

`THREEVA_BIN` must be a binary built for glibc ≤ 2.36 — a **published** release
binary or the one the release workflow builds on `ubuntu-22.04`. A plain local
`cargo build` links the host's glibc and will not run in `bookworm-slim`;
`SCENARIO_BASE_IMAGE=debian:trixie-slim` overrides the base image for that case.

Requires Docker with the Compose plugin. The script copies the binary into the
build context, builds, brings the stack up, waits for the client's exit code and
tears everything down.

## Result

| Binary | Base | Result |
|---|---|---|
| 2.12.6 (published) | `debian:bookworm-slim` | **15/15 checks pass** |
| 2.12.0 (published) | `debian:trixie-slim`¹ | **fails** |

¹ The published 2.12.0 binaries were built on glibc 2.39 (before the release
workflow's glibc-floor check existed), so they do not start in `bookworm-slim`.
On a newer base the failure is functional and is the one this scenario is meant
to catch: the DB module's top-level `await` does not block startup, so the seed
table is never created (`relation "seed" does not exist`) and the upload route
closes the connection (`502`). The 2.13.0 binary built by the release workflow
runs in `bookworm-slim`, so the gate uses that base by default.

## CI

`.github/workflows/release.yml` runs it in the `client-scenario` job right after
the Linux binaries are built, using the just-built x86_64 binary. The
`github-release` job depends on it, so a failure blocks the GitHub Release and
the crates.io publish.
