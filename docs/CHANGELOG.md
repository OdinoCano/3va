# Changelog

All notable changes to **3va** are documented here.
Format: [Keep a Changelog 1.0.0](https://keepachangelog.com/en/1.0.0/) · Versioning: [SemVer](https://semver.org/).

---

## [Unreleased]

### Fixed (major)

- **`3va start` was a fire-and-forget daemon with no supervision, and `3va audit`'s heuristic scan
  drowned real vulnerabilities in false positives.** Three related fixes:

  1. **`.exec()` false positives in the malware heuristic scanner** — any `.exec(...)` call, on any
     object, was flagged as `"Process execution"` (Critical), intending to catch
     `child_process.exec()` but also firing on `someRegex.exec(str)` — routine in parser-heavy
     packages like `mime`/`finalhandler` — producing dozens of false positives per audit. Fixed:
     `.exec(...)` alone no longer triggers unless the file also references `child_process`;
     `.execSync(...)`/`.spawn(...)` still trigger unconditionally (no `RegExp` counterpart to be
     confused with). (`crates/pm/src/malware_scanner.rs`)
  2. **`3va audit`'s two phases were ordered and labeled backwards** — the heuristic scan (pattern
     matches, not confirmed malware) printed first and was called "Threats", ahead of the OSV
     scan (real, published CVEs/GHSAs) that `--deny` actually gates on. Reordered: OSV prints first;
     the heuristic phase is now labeled "Heuristic Pattern Scan (informational)" (JSON key
     `"heuristic"`, not `"malware"`) and only a Critical-severity hit fails the command.
     (`crates/pm/src/lib.rs`, `crates/cli/src/main.rs`)
  3. **`3va start` had no crash recovery and couldn't run as a container's `PID 1`** — it spawned
     `3va run <entry>` once, detached, and never looked at it again; a crashed process just stayed
     dead, and because the daemon always forked-and-exited, using it as a Docker `CMD` killed the
     container immediately. Added a real supervisor (`3va __supervise`, internal): watches the app
     and restarts it (linear backoff, capped by `--max-restarts`, default 15) on unexpected exit.
     `3va start --attach`/`-a` runs the same supervisor in the foreground instead of daemonizing —
     use this as a container's `CMD`/`ENTRYPOINT` (a valid `PID 1` that never exits on its own).
     `3va start --instances N`/`-i` runs N app instances load-balanced on one port via
     `SO_REUSEPORT` (`VVVA_CLUSTER=1`, cluster-mode only — normal single-instance binds still fail
     loudly with `EADDRINUSE` on an accidental double-start), mirroring `pm2 -i`/Node's `cluster`
     module. `3va status` now shows an `Inst` column and a `crashed` status for a supervisor that
     gave up after `--max-restarts`. (`crates/cli/src/proc.rs`, `crates/cli/src/main.rs`,
     `crates/js/src/builtins/http_server.rs`)

