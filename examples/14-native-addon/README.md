# 14 — Native addon (Node-API)

Loads a real native addon, `bcrypt`, through 3va's Node-API (`napi_*`) layer.

## Setup

```bash
3va install --allow-net=registry.npmjs.org
```

## Run

```bash
3va run app.js \
  --allow-ffi \
  --allow-read=. \
  --allow-read=/usr/bin/ldd \
  --allow-read=/etc/alpine-release \
  --allow-env=ELECTRON_RUN_AS_NODE
```

## What it demonstrates

- `--allow-ffi` is required for any `.node` file. A native addon runs **outside** the
  sandbox, so grant it only for addons you trust.
- Why the other flags: `bcrypt` locates its prebuilt binary with `node-gyp-build`, which
  reads `/usr/bin/ldd` and `/etc/alpine-release` to tell glibc from musl, and checks
  `ELECTRON_RUN_AS_NODE`. Deny-by-default shows you exactly which reads a package makes.
- Synchronous and asynchronous (`napi_create_async_work`) calls into native code.

## Compatibility notes

- Linux and macOS prebuilds ship inside the package, so no compiler is needed.
- `bcrypt` (node-addon-api) and `@node-rs/argon2` (napi-rs) both load and give the same results as
  Node, synchronous and asynchronous. Other addons may still hit a Node-API function 3va does not
  export (an `undefined symbol` error at load time). `napi_wrap` finalizers run when the wrapped
  object is collected and when the environment is torn down, so native memory an addon attaches to
  a JS object with `napi_wrap` is freed.
