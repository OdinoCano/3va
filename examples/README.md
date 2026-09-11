# 3va Examples

A collection of small, self-contained projects that demonstrate how to use **3va** — the JavaScript/TypeScript runtime written in Rust with deny-by-default security.

> **Prerequisite:** have the `3va` binary on your `PATH`. See the [Installation guide](../README.md#installation) in the main README.

## Index

| # | Example | What it shows | Command |
|---|---------|---------------|---------|
| [01](./01-hello-world) | Hello World | The simplest script, zero permissions | `3va run app.js` |
| [02](./02-http-server) | HTTP Server | Node `http` module + `--allow-net` | `3va run server.js --allow-net=localhost` |
| [03](./03-file-ops) | File Operations | Scoped `--allow-read` / `--allow-write` | `3va run app.js --allow-read=./data.json --allow-write=./data.json` |
| [04](./04-typescript) | TypeScript | Running `.ts` natively, no build step | `3va run math.ts` |
| [05](./05-packages) | npm Packages | `3va install` + package permissions | `3va install` then `3va run app.js` |
| [06](./06-dev-server) | Dev Server | On-demand serving + full-page HMR | `3va dev --port 3000` |
| [07](./07-testing) | Test Runner | Jest-compatible tests, no config | `3va test` |
| [08](./08-bundler) | Bundler | Multi-file → single self-contained file | `3va bundle src/index.js -o dist/bundle.js --minify` |
| [09](./09-process-manager) | Process Manager | Background daemon + auto-restart | `3va start server.js --name demo-api` |
| [10](./10-permissions) | Permissions | Deny-by-default demo, grants one by one | `3va run demo.js` |
| [11](./11-react-native) | React Native | Run & test the shared TS logic of an RN app | `3va test` |
| [12](./12-expo) | Expo | Expo helpers + scripts; `3va create expo-app` | `3va run src/index.ts --allow-read=./src` |
| [13](./13-tauri) | Tauri | Frontend TS toolchain for a Tauri shell (dev/bundle/test) | `3va dev` / `3va bundle` / `3va test` |

## The core idea of 3va

**Deny everything, grant explicitly.** Every capability — filesystem, network, env vars, child processes, native addons — is blocked by default. You declare what each script may touch at the command line:

```bash
3va run app.ts \
  --allow-read=./data \
  --allow-write=./output \
  --allow-net=api.example.com \
  --allow-env=DATABASE_URL
```

This applies uniformly to your app code **and** to every dependency it pulls in.

## Common aliases

`r` = run · `i`/`add` = install · `t`/`spec` = test · `d` = dev · `b` = bundle · `ws` = workspace · `sh`/`shell` = sandbox

## Not sure which flags your script needs?

```bash
3va permissions learn app.ts   # runs it, observes usage, prints exact flags
3va permissions suggest app.ts # static analysis of the source, no execution
```

## Suggested order

Start with the basics, then mix and match:

1. **01** hello world → 3. **03** files → 2. **02** HTTP server (the two most common capabilities)
4. **04** TypeScript if you write `.ts`
5. **07** add tests to your project
6. **05** bring in dependencies
7. **06** develop with HMR, **08** ship a bundle
8. **09** deploy as a daemon
9. **10** to understand why any of the above need their flags

## Framework examples

**11 · 12 · 13** show how 3va fits next to a native app framework. The rule of thumb
for all three: *3va runs the pure JavaScript/TypeScript side* — shared logic,
tooling scripts, the frontend — while the native shell (RN bridge, Metro,
or the Tauri Rust `src-tauri/`) does its own thing:

- **[11](./11-react-native)** — React Native: reducers, selectors and validation
  TS puro compartido entre app y backend, ejecutado y testeado con `3va`.
- **[12](./12-expo)** — Expo: helpers de theme/texto/fecha + scripts (`3va run`),
  y `3va create expo-app` para generar una app Expo real.
- **[13](./13-tauri)** — Tauri v2: el frontend TypeScript como `beforeDevCommand`
  / `beforeBuildCommand` de Tauri: `3va dev` (Vite-style), `3va bundle` y
  `3va test` para la lógica de comandos sin compilar Rust.