- **`package.json["3va"].permissions`'s `allow-net` silently failed to let a script's own server
  start** — `http.createServer()`/`net.createServer()` default their bind host to `"0.0.0.0"` when
  `.listen(port)` is called without an explicit host, but the permission check compared that literal
  `"0.0.0.0"` against whatever host was actually granted (e.g. `allow-net: ["127.0.0.1"]`), which
  never matched — `.listen()` failed with no visible error (an unhandled rejection inside the async
  listen call). Read from the outside, it looked like `package.json` permissions weren't being read
  at all; they were, the exact-match model for a bind host just didn't make sense. Added
  `PermissionState::check_bind(host)` (`crates/permissions/src/capability.rs`), used only at
  `listen()` call sites (`crates/js/src/builtins/http_server.rs`,
  `crates/js/src/builtins/tcp.rs`'s `__netListen`): when the bind host is a wildcard/loopback
  address (`0.0.0.0`, `::`, `127.0.0.1`, `::1`, `localhost`), any existing `allow-net` grant
  authorizes the bind. Outbound checks (`fetch`, `http.request`, TCP connect, PQ-TLS) are
  unaffected — they still call the strict `check()`, so `allow-net: ["api.example.com"]` still
  cannot reach `127.0.0.1` or any other unlisted host (no SSRF regression).

- **`3va bundle` did not actually bundle multiple files** — `Bundler::add_entry()` only ever
  registered the entry file; `bundle()` never walked the import graph to discover, resolve, and
  inline anything it imported. `extract_imports()` found `import`/`require()` specifiers via regex
  but the result was only ever used for the code-splitter's dependency map — nothing acted on it to
  grow the module set. `generate_iife()` (the CLI's only reachable output format — `--format` isn't
  exposed) additionally only took *one* arbitrary module out of the set and discarded the rest. Net
  effect: any project with more than one file (i.e. any real project) produced a bundle containing
  a literal, unresolved `import` statement inside an IIFE function body — invalid syntax (`import`
  is only legal at module top level), confirmed failing `node --check`. Only a single file with zero
  imports ever worked.

  Added a real graph-walking bundler (`bundle_graph` in `crates/bundler/src/lib.rs`, wired into
  `bundle_file` for the default `Iife` format and no `--split`): BFS from the entry, each module
  transpiled to a CommonJS-shaped function body (`vvva_js::transpiler::transpile_to_cjs` — JSX/TS
  stripped in the same pass; already-CommonJS `node_modules` packages are left as-is), every
  `require(...)` call rewritten from its original specifier text to the target's resolved canonical
  absolute path (which doubles as its registry key), assembled into a `__modules[id] = function
  (module, exports, require) {...}` registry with a small `require()`/cache runtime, closing over
  the entry. `.json` becomes `module.exports = <parsed JSON>`; `.css` injects a `<style>` tag when a
  DOM is present and no-ops otherwise (bundles primarily target `3va run`, a non-browser context);
  image/font assets embed their original path as a string (not copied/hashed — no production asset
  pipeline yet). Verified against a real multi-file project with both an ESM and a CommonJS
  `node_modules` dependency, a JSON import, and a default-exported component: the output passes
  `node --check` and actually runs correctly via `3va run`, producing real interop results instead
  of a parse error. ponytail-scoped: tree-shaking, code-splitting (`--split`), and source maps
  (`--source-map`) are **not** implemented for this new path — only the pre-existing single-file
  `Bundler` API (unreachable from the CLI) still has them, now clearly a separate, older code path.
  (`crates/bundler/src/lib.rs`, `crates/bundler/Cargo.toml` — `vvva_js` moved from a dev- to a
  regular dependency)

- **`3va bundle` failed with "No such file or directory" if the output directory didn't exist** —
  `bundle_file` now creates it (`std::fs::create_dir_all`) before writing, matching every other
  bundler (Vite/webpack/esbuild all do this). (`crates/bundler/src/lib.rs`)

- **Two real bugs found in `vvva_js::transpiler`'s ESM→CommonJS converter** while building the
  bundler above (used by both `transpile_to_cjs` and, indirectly, `3va dev`'s CJS interop shim) —
  both were pre-existing, not introduced by the bundler work:
  1. `export default function main() {...}` compiled to `module.exports["default"] = main();` —
     **calling** the function eagerly at module-load time and exporting its return value, instead
     of exporting the function itself. Root cause: the default-export name extractor used
     `w.trim_end_matches('(')` to strip call parens from a token like `"main()"`, but that only
     strips a trailing `(` — the token *ends* with `)`, so nothing was ever stripped. Fixed to
     split on the first `(` and keep the identifier before it.
  2. `import X from "mod"` (a *simple* default import — by far the most common import form in any
     React codebase, e.g. `import App from "./App"`) compiled to `var X = require("mod");` with no
     `.default` unwrapping, silently binding `X` to the *whole CJS exports object* instead of the
     default export value — breaking `App()`/`<App />` at runtime for essentially every
     `export default function App() {...}` file. The *combined* import form
     (`import X, { a } from "mod"`) already correctly applied `(_tmp.default !== undefined) ?
     _tmp.default : _tmp` interop a few lines up in the same function; the simple form was the odd
     one out. Fixed to apply the same interop.
  (`crates/js/src/transpiler.rs`, `convert_export`/`convert_import`)

### Added

- **`package.json["3va"].permissions` is now actually read by `3va run`** — this closes the gap
  where the entry below (2.1.0, "`package.json#3va.permissions`") described a feature that was
  never wired into `build_permissions()`; `ThreeVaConfig`/`PackagePermissionScope` never existed
  in the codebase, and the README correctly listed the feature as "(v2.1 roadmap)" the whole time.
  This entry documents what's actually implemented now, in `crates/cli/src/main.rs`:
  - Per-scope `allow-read` / `allow-write` / `allow-net` / `allow-env` / `allow-ffi` /
    `allow-child-process`, merged across all scopes (scope keys are for human readability only —
    the capability engine has no per-module isolation) and unioned with `--allow-*` CLI flags
    (CLI only adds; it cannot revoke a `package.json` grant).
  - `deny-read` / `deny-write` / `deny-net` / `deny-env` / `deny-ffi` / `deny-child-process` per
    scope, checked before any `allow-*` — lets a broad directory-prefix grant exclude one file
    (e.g. a dependency file with a known CVE).
  - Relative paths in `allow-*`/`deny-*` resolve against the directory containing `package.json`,
    not the invocation `cwd`.
  - `${VAR}` expansion in path fields, against the environment of the host process launching
    `3va run` (not the sandboxed script's `process.env`), so one absolute root that differs per
    server/team can be declared once and switched per environment. Undefined variables are left
    as a literal placeholder (fails closed).
  - Top-level `"3va": { "no-prompt": true }`, equivalent to the new `--no-prompt` CLI flag on
    `3va run` (see below).
  - See `docs/06-permissions/06-package-json-permissions.md` for the full reference.

- **`3va run --no-prompt`** — forces `interactive = false` regardless of TTY state, so any
  capability not covered by `allow-*` is denied silently instead of prompting. Previously the
  only way to suppress prompts in an attended terminal was redirecting stdin from `/dev/null`.
  (`crates/cli/src/main.rs`)

- **`3va dev` entry discovery covers `.jsx`/`.tsx` and the `main.*` naming convention** —
  previously only `src/index.ts`, `src/index.js`, `index.ts`, `index.js` were tried, so a typical
  Vite/CRA-style React app (`src/main.jsx`) failed with "Entry file not found". Now also tries
  `src/index.tsx`, `src/index.jsx`, `src/main.ts`, `src/main.tsx`, `src/main.js`, `src/main.jsx`,
  `index.tsx`, `index.jsx`. (`crates/cli/src/main.rs`, `ENTRY_CANDIDATES`)

- **`3va dev` on-demand ESM serving (Vite-style)** — a root-level `index.html` referencing
  `<script type="module" src="/src/main.jsx">` now works directly, without a hand-written
  `public/index.html` pointing at `/bundle.js`. `.js`/`.jsx`/`.ts`/`.tsx` requests are transpiled
  per request (JSX/TS stripped via the existing `vvva_js::transpiler`) and their import
  specifiers rewritten (`rewrite_imports`/`rewrite_specifier`) to browser-resolvable URLs:
  project-relative imports stay project-relative, bare specifiers and anything under
  `node_modules` resolve via `vvva_js::esm::resolve_esm` to `/@fs/<abs-path>`. `import "./x.css"`
  is wrapped in a tiny style-injecting module (`?import` query); a `.css` path requested directly
  (a `<link>` tag) is served as raw CSS. The full-bundle path (`dist/bundle.js` via
  `vvva_bundler::bundle_file`, served at `/bundle.js`) is unchanged and still runs on start/on
  every source change, for a `public/index.html` that references it directly. The previous
  built-in fallback page (served when no `index.html` exists anywhere) was also fixed to actually
  execute the bundle (`<script src="/bundle.js">`) instead of only linking to it.
  (`crates/cli/src/main.rs`)

### Fixed

- `docs/06-permissions/05-interactive-prompts.md` no longer describes `--no-prompt` and
  `package.json` permission declarations as future/roadmap items.

- **`3va dev` shutdown could hang for the full 30s drain timeout even after repeated Ctrl+C** —
  the drain loop only polled `active_conns` on a timer and never listened for another `ctrl_c()`
  signal, so a long-lived connection (the `/__hmr` SSE stream a browser tab keeps open) pinned
  every shutdown at 30s regardless of how many times the user pressed Ctrl+C. A second Ctrl+C
  during drain now forces immediate shutdown. (`crates/cli/src/main.rs`, `run_dev_server`)

- **Extensionless relative imports of `.jsx`/directory-`index.jsx` silently failed to resolve** —
  `resolve_relative()` in `crates/js/src/esm.rs` (used by both the runtime's own ESM loader and
  the new `3va dev` on-demand serving above) probed `["js", "mjs", "ts", "tsx", "cjs"]` when an
  import had no extension, missing `jsx` entirely, and only checked `index.js`/`index.mjs`/
  `index.ts` for directory imports, missing `index.jsx`/`index.tsx`. `import App from "./App"`
  (no extension — the common React/Vite convention) resolving to `App.jsx` returned the original,
  nonexistent extensionless path instead. In `3va dev` this surfaced as a browser console error —
  *"Failed to load module script: … MIME type of text/html"* — because the dev server, unable to
  find the file, fell through to serving the SPA-fallback `index.html` for what the browser had
  requested as a `<script type="module">`. Fixed by adding `jsx` to the extension probe list and
  `index.jsx`/`index.tsx` to the directory-index list. Also hardened `3va dev` itself: a request
  path with a `.js`/`.jsx`/`.ts`/`.tsx`/`.css` extension that still resolves to nothing now
  returns a plain `404` instead of silently falling through to the SPA-fallback HTML, so a future
  resolution gap fails loudly instead of producing this same confusing MIME-mismatch error.
  (`crates/js/src/esm.rs`, `crates/cli/src/main.rs`)

- **`3va dev` on-demand serving of CommonJS `node_modules` deps threw `does not provide an
  export named 'X'`** — third-party packages are frequently CommonJS (`react-router-dom`'s `main`
  build, for example), which has no ES `export` statements at all. Serving that source verbatim
  as a `<script type="module">` executed without error but exposed zero named exports, so
  `import { Link } from "react-router-dom"` failed at the browser's module-linking stage. Added a
  CJS→ESM interop shim (`serve_cjs_interop`, `find_require_specifiers`, `find_cjs_export_names`
  in `crates/cli/src/main.rs`): CJS files (detected via the now-`pub` `vvva_js::esm::source_is_esm`)
  are wrapped in `(function(module, exports, require) { … })(...)`, with `require()` targets
  statically found and routed through the same on-demand resolution (recursing through more CJS
  interop if needed) and each statically-found `exports.NAME =` / `module.exports.NAME =` /
  `Object.defineProperty(exports, "NAME", …)` assignment re-exported as a named ES binding.
  Respects the Babel/TS `__esModule` interop marker for `export default`. ponytail-scoped: static
  text scan only, so a computed `require(someVar)` or dynamically-built export object isn't
  discoverable — throws a clear "unresolved require" error naming the file rather than failing
  silently; see the `ponytail:` comment on `serve_cjs_interop` for the upgrade path (pre-bundle
  the dependency with `vvva_bundler` instead) if a real package needs it.

  **Follow-up correction:** the static-scan-only version above still missed named exports for a
  real `react-router-dom` build, because tsc/Babel-compiled "barrel" files re-export everything
  from sub-modules via a runtime `__exportStar(require("./x"), exports)` copy loop (`for...in`
  over the required module) rather than a literal `exports.NAME = ` line anywhere in the source —
  invisible to any static text scan. Replaced with `discover_cjs_export_names_dynamic`: the
  wrapped CJS body is actually executed in a throwaway, sandboxed `rquickjs::Context::full`
  (2-second interrupt-bounded), with each `require()` target *recursively* resolved and
  discovered first (ESM deps via the new `find_esm_export_names`, CJS deps via this same function
  one level deeper, capped at 4 levels with a visited-set cycle guard) so the stub `require()`
  returns a real, correctly-keyed object for `__exportStar`'s loop to actually copy from — not a
  content-free placeholder. `resolve_cjs_export_names` unions this dynamic result with the
  original static scan (`find_cjs_export_names`) as a fallback for when execution fails outright
  (parse error, timeout, depth exhausted). `rquickjs` is now a direct dependency of `vvva_cli`
  (`crates/cli/Cargo.toml`) for this. (`crates/cli/src/main.rs`)

  **Second follow-up correction:** still failed against a *real* `react-router-dom@6.30.4`
  install (verified against the actual published package, not a synthetic repro). Two bugs, both
  fixed in `discover_cjs_export_names_dynamic`:
  1. `dist/main.js` does `module.exports = require(dev ? "./umd/x.dev.js" : "./umd/x.min.js")` —
     a whole-module conditional re-assignment to one of two sibling UMD builds, evaluated correctly
     once `process.env` was stubbed as a plain `{}` (already handled). But both UMD siblings
     `require("react")`/`require("react-dom")`, and `visited` was a single `HashSet` *mutated
     across siblings* rather than cloned per branch — after processing the first sibling, `react`
     was marked visited, so the second sibling's own `require("react")` was wrongly treated as an
     already-visited cycle and stubbed empty. `visited` is now passed as an immutable ancestor-path
     set, cloned (not shared) into each child call — siblings depending on the same package is
     normal, not a cycle. Added
     `dynamic_discovery_does_not_treat_a_shared_sibling_dependency_as_a_cycle` reproducing this
     with two tempfile siblings sharing a dependency.
  2. Even with (1) fixed, required-module stub *values* were plain `function(){}` (returns
     `undefined` when called). Real top-level code routinely uses what it requires immediately —
     `React__namespace.createContext(x).displayName = "y"` is exactly what a real
     `react-router-dom` UMD build does — and assigning a property onto `undefined` throws,
     aborting that module's entire discovery. Stub values are now `__stub()`, a self-referential
     `Proxy` (get/apply/construct traps all yield another `__stub()`) that tolerates being called,
     indexed, and assigned into indefinitely, rather than a fixed no-op function.
  (`crates/cli/src/main.rs`)

- **`3va dev` on-demand-served packages threw `Uncaught ReferenceError: process is not defined`
  in the browser** — separate from the two discovery-time bugs above (which only affected the
  server-side probe used to compute export *names*), the actual shim sent to the browser had no
  `process` global at all, and browser-targeted "development" builds (React's included) routinely
  check bare `process.env.NODE_ENV` with no `typeof` guard at module top level. Added a minimal
  `process` shim (`env.NODE_ENV = "development"`, `browser: true`) to `HMR_CLIENT_JS`
  (`crates/cli/src/main.rs`) — a classic (non-module) `<script>` already injected into every served
  HTML page, so it lands on `window` once and every ES module/CJS-interop shim loaded afterward
  sees the same `process` without redeclaring it per file. See `docs/compatibility.txt` for why
  `process` is safe as a pure-JS, no-native-binding polyfill.

- **`3va dev`-served `.jsx`/`.tsx` threw `Uncaught ReferenceError: React is not defined`** for any
  file using JSX without an explicit `import React from "react"` — the automatic-runtime
  convention nearly every current React/Vite scaffold relies on (JSX "just works" with no React
  import). `vvva_js::transpiler::transpile_jsx` compiles JSX to the *classic* runtime
  (`React.createElement(...)`), which assumes `React` is already in scope; it doesn't add the
  import itself. Added `inject_react_import_if_needed` (`crates/cli/src/main.rs`): after
  transpiling, if the output references `React.createElement` and the original source never
  imported `React`, prepends `import React from "react";` (which then flows through the normal
  `rewrite_imports` pass like any other import, resolving to the real installed `react` package).
  Does not touch `vvva_js::transpiler` itself, or the `3va run`/`3va test` code paths that also
  call it — scoped to the dev-server-only shim step.

### Changed

- **`3va dev` startup banner now matches Vite's `Local:`/`Network:` convention** instead of
  printing a single `Ready:` line. With the default `--host 127.0.0.1` (loopback-only, matching
  Vite's own default), prints `Local:` plus a `(!) Use --host to expose on the network` hint —
  same as Vite, no dead `Network:` link for a server nothing outside the machine can reach. With
  an explicit non-loopback `--host` (e.g. `0.0.0.0`), prints `Local: http://localhost:PORT` and
  `Network: http://<lan-ip>:PORT` so another device on the same network (a phone, for a quick
  mobile check) has a real address to use. The LAN IP comes from `primary_lan_ip()`
  (`crates/cli/src/main.rs`) — the standard zero-dependency `UdpSocket::connect` route-picking
  trick, not real interface enumeration, so it reports one IP (the one on the route to the public
  internet), not every NIC the way Vite's list sometimes does for Docker bridges/extra VPN
  adapters; real multi-NIC enumeration needs `getifaddrs` or a new dependency and wasn't judged
  worth it for a startup banner.

### Added (continued)

Found by cloning the real Vite and pnpm source into `.compatibility/` and comparing their actual
dev-server/resolution behavior against `3va dev`:

- **`import img from "./logo.png"` / `import data from "./x.json"` now work** — previously the raw
  file bytes were served with `Content-Type: application/javascript` (image) or `application/json`
  (JSON) for a `<script type="module">` request, which the browser's strict MIME-type check
  rejects; these imports silently didn't work. `rewrite_specifier` now appends the same `?import`
  marker already used for `import "./x.css"` to any specifier resolving to a
  `DEV_STATIC_ASSET_EXTENSIONS` file or `.json`, and `serve_dev_source` returns `export default
  "<url>"` for assets (matching Vite's `plugins/asset.ts` convention — a URL string, not the bytes)
  and `export default <parsed JSON>` for JSON (valid JS syntax as-is, no transform needed). A
  direct request for the same path (a `<link>`/`<img>`/browser-typed fetch, no `?import`) still
  gets the raw file, unchanged. (`crates/cli/src/main.rs`)

- **CJS export discovery is now cached** — every request for a `node_modules` CJS dependency
  previously re-ran the *entire* recursive QuickJS discovery tree (`discover_cjs_export_names_dynamic`)
  from scratch, including every nested `require()` target, on every single page load or HMR
  reload; Vite avoids the equivalent redundant work via its on-disk `optimizeDeps` pre-bundle
  cache. Added `CJS_DISCOVERY_CACHE`, a process-lifetime `HashMap<PathBuf, (SystemTime, Vec<String>)>`
  behind a `LazyLock<Mutex<_>>` (stdlib only, no new dependency), checked at every recursion depth
  — not just the outermost per-request call — so a shared sub-dependency like `react`, required by
  many files, pays the QuickJS execution cost once total rather than once per requiring file.
  Invalidated by mtime, so editing a `node_modules` file during a session is picked up on the next
  request. (`crates/cli/src/main.rs`)

- **`3va <name>` falls back to `package.json.scripts.<name>`** when `<name>` isn't one of 3va's own
  subcommands (`3va build`, `3va lint`, etc.) — the same convention `npm run <name>`/`pnpm <name>`
  follow, and the actual reason `3va build` didn't work even though `package.json` declared a
  `"build"` script: 3va had no generic single-project script runner at all (only
  `3va workspace run <script>`, monorepo-only). `Cli::try_parse()` replaces the old `Cli::parse()`
  in `main()`; on `ErrorKind::InvalidSubcommand`, `try_run_package_script` checks
  `package.json["scripts"]` for a matching key and, if found, shells out to the project's actual
  package manager (`pnpm`/`yarn`/`bun`/`npm`, detected by lockfile) — `<pm> run <name>` — exiting
  with the child's exit code, exactly mirroring what `npm run`/`pnpm <name>` would do. Mirrors the
  pattern already used by `vvva_pm::run_workspace_script`, which itself shells out to `npm run`
  rather than reimplementing PATH/`node_modules/.bin` semantics. A real 3va subcommand always wins
  over a same-named script (`3va dev` runs 3va's own dev server, never `scripts.dev`, even if one
  is defined) — only genuinely unrecognized names fall through. (`crates/cli/src/main.rs`)

  **Follow-up correction:** the version above silently ran the delegated script the instant a
  package.json `scripts.<name>` matched — a real philosophy violation, not just a missing feature.
  The delegated process is a full external binary (the real package manager, running arbitrary
  shell) completely outside `vvva_permissions`' capability model, which only governs JS executed
  *inside* 3va's own QuickJS engine — there is no `--allow-*` flag that could meaningfully restrict
  what an already-compiled `vite`/`webpack`/etc. binary does, and auto-running arbitrary shell
  commands the moment someone types `3va <name>` is exactly the class of thing `3va install`
  already refuses to do for postinstall scripts ("There are no exceptions"). Gated behind the same
  interactive-consent pattern used everywhere else in 3va
  (`docs/06-permissions/05-interactive-prompts.md`): a `[y/N]` prompt in a TTY, a hard deny with no
  prompt outside one (CI/pipes — matches `PermissionState::prompt_user`'s own non-interactive
  behavior), and `--yes` or `"3va": { "no-prompt": true }` (reusing the existing
  `read_package_json_permissions`/`no-prompt` mechanism) to skip the prompt deliberately.
  (`crates/cli/src/main.rs`, `try_run_package_script`)

---

## [2.1.0] — 2026-06-22

### Added

- **`timers/promises`** — Full Node.js 18+ timers/promises module. `setTimeout`, `setInterval`, and
  `setImmediate` return Promises/AsyncIterators. All three support `options.signal` (AbortSignal).
  `for await...of setInterval(delay, value)` works correctly. (`crates/js/src/builtins/modules.rs`)

- **`stream/web` (WHATWG Streams)** — `ReadableStream`, `WritableStream`, and `TransformStream`
  globals (from v1.0.0) now also accessible via `require('stream/web')` subpath, consistent with
  Node.js 16+ API. `[Symbol.asyncIterator]()`, `tee()`, `pipeTo()`, `pipeThrough()` all available.

- **`dns` module (full)** — `dns.lookup()`, `dns.resolve()`, `dns.resolve4()`, `dns.resolve6()`,
  `dns.promises.*` via `tokio::net::lookup_host` (A/AAAA). `resolveMx`/`resolveTxt`/`resolveSrv`/
  `resolveNs`/`resolveCname`/`resolveNaptr`/`resolvePtr`/`resolveSoa`/`reverse` now issue real
  DNS queries via `hickory-resolver` (native `__dnsQuery`) instead of returning stubbed empty
  results. Requires `--allow-net`.

- **Network protocol modules** — New built-in modules for IRC, FTP, POP3, MQTT, SSH/SFTP, and WebRTC.
  **`irc`, `ftp`, `pop3`, `mqtt`, and `ssh` do real network I/O**:
  - `irc.Client` — RFC 2812 line protocol (real socket connect, PING→PONG, PRIVMSG/JOIN/PART parsing)
  - `ftp.Client` — RFC 959 commands (real socket connect, USER/PASS auth, PASV data channel, LIST/RETR/STOR)
  - `pop3.Client` — RFC 1939 line protocol (real socket connect, USER/PASS, LIST/RETR/DELE)
  - `mqtt.connect()` — MQTT 3.1.1 binary protocol (real socket connect, CONNECT/SUBSCRIBE/PUBLISH,
    QoS 0 only — no PUBACK/PUBREC/PUBREL/PUBCOMP handshake, no PINGREQ keepalive)
  - `ssh.Client` — real SSH via `russh` (password auth, `exec()` with real stdout/stderr/exit code)
    and `russh-sftp` (readdir/readFile/writeFile/mkdir/rmdir/unlink/rename/stat over a real SFTP
    subsystem channel). Host key verification is not implemented — any server key is accepted
    (`StrictHostKeyChecking=no` equivalent); no public-key auth yet, password only.

  `webrtc` is **mocked** — API surface matches `RTCPeerConnection`/`RTCDataChannel` but no real
  ICE/STUN/TURN/DTLS/SRTP (requires P2P signaling infrastructure).

  All modules gate `connect()` behind `Capability::Network(host)`. (`crates/js/src/builtins/`)

- **`readline` module** — `createInterface()`, `Symbol.asyncIterator()`, `question()`,
  `pause()`/`resume()`, `setPrompt()`, `write()`, and `line`/`close`/`SIGINT` events.
  `readline.promises` namespace also implemented. `process.stdin` is backed by real OS stdin
  (native `__stdinRead`, blocking read on a background thread) and `Interface` consumes it via
  Node-style `'data'`/`'end'` events, in addition to the WHATWG `getReader()` path.

- **`package.json#3va.permissions`** — New manifest field for declaring required permissions in
  `package.json`. Supports `Capability::Network(host)` with `grant`/`deny`/`prompt`. Enables
  `--package-json` flag on `permissions suggest`/`learn`. Added `ThreeVaConfig` and
  `PackagePermissionScope` types. (`crates/pm/src/manifest.rs`, `crates/cli/src/main.rs`)
  **Correction:** this entry described work that never shipped — `ThreeVaConfig` and
  `PackagePermissionScope` are not present in this codebase, and `3va run` did not read
  `package.json` permissions until the [Unreleased] entry above. Left here unmodified for
  historical accuracy of what this changelog said at the time.

- **`3va create <template>`** — New CLI subcommand for scaffolding projects. Supported frameworks:
  nuxt, solid, redwood, refine, next, astro, remix, svelte. Uses official framework scaffolders
  via `npx`/`create-*`. Auto-grants `--allow-net --allow-child-process` for scaffolding.
  (`crates/cli/src/main.rs`)

- **`--heap-snapshot[=<path>]`** — New CLI flag on `3va run`. Writes a Chrome DevTools Memory
  panel-compatible `.heapsnapshot` JSON file after script execution. Default path:
  `heap-<timestamp>.heapsnapshot`. Uses QuickJS memory APIs and global object enumeration.
  (`crates/js/src/lib.rs`, `crates/cli/src/main.rs`)

- **SLSA Level 2 provenance** — Release workflow (`release.yml`) now generates cryptographically
  verifiable provenance for all binaries using `slsa-github-generator`. Binaries are signed
  with `cosign sign-blob` via OIDC keyless signing. Provenance (`.intoto.jsonl`) and signatures
  (`.sig`) are uploaded as release assets.

- **Automated security audit** — New `.github/workflows/security-audit.yml` runs `cargo audit`
  and `cargo deny check` every Monday at 9:00 AM UTC. On failure, automatically opens a GitHub
  Issue with the audit output. Also runs on push to main.

### Changed

- **Build reproducibility** — Release builds now use `cargo build --release --locked` to ensure
  deterministic builds from the committed `Cargo.lock`.

- **`process.version` / `process.versions['3va']`** — Bumped to `2.1.0`.

### Security

- See [SECURITY.md](./SECURITY.md) for RUSTSEC-2023-0071 status.

---

## [v2.1.3] — 2026-06-22

### Fixed

- **Clippy warnings** — Resolved `dead_code`, `await_holding_lock`, and `manual_strip` lints
  across builtin modules:
  - `grpc.rs`: Changed `rx` from `std::sync::Mutex` to `tokio::sync::Mutex` to fix
    `await_holding_lock` lint during `recv().await`
  - `imap.rs`: Added `#[allow(dead_code)]` on unused `Logout` variant and `send_command` method;
    fixed `manual_strip` to use `strip_prefix()`
  - `irc.rs`, `mqtt.rs`, `ssh.rs`, `webrtc.rs`: Added `#[allow(dead_code)]` on unused struct fields
  - Test files: Added `#[allow(dead_code)]` on `engine_with_net` helper functions

### Changed

- **`process.version` / `process.versions['3va']`** — Bumped to `2.1.3`.

### Security

- See [SECURITY.md](./SECURITY.md) for RUSTSEC-2023-0071 status.

---

## [2.0.4] — 2026-06-19

### Added

- **`localStorage` / `sessionStorage`** — Web Storage API globals. `sessionStorage` is in-memory only.
  `localStorage` is backed by `~/.local/share/3va/localStorage.json` (persisted across runs, path
  overridable via `3VA_LOCALSTORAGE_PATH`). Both follow the standard `getItem`/`setItem`/`removeItem`/
  `clear`/`key`/`length` API. (`crates/js/src/builtins/modules.rs`, `crates/js/src/builtins/process.rs`)

- **`URLPattern`** — Web URL Pattern API global. Supports `:param` named groups, `*` wildcards, and
  `{optional}?` segments. `test(url)` returns boolean; `exec(url)` returns matched groups per URL
  component. Accepts full URL strings, relative pathnames, and `URLPatternInit` objects.
  (`crates/js/src/builtins/modules.rs`)

- **`EventSource` (Server-Sent Events)** — Real SSE client backed by a Rust background thread.
  Connects to any HTTP SSE endpoint, dispatches `message`, `open`, `error`, and custom event types.
  Supports `onmessage`, `onerror`, `onopen`, `addEventListener`, and `close()`.
  (`crates/js/src/builtins/event_source.rs`)

- **`node:sqlite`** — Built-in SQLite via `rusqlite` (SQLite compiled into the binary, no system
  dependency). `new DatabaseSync(path)` opens a connection; `db.exec(sql)`, `db.prepare(sql)` →
  `StatementSync` with `.run(params)`, `.get(params)`, `.all(params)`. Matches Node 22.5+ API.
  (`crates/js/src/builtins/sqlite.rs`)

- **`3va watch <file>`** — New CLI command. Runs a file and restarts it on source changes using the
  `notify` crate. Accepts `--allow-read/net/write/env` and `--delay` (debounce ms, default 300).
  (`crates/cli/src/main.rs`)

- **`fs.cp()` / `fs.cpSync()`** — Recursive directory copy. (`crates/js/src/builtins/fs.rs`)

- **`EventEmitter.once` / `EventEmitter.on` static** — Node 11.13+/12.16+ static helpers:
  `EventEmitter.once(emitter, event)` → Promise; `EventEmitter.on(emitter, event)` → AsyncIterator.

- **`http.globalAgent` / `https.globalAgent`** — Stub matching Node.js API shape.

- **`process.resourceUsage()`** — Returns `userCPUTime`, `systemCPUTime`, `maxRSS`, and counters.

- **Real brotli compression** — `zlib.brotliCompress/Decompress` now use the `brotli` crate instead
  of the previous gzip alias.

### Fixed

- **`URLPattern` matching** — Three bugs corrected: non-specified URL parts defaulted to `/^$/`
  (never matched) instead of `*`; relative strings in `exec()` threw instead of matching pathname;
  full-URL string `init` was parsed as pathname-only. Functional test suite added and registered.

### Dependencies

- `brotli = "7"`, `rusqlite = { version = "0.32", features = ["bundled"] }` added to workspace.

---

## [2.0.3] — 2026-06-19

### Added

- **`fs.cpSync` / `fs.cp` recursive copy** — new `__fsCpSync` native function backs recursive directory copy.
  `fs.cpSync(src, dest)` and `fs.cp(src, dest, cb)` now support copying both files and directories
  recursively. Permission checks for read (`src`) and write (`dest`) are enforced. (`crates/js/src/builtins/fs.rs`)

- **`EventEmitter.once` / `EventEmitter.on` static helpers** — `EventEmitter.once(emitter, event)` returns
  a `Promise` that resolves with the arguments of the next emission. `EventEmitter.on(emitter, event)` returns
  an `AsyncIterator` for streaming events — matching Node.js 12.16+ API. (`crates/js/src/builtins/modules.rs`)

- **`http.globalAgent` / `https.globalAgent`** — exposed as `{ maxSockets: Infinity, maxFreeSockets: 256, keepAlive: false }`
  stub, matching Node.js API shape. (`crates/js/src/builtins/modules.rs`)

- **`process.resourceUsage()`** — returns a Node.js-compatible object with `userCPUTime`, `systemCPUTime`,
  `maxRSS`, and remaining system resource counters derived from `process.cpuUsage()` and `process.memoryUsage()`.
  (`crates/js/src/builtins/process.rs`)

- **Real brotli compression** — `zlib.brotliCompress` / `zlib.brotliDecompress` (async) and
  `zlib.brotliCompressSync` / `zlib.brotliDecompressSync` (sync) now use actual brotli via the
  `brotli 7` crate instead of the previous gzip fallback. (`crates/js/src/builtins/zlib.rs`, `Cargo.toml`)

### Changed

- `brotli` crate added as workspace dependency (`brotli = "7"`).

## [2.0.0] — 2026-06-04

### Added

- **`3va.config.ts` / `.js` / `.json` project config** — new `vvva_config` crate loads a config file by walking up from the project root. All CLI commands pick up defaults from the config; CLI flags always override. Config-file `.ts`/`.js` object literals are parsed without JS execution (sandboxed static analysis). `3VA_<SECTION>_<KEY>` environment variables override config-file values. New `3va config [key] [--check]` subcommand shows the resolved config.

- **Real `worker_threads` (OS-thread isolation)** — `new Worker(file, { workerData })` now spawns an OS thread with its own `JsEngine` instance and Tokio runtime. Message passing uses `std::sync::mpsc` channels bridged via `__workerCreate` / `__workerSend` / `__workerRecv` / `__workerTerminate` native functions. `parentPort.postMessage` inside workers pushes to the main thread's poll queue. `SharedArrayBuffer`/`Atomics` are a declared non-goal (incompatible with per-thread QuickJS isolation).

- **`dgram` UDP sockets** — `require('dgram').createSocket('udp4'|'udp6')` returns a real UDP socket backed by `std::net::UdpSocket`. Full `bind`, `send`, `close`, `address` API. Incoming datagrams are received on a background thread and polled from JS via `setInterval`. Requires `--allow-net`.

- **Content-Security-Policy for `3va dev`** — all HTML responses from the development server now include a `Content-Security-Policy` header by default. The default policy is `default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:`. Disable with `--no-csp`; configure in `3va.config.ts` under `dev.csp`.

- **PQ crypto API alignment** — `pq.kem.generateKeyPair` and `pq.dsa.generateKeyPair` (camelCase) are the new primary names. `generateKeypair` kept as a deprecated alias. `pq.dsa.sign({ key, data })` and `pq.dsa.verify({ key, data, signature })` named-object forms added alongside the old positional form.

- **Parallel test execution** — `3va test --concurrency=N` runs up to N test files concurrently, each in its own `JsEngine` instance. Default: number of logical CPUs. `TestConfig.concurrency` field added to the Rust API.

- **Mock API (`3va:test`)** — `require('3va:test')` exposes `mock.fn(impl?)`, `mock.method(obj, name, impl?)`, and `mock.timers.{enable, tick, reset}`. Each spy tracks `.mock.calls[i].{arguments, result, error}`. `mock.method` restores the original via `.mock.restore()`. Also aliased as `require('node:test')`.

- **JUnit XML and TAP reporters** — `3va test --reporter=junit` emits JUnit XML (Jenkins/GitHub Actions compatible); `--reporter=tap` emits TAP version 13; `--reporter=dot` emits dot format; `--reporter=json` emits JSON array. Default: terminal output.

- **Topological workspace script execution** — `3va workspace run <script>` now builds a dependency DAG from `workspace:*` entries and runs scripts in topological order (leaves first). `--affected [--base=main]` detects packages changed since the merge base via `git diff --name-only`. `--concurrency=N` limits parallel execution slots.

- **Workspace dependency graph** — `3va workspace graph` prints an ASCII DAG with dependency arrows.

- **REPL plugins** — `3va sandbox --plugin=inspect` and `--plugin=history` load built-in plugins. Pass a file path to load a custom plugin. Plugins register dot-commands (e.g. `.inspect <expr>`) dispatched by the REPL loop.

- **Migration tool (`3va codemod`)** — `3va codemod --from=1 --to=2 ./src` applies AST-level source transforms: renames `pq.kem.generateKeypair` → `generateKeyPair`, `pq.dsa.generateKeypair` → `generateKeyPair`, rewrites positional `pq.dsa.sign(k, d)` → `sign({ key, data })`, and `pq.dsa.verify(k, d, s)` → `verify({ key, data, signature })`. `--dry-run` previews changes; `--revert` restores `.bak` backups.

### Changed

- All crate versions bumped from `1.0.0` to `2.0.0`.
- `process.version` and `process.versions['3va']` updated to `2.0.0`.
- `WorkspaceAction::Run` now builds a topological graph and uses `vvva_pm::WorkspaceGraph` internally. Old sequential execution replaced.

### Security

- CSP header enabled by default in `3va dev` to reduce XSS risk in development-served HTML.
- `worker_threads` Workers inherit a read-only copy of the parent `PermissionState`; parents cannot elevate worker permissions after creation.

---

## [1.0.0] — 2026-06-04

### Added

- **CDP Inspector (`--inspect`)** — WebSocket Chrome DevTools Protocol server. `debugger;` statements are rewritten to `__3va_debugger__()` at source-load time. Pause is implemented via `tokio::task::block_in_place` + `Condvar` so the Tokio runtime remains responsive. Connect with `chrome://inspect` or any DAP-compatible IDE.
- **NAPI native module loading (`--allow-ffi`)** — ~30 NAPI v8 functions exposed as `unsafe extern "C"`. `.node` addons loaded via the standard `require('./addon.node')` path. Requires `--allow-ffi` permission.
- **WebAssembly (WASM)** — WASI-preview1-compatible runtime via `wasmtime`. Supports `.wasm` and `.wat` files. Full permission integration (filesystem preopens, env vars scoped to granted capabilities).
- **Post-quantum TLS (`__pqTlsConnect`)** — hybrid classical TLS + ML-KEM-768 key exchange. Returns `{ connId, pqSharedSecret }`. Non-blocking: runs inside `spawn_blocking`. Requires `--allow-net`.
- **Post-quantum crypto JS API** — `require('crypto').pq.kem.{generateKeypair,encapsulate,decapsulate}` (ML-KEM-768) and `require('crypto').pq.dsa.{generateKeypair,sign,verify}` (ML-DSA-65).
- **Fuzz targets in CI** — 3 fuzz targets built on nightly; 30 s smoke run in GitHub Actions.
- **Doc-tests** — public API surfaces of `vvva_core`, `vvva_permissions`, `vvva_crypto`, and `vvva_js` now have doc-tests.
- **`SECURITY.md`** — explicit acceptance rationale for RUSTSEC-2023-0071 (Marvin Attack); documents "before 1.0" review requirement.
- **Process manager subcommands** — `start`, `stop`, `restart`, `status`, `logs`, `delete` for running scripts as managed background daemons.

### Fixed

- `__pqTlsConnect` was synchronous on the JS event loop (blocked all timers and I/O during TLS handshake). Now runs inside `spawn_blocking` and is registered as `Async`.
- `SemverRange` silently rejected `"latest"`, `"1.x"`, `">=1.0.0 <2.0.0"` forms — added dist-tag, x-range, and compound-range support.
- Dependency resolver produced non-deterministic lockfiles (HashMap iteration order). Resolution stack is now sorted alphabetically.
- Version conflict in the resolver was silent — now emits a structured `tracing::warn!`.
- `Content-Length` header forwarded to JS did not reflect the 100 MiB internal cap.

### Changed

- `rquickjs-core 0.6.2` vendored at `vendor/rquickjs-core/` with a one-line fix for the `never type fallback` future-incompatibility lint (Rust Edition 2024).
- All crate versions bumped to `1.0.0`.

### Added

- **Expo / React Native package support** (`crates/js/src/builtins/modules.rs`, `crates/js/src/transpiler.rs`):
  Real Expo npm packages (`expo-modules-core`, `expo-constants`, `expo-asset`, `expo-font`, `expo-file-system`) load and execute without errors. Covered by `crates/js/tests/pipeline.rs` (ESM/CJS) and `crates/js/tests/framework_compat.rs` (import.meta).

  *ESM→CJS converter fixes:*
  - **Circular dependency guard** — `module.exports` pre-cached before eval; circular requires get the partial exports object instead of re-executing the module (matches Node.js behavior, eliminates stack overflows).
  - **`export default X` chained assignment** — was setting `.default` on the OLD `module.exports` object due to JS LHS-ref evaluation order. Now uses two statements: `module.exports = X` + deferred `module.exports.default = module.exports`.
  - **Destructuring exports** — `export const { a, b } = X` and `export const [a, b] = X` now correctly emit individual `module.exports.a = a` entries.
  - **Uninitialised var export** — `export var X;` is now deferred so TypeScript enum IIFEs can fill the value before `module.exports.X` is set.
  - **Empty export marker** — `export {}` (OXC emits this to tag a file as ESM) is now a no-op; previously surfaced as "unsupported keyword: export".
  - **Dynamic `import()`** — inline `import(specifier)` expressions are rewritten to `__importAsync(specifier)` which wraps synchronous `require()` in a resolved Promise.
  - **Deferred exports** — all deferred assignments wrapped in `try{}catch{}` to tolerate read-only properties defined by `Object.defineProperty`.

  *Platform-aware extension resolution:*
  - `resolve_file_path` probes `.web.js`, `.web.tsx`, `.web.ts`, `.web.mjs` before the generic `.js`, `.tsx`, `.ts` variants. Expo `.web.*` files are the correct choice in a server/CLI context (they avoid native bridge imports).
  - Index file probing follows the same order (`index.web.ts` before `index.ts`).

  *TypeScript transpiler:*
  - `SemanticBuilder::with_enum_eval(true)` added — prevents OXC panic when transforming TypeScript `const enum` declarations.

  *New React Native / Expo polyfills:*
  - `react-native` pre-cached in `__requireCache` with `Platform`, `NativeModules`, `TurboModuleRegistry`, `PixelRatio`, `Dimensions`, `StyleSheet`, `Animated`, and all major component stubs.
  - `@react-native/assets-registry` pre-cached with `registerAsset` / `getAssetByID`.
  - `NativeModules` proxy changed to return `undefined` for unregistered module names (previously returned a truthy function-proxy, causing `JSON.parse(function(){})` errors in `expo-constants`).
  - `expo-modules-core` polyfill extended with `NativeModule`, `SharedObject`, `SharedRef` (extendable base classes), `registerWebModule`, `Platform`, `uuid`.
  - `requireOptionalNativeModule` returns `null` — correct for a web/server environment where optional native modules are absent.
  - `process.env.EXPO_OS = 'web'` — Expo packages branch on this to select server-safe code paths.

- **CPU sampling profiler (`--prof`)** (`crates/js/src/profiler.rs`):
  - `3va run app.ts --prof` — collects samples every `--prof-interval` ms (default 10) via `setInterval` + `new Error().stack`; writes V8-compatible `.cpuprofile` JSON.
  - `--flamegraph=<path>` — also emits an Inferno-style SVG flamegraph using the `inferno` crate.
  - `3va prof <file>` subcommand — post-hoc analysis: prints top-N hot functions by self% or re-generates a flamegraph from an existing `.cpuprofile`.
  - `console.profile(label)` / `console.profileEnd(label)` — JS-side region markers, active when `--prof` is passed.
  - `JsEngine::new_with_profiler(perms, interval_ms)` / `JsEngine::take_profiler()` — public Rust API.
  - 7 unit tests: stack parser, location parser, folded-stacks aggregation, `.cpuprofile` JSON validity, `analyze_cpuprofile`, JS bootstrap interval embedding.



- **`Buffer` como subclase real de `Uint8Array`** (`builtins/buffer.rs`):
  Reescrito usando el patrón *prototype swap*: el constructor devuelve un `Uint8Array` real con `Buffer.prototype` en su cadena. Esto garantiza:
  - `buf instanceof Uint8Array` → `true`
  - `buf[0]` → valor de byte correcto (proxy nativo de TypedArray)
  - `[...buf]` spread, `buf.set()`, `Array.from(buf)` — todos funcionan como nativos
  - `DataView`, `Float32Array` y otras vistas sobre `buf.buffer` funcionan sin conversión
  - Compatibles con `ws`, `msgpackr`, `protobufjs` y cualquier librería que accede a bytes directamente
  Todos los métodos (`readUInt32BE`, `writeFloatLE`, `BigUInt64`, `slice`, `subarray`, etc.) actualizados para operar sobre `this` directamente.

- **`crypto.createSign`/`createVerify` — RSA PKCS1v15 y ECDSA reales** (`builtins/crypto.rs`):
  Implementación nativa vía crates Rust. Soporta:
  - **RSA PKCS#1 v1.5**: algoritmos `RSA-SHA256`, `RSA-SHA384`, `RSA-SHA512`, `SHA256`, `SHA1`
  - **ECDSA P-256**: `SHA256` con clave P-256
  - **ECDSA P-384**: `SHA384` con clave P-384
  - Salida en formato **DER** (compatible con `jsonwebtoken`, `passport-jwt`, `jose`)
  - Acepta DER y P1363 (raw r‖s) en verificación
  ```js
  const sig = crypto.createSign('RSA-SHA256').update(data).sign(privateKey);
  crypto.createVerify('RSA-SHA256').update(data).verify(publicKey, sig); // → true
  ```

- **`crypto.createPrivateKey`/`createPublicKey`/`createSecretKey`** (`builtins/crypto.rs`):
  Importa claves PEM existentes en objetos `KeyObject` compatibles con Node.js.
  - `.type` → `'private'`, `'public'`, o `'secret'`
  - `.asymmetricKeyType` → `'rsa'` o `'ec'`
  - `.export()` → PEM string o Uint8Array (con `format: 'der'`)
  Desbloquea: `jsonwebtoken` con claves externas, `passport-jwt`, `@panva/jose`.

- **`crypto.sign`/`crypto.verify` (one-shot, Node.js 15+)** (`builtins/crypto.rs`):
  ```js
  const sig = crypto.sign('SHA256', data, privateKey);
  crypto.verify('SHA256', data, publicKey, sig); // → boolean
  ```

- **`crypto.createHash('md5')`** (`builtins/crypto.rs`):
  MD5 ahora soportado vía crate `md-5 0.10` (algoritmo RustCrypto). Para fingerprinting de contenido,
  ETags, compatibilidad legacy. No recomendado para seguridad.

- **`crypto.getCiphers()`/`getHashes()`/`getCurves()`** (`builtins/crypto.rs`):
  Nuevas funciones de enumeración que devuelven los algoritmos soportados.

- **`crypto.generateKeyPair` / `generateKeyPairSync` — RSA y EC nativos** (`builtins/crypto.rs`):
  Generación de pares de claves asimétricas vía Rust (`rsa 0.9`, `p256 0.13`, `p384 0.13`).
  - `crypto.generateKeyPairSync('rsa', { modulusLength: 2048 })` → `{ publicKey, privateKey }` con `.export()` que devuelve PEM PKCS#8/SPKI.
  - `crypto.generateKeyPair('rsa', opts, callback)` — versión async con spawn_blocking.
  - Curvas EC soportadas: `P-256` (`prime256v1`), `P-384` (`secp384r1`).
  - Claves RSA-PSS: misma implementación que RSA estándar.
  Desbloquea: JWT RS256/ES256/ES384 con `jsonwebtoken`, `passport-jwt`, `node-jose`.

- **`crypto.webcrypto`** (`builtins/crypto.rs`):
  Añadido `crypto.webcrypto = { subtle }` como alias al `crypto.subtle` existente.
  Requerido por Hono, edge runtimes, y cualquier código que accede a WebCrypto vía `require('crypto').webcrypto`.

- **`crypto.scryptSync` — implementación real con scrypt** (`builtins/crypto.rs`):
  Sustituye la aproximación anterior (PBKDF2 como fallback) por `__cryptoScryptSync`, que llama
  directamente a la implementación nativa `scrypt::scrypt`. Nuevo binding Rust síncrono análogo a `__cryptoPbkdf2Sync`.

- **`child_process.execSync` / `spawnSync`** (`builtins/child_process.rs`):
  Implementación real que bloquea el hilo llamante vía `std::process::Command::output()`.
  - `execSync(cmd, opts)` — devuelve stdout como Buffer o string; lanza en exit ≠ 0.
  - `spawnSync(cmd, args, opts)` — devuelve `{ status, stdout, stderr, pid, signal, error }`.
  - Ambos respetan el sistema de capabilities: requieren `--allow-child-process`.
  Desbloquea: Vite/esbuild postinstall, Prisma query engine bootstrap, CLIs con Node.js.

- **`util.parseArgs`** (`builtins/modules.rs`):
  Implementación completa del API de Node.js 18+ para parseo de argumentos CLI.
  Soporta: `--flag`, `--key=value`, `--key value`, flags booleanos, valores múltiples, positionals, `--`, defaults, y el campo `tokens` opcional.

- **`reflect-metadata` polyfill** (`builtins/modules.rs`):
  Polyfill JS completo de la API `Reflect.metadata` para decoradores TypeScript.
  Implementa: `defineMetadata`, `getMetadata`, `getOwnMetadata`, `hasMetadata`, `hasOwnMetadata`,
  `deleteMetadata`, `getMetadataKeys`, `getOwnMetadataKeys`, `decorate`.
  Accesible vía `require('reflect-metadata')`. Desbloquea: NestJS, TypeORM, tsyringe, routing-controllers.

### Fixed

- **`assert.deepStrictEqual` — implementación completa** (`builtins/modules.rs`):
  La implementación anterior usaba `JSON.stringify` que fallaba con:
  - Valores `undefined` (eliminados por JSON)
  - TypedArrays (`Uint8Array`, `Int32Array`, etc.)
  - Referencias circulares
  - Objetos `Date`, `RegExp`, `Map`, `Set`
  La nueva implementación hace comparación estructural recursiva con:
  - Detección de ciclos vía lista de pares visitados
  - Soporte para `Date` (comparación por timestamp), `RegExp` (por string), `TypedArray`, `Map`, `Set`
  - Semántica estricta (`===`) vs no-estricta (`==`) según el método
  También añadidos: `notDeepStrictEqual`, `notStrictEqual`, `ifError`, `fail`.

- **`Buffer.isBuffer(x)` ahora devuelve `true` para `Uint8Array` nativo** (`builtins/buffer.rs`):
  Anteriormente devolvía `false` para `Uint8Array` no envuelto en `Buffer`, rompiendo librerías que
  hacen `if (!Buffer.isBuffer(x)) throw`. Ahora `Buffer.isBuffer(new Uint8Array(4)) === true`.

- **`util.inspect` — manejo de referencias circulares y `Symbol.for('nodejs.util.inspect.custom')`** (`builtins/modules.rs`):
  La implementación anterior hacía `JSON.stringify` que lanzaba en objetos circulares. Ahora:
  - Detecta ciclos y muestra `[Circular *]`.
  - Llama `obj[Symbol.for('nodejs.util.inspect.custom')]` si existe (requerido por pino, winston, etc.).
  - Formatea funciones como `[Function: name]`, fechas como ISO, y errors como `[ErrorType: message]`.
  - Limita la profundidad (2 niveles por defecto, configurable con `{ depth: n }`).

- **Framework detection — `3va dev` ahora detecta y delega a 8 frameworks** (`crates/cli/src/main.rs`):
  `3va dev` detecta automáticamente el framework del proyecto y delega en su dev server nativo.
  - Astro (`astro.config.*` → `astro dev`)
  - Next.js (`next.config.*` → `next dev`)
  - Nuxt (`nuxt.config.*` → `nuxi dev`)
  - SvelteKit (`svelte.config.*` + `@sveltejs/kit` → `vite dev`)
  - Remix (`remix.config.*` → `remix dev`)
  - Gatsby (`gatsby-config.*` → `gatsby develop`)
  - SolidStart (`app.config.*` → `vinxi dev`)
  - Qwik (`qwik.config.*` → `qwik dev`)
  Los flags `--port`, `--host` y `--open` se reenvían automáticamente al CLI del framework.

- **Process manager nativo — comandos `start`, `stop`, `restart`, `status`, `logs`, `delete`** (`crates/cli/src/proc.rs`, `crates/cli/src/main.rs`):
  Nuevo sistema de gestión de procesos en producción similar a PM2:
  - `3va start <entry>` — inicia un proceso como daemon (nuevo session group vía `setsid`).
  - `3va stop <name>` — detiene con SIGTERM → SIGKILL tras 1.5 s.
  - `3va restart <name>` — reinicia con la misma configuración.
  - `3va status [name]` — muestra estado de procesos con códigos de color.
  - `3va logs <name>` — muestra las últimas N líneas del log.
  - `3va delete <name>` — elimina permanentemente el proceso y sus logs.
  Los metadatos se almacenan en `~/.3va/processes/<name>.json` y los logs en `~/.3va/processes/<name>.log`.

- **`EventEmitter` — API completa** (`modules.rs`):
  Nuevos métodos que muchos paquetes npm dan por sentados:
  - `prependListener(event, fn)` / `prependOnceListener(event, fn)` — agregan listeners al inicio de la cola (en lugar del final).
  - `rawListeners(event)` — devuelve los wrappers internos de `once` tal como están (a diferencia de `listeners()` que los desenvuelve).
  - `eventNames()` — array con todos los eventos que tienen listeners registrados.
  - `getMaxListeners()` — devuelve el límite configurado con `setMaxListeners()`.
  - `EventEmitter.listenerCount(emitter, event)` — método estático de compatibilidad Node.js.
  - `EventEmitter.defaultMaxListeners` — propiedad estática equivalente a `EventEmitter.setMaxListeners`.
  - `listeners()` corregido: ahora devuelve la función original (no el wrapper de `once`).

- **`zlib` — Transform streams reales** (`builtins/zlib.rs`):
  Las funciones `createGzip`, `createGunzip`, `createDeflate`, `createInflate`, `createDeflateRaw`, `createInflateRaw` ya no devuelven objetos vacíos. Ahora devuelven **Transform streams** con:
  - `write(chunk[, enc, cb])` — comprime/descomprime asíncronamente; emite `data` con el resultado.
  - `end([chunk][, cb])` — espera a que todos los `write()` pendientes completen antes de emitir `finish`/`end`.
  - `pipe(dest)` / `unpipe(dest)` — encadenamiento estándar de streams.
  - `on/once/off/emit` — interfaz EventEmitter completa.
  - `pause()`, `resume()`, `destroy()`, `setEncoding()`.
  - Propagación correcta al destino (`pipe`) en eventos `data` y `end`.
  - Brotli: `createBrotliCompress` / `createBrotliDecompress` (alias sobre gzip — compresión real pendiente).

- **`zlib` — métodos síncronos reales** (`builtins/zlib.rs`):
  `gzipSync`, `gunzipSync`, `deflateSync`, `inflateSync`, `deflateRawSync`, `inflateRawSync`,
  `brotliCompressSync`, `brotliDecompressSync` — ya no lanzan "not available". Están respaldados
  por las mismas funciones Rust (`flate2`) pero ejecutadas de forma síncrona (sin `spawn_blocking`).
  Útiles en transformaciones de build-time.

- **`process` — EventEmitter completo** (`builtins/process.rs`):
  El objeto `process` ahora expone la API EventEmitter completa:
  `on`, `once`, `off`, `emit`, `removeListener`, `removeAllListeners`, `addListener`,
  `prependListener`, `prependOnceListener`, `rawListeners`, `eventNames`, `listenerCount`.
  Los listeners de señales (`SIGINT`, `SIGTERM`, etc.) se registran con la misma API.

- **`process.memoryUsage()` — valores reales en Linux** (`builtins/process.rs`):
  Lee el RSS real de `/proc/self/status`. Devuelve `{ rss, heapTotal, heapUsed, external, arrayBuffers }`.
  `process.memoryUsage.rss()` — atajo directo al RSS.

- **`process.cpuUsage([prev])` — valores reales en Linux** (`builtins/process.rs`):
  Lee tiempos de CPU de `/proc/self/stat`. Devuelve `{ user, system }` en microsegundos.
  Acepta un valor previo para obtener el diferencial.

- **`process.uptime()`** — segundos transcurridos desde el inicio del proceso.
- **`process.title`**, **`process.execPath`**, **`process.execArgv`** — propiedades estándar.
- **`process.abort()`** — llama a `process.exit(1)`.
- **`process.kill(pid)`** — llama a `process.exit(0)` si `pid === process.pid`.
- **`process.report`** — objeto stub compatible con `--report-*` de Node.js.
- **`process.allowedNodeEnvironmentFlags`** — `Set` vacío (compatibilidad).
- **`process.setUncaughtExceptionCaptureCallback(fn)`** / `hasUncaughtExceptionCaptureCallback()`.

- **`os` — valores reales del sistema** (`builtins/process.rs` + `modules.rs`):
  - `os.hostname()` — nombre real del host vía `gethostname(3)`.
  - `os.totalmem()` / `os.freemem()` — memoria real de `/proc/meminfo` en Linux.
  - `os.uptime()` — uptime real de `/proc/uptime` en Linux.
  - `os.platform()` / `os.arch()` — derivados de `process.platform` / `process.arch`.
  - `os.homedir()` / `os.tmpdir()` — respetan `process.env.HOME` / `process.env.TMPDIR`.
  - `os.EOL` — `'\r\n'` en Windows, `'\n'` en Unix.
  - `os.availableParallelism()`, `os.getPriority()`, `os.setPriority()`, `os.machine()`.
  - `os.constants.signals`, `os.constants.errno`, `os.constants.priority` con valores correctos.
  - `os.userInfo()` — respeta `process.env.USER` y `process.env.HOME`.

- **`path` — reescritura completa** (`modules.rs`):
  Implementación generada por `makePath(sep, isAbsFn)` — soporta posix y win32 con la misma lógica:
  - `path.relative(from, to)` — antes devolvía `to` sin modificar. Ahora calcula la ruta relativa real con `..`.
  - `path.normalize(p)` — colapsa `.` y `..` correctamente.
  - `path.resolve(...parts)` — sube hasta encontrar una parte absoluta o usa `process.cwd()`.
  - `path.posix` — submódulo con separador `/` (mismo objeto en Linux/macOS).
  - `path.win32` — submódulo con separador `\` e `isAbsolute` con regex `C:\`.
  - `require('path/posix')`, `require('node:path/posix')`, `require('path/win32')`, `require('node:path/win32')`.
  - `path.toNamespacedPath(p)` — identidad (no-op en POSIX).
  - `path.matchesGlob()` — stub (devuelve `false`).

- **`fs` — operaciones basadas en file descriptor** (`builtins/fs.rs`):
  Respaldadas por una tabla de FDs Rust (`Arc<Mutex<FdTable>>`) con `std::fs::File` reales:
  - `fs.open(path, flags[, mode], cb)` / `fs.openSync(path, flags[, mode])` → `fd` entero.
    Flags de texto: `'r'`, `'r+'`, `'w'`, `'w+'`, `'a'`, `'a+'`, `'wx'`, `'wx+'`, etc.
  - `fs.close(fd[, cb])` / `fs.closeSync(fd)`.
  - `fs.read(fd, buffer, offset, length, position, cb)` / `fs.readSync(...)` → bytes leídos.
  - `fs.write(fd, buffer, offset, length, position, cb)` / `fs.writeSync(...)` → bytes escritos.
  - `fs.fstat(fd[, cb])` / `fs.fstatSync(fd)` → objeto stat con las mismas propiedades que `statSync`.
  - `fs.fsync(fd[, cb])` / `fs.fdatasync(fd[, cb])` — completado silencioso.
  - `fs.ftruncate(fd[, len][, cb])` / `fs.ftruncateSync`.

- **`fs.mkdtemp(prefix[, opts][, cb])` / `fs.mkdtempSync(prefix)`** — crea un directorio temporal único.

- **`fs.opendir(path[, opts][, cb])` / `fs.opendirSync(path)`** — devuelve un objeto `Dir` con:
  - `read([cb])` — entrada siguiente como `Dirent` o `null` al terminar; también retorna Promise.
  - `readSync()` — variante síncrona.
  - `close([cb])` — cierra el directorio.
  - `[Symbol.asyncIterator]()` — iterable asíncrono compatible con `for await...of`.
  - `[Symbol.iterator]()` — iterable síncrono.

- **`fs` — métodos adicionales** (`modules.rs`):
  `fs.truncate`, `fs.lutimes`, `fs.lutimesSync`, `fs.lchown`, `fs.lchownSync`,
  `fs.chown`, `fs.chownSync`, `fs.fchown`, `fs.fchownSync`,
  `fs.fchmod`, `fs.fchmodSync`, `fs.link`, `fs.linkSync`,
  `fs.readlink`, `fs.readlinkSync`.

- **`stat_meta_to_json` — helper compartido** (`builtins/fs.rs`):
  Función Rust que serializa `std::fs::Metadata` a JSON. Usada por `statSync`, `lstatSync` y el nuevo `fstatSync`.

### Added

- **WinterCG `Headers` class** — `new Headers(init?)` where `init` may be a plain object, a
  `[[key, value]]` array, or another `Headers`. Case-insensitive; iterable via `for..of`,
  `entries()`, `keys()`, `values()`, `forEach()`. `getSetCookie()` returns all `set-cookie`
  values as a separate array. (`modules.rs`)

- **WinterCG `Request` class** — `new Request(url | request, init?)`. Properties: `url`, `method`,
  `headers` (`Headers`), `bodyUsed`, `signal`, `duplex`, `mode`, `credentials`, `cache`,
  `redirect`, `referrer`, `integrity`, `keepalive`. Body methods: `text()`, `json()`,
  `arrayBuffer()`, `bytes()`, `blob()`, `formData()`, `clone()`. `fetch()` now accepts a
  `Request` object as its first argument. (`modules.rs` + `fetch.rs`)

- **WinterCG `Response` class** — `new Response(body?, init?)`. Properties: `ok`, `status`,
  `statusText`, `headers` (`Headers`), `url`, `redirected`, `type`, `bodyUsed`. Body methods
  same as `Request`. Static: `Response.json(data, init?)`, `Response.error()`,
  `Response.redirect(url, status?)`. `fetch()` now returns a `Response` instance instead of a
  plain object. (`modules.rs` + `fetch.rs`)

- **`structuredClone(value)`** — global deep-clone function (JSON round-trip). Throws
  `DataCloneError` for non-serializable values (functions, circular refs), matching browser
  behaviour. (`modules.rs`)

- **`navigator` global** — read-only object with `userAgent` (`'3va/0.1 (QuickJS)'`), `language`,
  `languages`, `onLine`, `hardwareConcurrency`, `platform`, `cookieEnabled`, `doNotTrack`.
  Required by many edge/worker detection checks. (`modules.rs`)

- **`self === globalThis`** — `globalThis.self` is now set to `globalThis`, unblocking worker-
  compat code that checks `typeof self !== 'undefined'`. (`modules.rs`)

- **`require('crypto')` — real implementation** — no longer a placeholder that returns random
  garbage. Now wraps `globalThis.crypto` (the Rust-backed SubtleCrypto). Added:
  `getRandomValues`, `randomBytes`, `randomUUID` (CSPRNG), `createHash(alg)` / `createHmac(alg, key)`
  (async `.digest(enc)` returning a `Promise`), `timingSafeEqual(a, b)`, `pbkdf2(...)`,
  `constants`. (`modules.rs`)

- **`jsr:` specifier support** — `require('jsr:@scope/name')` and ESM `import 'jsr:@scope/name'`
  now resolve by stripping the `jsr:` prefix and looking up the package in `node_modules/` as
  a regular scoped package. Use `3va install @scope/name --allow-net=jsr.io` to install.
  (`modules.rs`)

- **`http.createServer(handler)` — real HTTP/1.1 server** — `require('http').createServer(handler)`
  now binds a real TCP port and serves HTTP/1.1 connections. Backed by `builtins/http_server.rs`
  (Rust, async Tokio listener). Handler receives Node.js-compatible `IncomingMessage` (`req.method`,
  `req.url`, `req.headers`, `req._body`) and `ServerResponse` (`res.writeHead()`, `res.write()`,
  `res.end()`, `res.setHeader()`, `res.statusCode`). Requires `--allow-net=<bind-host>`. Handles
  multiple sequential requests per server instance.

  ```js
  const http = require('http');
  http.createServer((req, res) => {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ path: req.url }));
  }).listen(3000, '0.0.0.0');
  ```

- **ML-KEM-768 (FIPS 203 / Kyber)** — post-quantum Key Encapsulation Mechanism in `vvva_crypto`.
  `MlKemKeypair::generate()`, `encapsulate(&ek)`, `decapsulate(&dk, ct)`. Key sizes: EK=1184 B,
  DK=64 B (seed), CT=1088 B, SS=32 B. Wrong-key decapsulation returns a different shared secret
  (implicit rejection per spec). Hex serialization helpers included.
  (`crates/crypto/src/kem.rs`)

- **ML-DSA-65 (FIPS 204 / Dilithium)** — post-quantum digital signature scheme in `vvva_crypto`.
  `generate_signing_key()`, `sign(&sk, msg)`, `verify(&vk, msg, &sig)`. Key sizes: SK=32 B (seed),
  VK=1952 B, sig=3309 B. Stateless — safe to reuse the same key for multiple messages.
  (`crates/crypto/src/dsa.rs`)

- **`crypto.subtle` (Web Crypto API)** — full `SubtleCrypto` object on `globalThis.crypto.subtle`
  and `require('crypto').subtle`. Backed by `builtins/crypto.rs` (Rust) + JS for HKDF/PBKDF2.
  Supported operations: `digest` (SHA-1/224/256/384/512), `generateKey` (AES-GCM-128/256, AES-CBC,
  AES-CTR, HMAC), `importKey`/`exportKey` (`raw` + `jwk`), `sign`/`verify` (HMAC), `encrypt`/`decrypt`
  (AES-GCM), `deriveBits`/`deriveKey` (HKDF, PBKDF2). `wrapKey`/`unwrapKey` throw `NotSupportedError`.

- **`response.formData()`** — `fetch` responses now parse their body into a `FormData` object.
  Supports `application/x-www-form-urlencoded` (percent-decoding, `+`→space) and
  `multipart/form-data` (boundary splitting, `Content-Disposition` parsing, file parts become `File`
  objects). Any other `Content-Type` rejects with `TypeError`. (`builtins/fetch.rs`)

- **`net` / `tls` — real TCP/TLS sockets** — `require('net')` and `require('tls')` now return
  Rust-backed implementations. `Socket` class wraps `TcpStream` (plain) or `TlsStream` (TLS via
  `native-tls`). API: `connect()`, `write()`, `end()`, `destroy()`, `setEncoding()`, `setTimeout()`,
  `on('data'|'end'|'error'|'close')`, `pipe()`.
  Requires `--allow-net=<host>`. (`builtins/tcp.rs`, `modules.rs`)

- **`net.createServer(handler)` — raw TCP server** — `require('net').createServer(handler)` binds
  a real TCP port and calls `handler(socket)` for each incoming connection. `Socket` exposes
  `write(data)`, `end()`, `on('data'|'end'|'error'|'close')`. Server exposes `listen(port, host)` and
  `server.listening` flag. Backed by `__netListen` / `__netAcceptAsync` in `builtins/tcp.rs`.
  Requires `--allow-net=<bind-host>`.

  ```js
  const net = require('net');
  net.createServer((socket) => {
    socket.write('hello\n');
    socket.end();
  }).listen(4000, '127.0.0.1');
  ```

- **`http2` client** — `require('http2').connect(authority)` returns an `Http2Session`. Sessions
  expose `request(headers)` which returns an `Http2Request` that emits `response`, `data`, and `end`
  events. NGHTTP2 constants available as `http2.constants`. Backed by `__fetchAsync`; does not
  implement real HTTP/2 framing. (`modules.rs`)

- **`--allow-env=VAR[,VAR,...]` scoped environment access** — `--allow-env` now
  accepts an optional comma-delimited list of variable names.
  - `--allow-env=` (no value) → grants `EnvAccess` (all variables, previous behaviour).
  - `--allow-env=NODE_ENV` → grants `EnvVar("NODE_ENV")` only; all other variables
    are hidden from `process.env`.
  - `--allow-env=NODE_ENV,PORT` → grants only the two listed names.
  - Not providing the flag → `process.env` is an empty object `{}`.
- **`Capability::EnvVar(String)`** — new capability variant for per-variable scoping.
  `EnvAccess` (all) covers any `EnvVar(x)` via `caps_match`; the reverse does not hold.
- **`process.env` permission enforcement** — `process.env` is now populated by
  filtering the host environment through `PermissionState::check(&Capability::EnvVar(key))`
  at injection time. Variables not granted are absent from the object regardless of
  whether they exist in the host environment. Previously all variables were exposed
  unconditionally even without `--allow-env`.

### Fixed

- **Package extraction robustness** (`crates/pm/src/fetcher.rs`) — `PackageFetcher::extract`
  no longer aborts the entire installation on the first entry error. Changes:
  - Entries with `..` or absolute path components are rejected (path traversal).
  - Resolved output paths are verified to stay within the destination directory
    (prevents canonical-escape attacks).
  - `EntryType::Symlink` / `EntryType::Link` are always skipped (supply-chain risk).
  - Directory entries are handled with `create_dir_all` rather than `unpack`.
  - Per-entry IO errors are logged as `WARN` and skipped; extraction continues.
  - Fixes silent install failures for large packages that include native code
    (e.g. `react-native`, `canvas`, `sharp`) — these now extract correctly to
    `node_modules/` instead of being left absent.



- **`3va run` script arguments** — arguments after `--` are forwarded to the script via `process.argv[2+]`. Example: `3va run server.ts -- --port 3000 --dev`. `process.argv[0]` = binary path, `process.argv[1]` = absolute script path, `process.argv[2+]` = script args.
- **`3va install` multi-package** — `install` now accepts multiple packages in one invocation: `3va install next react react-dom`. Previously only one package was accepted.
- **`--allow-net=` without value** — passing `--allow-net=` (empty value via `=`) grants network access to all hosts (equivalent to `*`). Same semantics for `--allow-read=` (all paths) and `--allow-write=` (all paths). Multiple flags can be combined: `3va run app.js --allow-net= --allow-read= --allow-write=`.
- **`process.cwd()`** — returns the real working directory. Previously `undefined`.
- **`process.chdir()`** — no-op stub (sandboxed runtime does not change working directory).
- **`process.nextTick(cb, ...args)`** — schedules `cb` in a microtask via `Promise.resolve().then()`, matching Node.js semantics. Multiple callbacks queued in the same tick are flushed in order.
- **`process.hrtime.bigint()`** — returns `BigInt(Date.now()) * 1_000_000n`.
- **`setImmediate` / `clearImmediate`** — exposed as globals; `setImmediate` is backed by `setTimeout(fn, 0)`.
- **`process.versions` expanded** — now includes `node: "20.0.0"`, `v8: "11.3.244.8-node.20"`, `uv: "1.44.2"`, `zlib: "1.2.13"`, `openssl: "3.0.0"`, `modules: "115"`. Packages that inspect `process.versions.node` no longer crash.
- **`process.stdout.fd` / `process.stderr.fd`** — set to `1` and `2` respectively. `isTTY` set to `false`.
- **`global` / `GLOBAL` globals** — `globalThis.global` and `globalThis.GLOBAL` are now aliases for `globalThis`, unblocking packages that use `global.xxx` (e.g. `node-polyfill-crypto`).
- **`require('module')` shim** — the built-in `module` package now exposes `Module._resolveFilename()`, `Module._cache`, `Module._load()`, `Module.prototype.require()`, `Module.createRequire()`, `Module.createRequireFromPath()`, `Module.builtinModules`, `Module.isBuiltin()`, and `Module.syncBuiltinESMExports()`. Required by Next.js `require-hook.js` and many other packages.
- **`fs` expanded** — 15 new functions (sync + async + `fs.promises`):
  - `existsSync(path)` — now exposed on the `fs` object (was on `globalThis` only).
  - `statSync(path)` / `stat(path[, cb])` — returns a stat object with `isFile()`, `isDirectory()`, `isSymbolicLink()`, `size`, `mode`, `mtime`, `atime`, `ctime`, `birthtime`, `mtimeMs`, `atimeMs`, `ctimeMs`.
  - `lstatSync(path)` / `lstat(path[, cb])` — same as `stat` but does not follow symlinks.
  - `accessSync(path[, mode])` / `access(path[, mode][, cb])` — checks existence and sandbox read/write permissions. `mode` flags: `fs.constants.F_OK` (0), `R_OK` (4), `W_OK` (2), `X_OK` (1).
  - `realpathSync(path)` / `realpath(path[, cb])` — calls `std::fs::canonicalize`.
  - `unlinkSync(path)` / `unlink(path[, cb])` — removes a file.
  - `renameSync(from, to)` / `rename(from, to[, cb])` — moves/renames.
  - `copyFileSync(src, dest)` / `copyFile(src, dest[, cb])` — copies a file.
  - `chmodSync(path, mode)` / `chmod(path, mode[, cb])` — changes Unix permissions.
  - `symlinkSync(target, path)` / `symlink(target, path[, cb])` — creates a symlink.
  - `appendFileSync(path, data)` / `appendFile(path, data[, cb])` — appends to a file.
  - `createReadStream(path[, opts])` — returns an EventEmitter that emits `data`/`end`/`error`. Reads are lazy (fired via `setTimeout(0)` so the event loop can drain first).
  - `createWriteStream(path[, opts])` — returns an object with `write(chunk)` and `end([chunk])`. Flushes the entire buffer to disk on `end()`.
  - `watch(path[, opts][, cb])` — returns an EventEmitter stub (no inotify; sandbox limitation).
  - `readdirSync(path, { withFileTypes: true })` — returns `Dirent`-like objects with `name`, `isFile()`, `isDirectory()`, `isSymbolicLink()`.
  - `fs.constants` — `{ F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1, COPYFILE_EXCL: 1 }`.
  - `fs.promises.*` — all async methods mirrored (readFile, writeFile, readdir, mkdir, rm, stat, lstat, access, realpath, rename, unlink, copyFile, chmod, symlink, appendFile).
  - `require('fs')` and `require('node:fs')` now return the full expanded object; `require('fs/promises')` returns `fs.promises`.
- **JSX transform** — the Oxc transpiler now supports JSX via the Classic runtime (`React.createElement`):
  - `.jsx` / `.tsx` files: always transformed.
  - `.ts` / `.mts` / `.cts` files: TypeScript strip only (no JSX).
  - `.js` / `.mjs` / unknown extensions: auto-detection via `looks_like_jsx()` heuristic — if the source contains `<Tag` or `</Tag`, JSX transform is applied automatically.
  - JSX fragments use `React.Fragment`.
  - `transpile_jsx(source)` and `transpile_js(source)` are now public API in `vvva_js::transpiler`.
  - `looks_like_jsx(source) -> bool` is public for callers that want to pre-check.
- **Flow type stripping** — `transpile_js()` includes a two-pass Flow fallback:
  1. Strips `@flow`, `@format`, `import type`, `import typeof` pragmas.
  2. If Oxc still fails, falls back to `strip_inline_flow_types()` which removes `: Type` annotations from `const`/`let`/`var` declarations and function parameters at character level (no regex). Enables basic Flow-annotated `.js` files from React Native packages to be loaded via `require()`.

### Changed

- `--allow-read`, `--allow-net`, `--allow-write` in `run`, `install`, `update`, `reinstall` now use `require_equals = true` and `value_delimiter = ','`:
  - **Old:** `--allow-net registry.npmjs.org` (space-separated, consumed next positional arg as value — broken with `--allow-net` followed by FILE).
  - **New:** `--allow-net=registry.npmjs.org` or `--allow-net=host1,host2` (equals sign required; comma-delimited list; omitting value after `=` grants wildcard).
- `process.argv` construction moved from `inject_process` (captured all raw CLI args) to `eval_file` / `eval_file_with_args`:
  - `process.argv[0]` = path to the `3va` binary.
  - `process.argv[1]` = absolute path to the script being run (set just before execution).
  - `process.argv[2+]` = script arguments passed after `--` (set by `eval_file_with_args`).
- `3va install` `package` field renamed from `Option<String>` to `Vec<String>` (`packages`). Backward-compatible: omitting all packages still installs from manifest.

### Fixed

- `--allow-net=` followed immediately by a positional argument (`<FILE>`) no longer silently consumed the file path as the network host value.
- `--allow-read=` and `--allow-write=` combining multiple empty flags in one command (`--allow-net= --allow-read= --allow-write=`) no longer errors with "a value is required".
- `process.argv` no longer duplicated script args when `eval_file_with_args` was called (the raw `std::env::args()` snapshot included the `--` args, causing double-appending).
- `fs.statSync().isFile()` and `fs.statSync().isDirectory()` returned the method function body instead of a boolean (the boolean values from JSON were overwritten before being captured in the closure). Fixed by saving raw booleans before creating method functions.

### Added (previous session)

- `3va dev` — full development server with HMR, SPA fallback, static serving.
- `3va audit --secrets` — Phase 3 audit for hardcoded secrets in dependencies.
- `3va audit --json` — machine-readable JSON output.
- `3va sandbox` — interactive REPL with multi-line support, session commands, TTY detection.
- `3va test --watch` / `--coverage` / `--update-snapshots`.
- `3va bundle --split` / `--minify` / `--source-map`.
- Full ESM support with `EsmResolver` and `EsmLoader`.
- `3va update` with per-package registry tracking.

- `3va dev` — full development server:
  - Flags `--port <N>` (default 3000), `--host <H>` (default 127.0.0.1), `--open`, `--public-dir <D>`.
  - HMR via Server-Sent Events at the `/__hmr` endpoint.
  - HMR client script injected automatically before `</body>` in all served HTML.
  - Static file serving with correct MIME types (15 supported types).
  - SPA fallback: unknown routes serve `public/index.html`.
  - Automatic rebuild with 300 ms debounce when detecting changes in `.js`, `.ts`, `.jsx`, `.tsx` files.
  - Built-in development page when `public/index.html` does not exist.
- `3va audit --secrets` — Phase 3 audit: detection of hardcoded secrets in the project's source files (AWS keys, GitHub tokens, PEM private keys, JWT tokens, Stripe keys and other common patterns) via `SecretsScanner`.
- `3va audit --json` — machine-readable output with `{ passed, phases: { malware, osv, secrets } }` structure; completely suppresses human-readable output.
- `audit_packages_silent()` in `vvva_pm` — audit variant without console output, used internally in `--json` mode.
- `3va sandbox` — full interactive REPL:
  - Multi-line support with balanced bracket detection (parentheses, brackets, braces).
  - Session commands: `.help`, `.clear`, `.allow-read=PATH`, `.allow-write=PATH`, `.allow-net=HOST`, `.allow-env`, `.permissions`; `exit`/`quit` to leave.
  - Node.js-style output formatting: objects as JSON, explicit `undefined` for statements.
  - TTY detection: in pipes and CI environments (stdin non-TTY), exits immediately without blocking.
- `3va test --watch` — automatically re-runs the suite when detecting file changes.
- `3va test --coverage` — statement/line coverage report upon test completion.
- `3va test --update-snapshots` / `-u` — overwrites existing snapshots with current values.
- `3va bundle --split` — code splitting; `--minify` — minification; `--source-map` — source map generation.
- Full ESM: `EsmResolver` and `EsmLoader` in `vvva_js::esm`; `import`/`export` support with relative paths, re-exports and TypeScript modules.
- Full async/await and Promise chain support via the `execute_pending_job` microtask loop.
- Bundler watch mode (`start_watch_mode`) with real `notify` watcher (previously was a stub without implementation).
- `describe` blocks and snapshot support (`toMatchSnapshot`) in the test runner.
- `list_granted()` in `PermissionState` — exposes the list of capabilities granted in the current session.
- `3va update` subcommand with per-package registry tracking.
- `registry` field in `3va-lock.json` (in `packages` and `dependencies`) to record the origin of each installed package.
- Registry preservation logic in the lockfile upon regeneration: registries of already installed packages are not lost when installing new packages.
- `--allow-net` validation in `3va update`: the CLI inspects the lockfile, groups packages by registry and displays the exact command to run if any authorized host is missing.
- Multi-registry support in the same project (e.g., `axios` from `registry.npmjs.org` and `@std/path` from `jsr.io`).
- Methods `registry_for()`, `registries_needed()` and `set_registry()` in `Lockfile` (`crates/pm/src/lockfile.rs`).
- 11 integration tests in `crates/test/tests/runner_integration.rs`.
- 12 unit tests in `crates/pm/src/auditor.rs`.
- 28 tests in `crates/js/tests/pipeline.rs` (ESM, async/await, TypeScript, permissions).
- Integration suite `scripts/integration_tests.sh`: 58 tests in 12 phases (100% passing).

### Fixed

- `is_esm_source()` stopped scanning upon finding the first line that was not an import; now scans the entire file with block comment tracking.
- Snapshot permission failed when the test file was in `/tmp/` (TempDir); now `FileRead`/`FileWrite` is granted to the test file's parent directory.
- `audit --json` emitted human-readable output before JSON because the malware scanner wrote directly to stdout; resolved via `audit_packages_silent()`.
- `run_audit_human` returned before reaching Phase 3 if Phase 1 (malware) or Phase 2 (OSV) produced an error; now all three phases are resilient to individual failures and always execute independently.

---

## [0.1.0-dev] - 2026-05-19

> Active development version. Not yet published as a stable release.

### Added

#### Package Manager (`crates/pm`)
- `3va install <package>[@<version>] --allow-net=<registry-host>` — secure installation from npm, Yarn or JSR.
- `3va reinstall <package> --allow-net=<registry-host>` — forced reinstallation.
- Automatic registry derivation from the host in `--allow-net` (no separate `--registry` flag needed).
- Support for three integrated registries: `registry.npmjs.org`, `registry.yarnpkg.com`, `jsr.io`.
- Support for scoped packages (`@scope/name`, mandatory in JSR).
- Package existence verification before installing.
- Version resolution: if not specified, uses `dist-tags.latest`; if the requested version does not exist, shows the 5 closest by semver distance.
- Version suggestions in `name@version` format.
- Security gate: any attempt to install without `--allow-net` shows an explanatory error and suggests the correct command — no silent network calls.
- Already installed package detection: prevents accidental reinstallation and suggests `reinstall`.
- `package.json` and `3va-lock.json` update after each successful installation.
- Signature verification via `SignatureVerifier` (SHA-256/SHA-512).
- JSR API: `/api/scopes/{scope}/packages/{name}/versions` endpoint.
- Semver distance algorithm: score = `major × 1_000_000 + minor × 1_000 + patch`.

#### CLI (`crates/cli`)
- Subcommands: `run`, `install`, `reinstall`, `update`, `dev`, `bundle`, `test`, `audit`, `doctor`, `sandbox`.
- Global `--accessible` flag for accessible mode (no colors or animations, EN 301 549 compliant).
- Granular permissions in `run`: `--allow-read`, `--allow-write`, `--allow-net`, `--allow-env`, `--allow-child-process`.
- Interactive permission prompt enabled by default in `run`.

#### JavaScript Engine (`crates/js`)
- QuickJS integration via `rquickjs`.
- Automatic TypeScript transpilation when executing `.ts`.
- CommonJS-compatible module system.
- Global APIs: `console`, `fetch`, `fs` (restricted by permissions), timers.

#### Bundler (`crates/bundler`)
- `3va bundle <input> --output <output>` — application bundling.
- TypeScript transpilation in the bundle process.

#### Test Runner (`crates/test`)
- `3va test [paths]` — test suite execution.
- Automatic discovery of `*.test.ts`, `*.test.js`, `*.spec.*` files.

#### Security
- `SignatureVerifier`: SHA-256 and SHA-512 hash calculation and verification of files.
- `MalwareScanner`: static analysis of dependencies.
- `AuditLogger`: sensitive operation logging.
- Interactive prompting for runtime permission requests.
- Post-install scripts disabled by default.

### Changed
- The `--registry` flag was removed from the design. The registry is determined exclusively by the authorized host in `--allow-net` — consistent with 3va's capability model.

### Architecture
- Cargo workspace with crates: `vvva_core`, `vvva_cli`, `vvva_permissions`, `vvva_js`, `vvva_pm`, `vvva_bundler`, `vvva_test`.
- Rust edition 2024.
- Async runtime: Tokio.

---

## Entry format

Each version follows the structure:

```
## [X.Y.Z] - YYYY-MM-DD

### Added        — new functionality
### Changed      — changes in existing functionality
### Deprecated   — functionality to be removed in future versions
### Removed      — removed functionality
### Fixed        — bug fixes
### Security     — vulnerability patches (reference CVE if applicable)
```

---

*Compliant with Keep a Changelog 1.0.0 and SemVer 2.0.0.*
