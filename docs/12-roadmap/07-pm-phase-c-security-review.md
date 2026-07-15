# 07 - PM Feature Parity, Phase C — Security Model Review

Per [`06-pm-feature-parity.md`](06-pm-feature-parity.md) § 6.4, each Phase C
item (hooks, self-version-management, Plug'n'Play) touches the permission
model or the trust boundary around code execution, so each requires a
written review — the same gate CODEOWNERS already puts on
`/crates/permissions/` — before implementation starts. This document is that
review: it is a request for maintainer sign-off, not an implementation plan.
Nothing in Phase C should be coded until the open questions below are
answered and this doc (or a follow-up) records the decision.

Status: **unreviewed — do not implement against this doc yet.**

---

## 7.1 Hooks

### What's being proposed
Lifecycle hooks (`postInstall`, `preInstall`, ...) declared in the
**top-level project's own** `package.json["3va"].hooks`, executed via the
same `3va run --allow-*` sandbox path `dlx` already reuses
(`build_permissions` in `crates/cli/src/main.rs`).

### The invariant that must hold
> No dependency's hook ever runs — only the top-level project's.

This is the one non-negotiable rule stated in the roadmap doc. Violating it
reopens the exact supply-chain hole (arbitrary code execution from a
transitive dependency at install time) that 3va exists to close.

### Where this can go wrong
- **Hook lookup must never read a nested manifest.** The install BFS in
  `install_with_transitive` (`crates/pm/src/lib.rs`) walks every
  `node_modules/<pkg>/package.json` it fetches to propagate transitive
  `dependencies`/`peerDependencies` (`collect_dep_specs`). If hook lookup is
  implemented as "scan every package.json under node_modules for a `hooks`
  key," it **will** find and execute a dependency's hook. Hook lookup must
  be hardcoded to read exactly one file:
  `project_root.join("package.json")` — never anything under `node_modules`.
- **Scope during hook execution.** The per-package permission scope
  (`crates/permissions/src/scope.rs`, `ScopeGuard`/`current_scope()`) exists
  so a grant declared for one dependency doesn't leak to another. A hook
  runs as the *root* project, so it must execute at `ROOT_SCOPE` — but if a
  hook's own script code calls `require()` on a dependency, that require
  call must correctly push a `ScopeGuard` for that dependency the same way
  normal script execution does. This needs a concrete test: a hook that
  requires a dependency with a narrower grant than the hook's own must not
  inherit the hook's broader grant.
- **Hook name collision.** If the manifest parser that reads
  `"3va": {"hooks": {...}}` shares any code path with the parser that reads
  a dependency's `scripts`/`bin` (e.g. a generic "find any `hooks` key in
  any package.json in the tree" helper reused from elsewhere), a key
  collision could cause a nested manifest's `hooks` to be misread as the
  root's. The implementation must use a dedicated, narrowly-scoped read —
  not a shared "search all package.json files" utility.
- **When do hooks fire relative to `install_with_transitive`'s BFS?** If
  `postInstall` fires per-package as each dependency finishes linking
  (mirroring npm's model), that reopens the same hole via a different
  door — a hook keyed to a *dependency's* completion event, running with
  root scope, is functionally identical to running that dependency's own
  hook. Hooks must fire exactly once, after the whole install finishes, tied
  to the root project only.

### Open questions for the reviewer
1. Should hook execution be opt-in per-project (a flag in `package.json`,
   separate from merely declaring `hooks`), so a project can't be
   surprised by a hook firing just because someone added a `hooks` key?
2. What's the audit story? `crates/permissions` already has
   `AuditLog`/`--audit-log` for `3va run`. Hooks should probably force
   `--audit-log` on by default, or at minimum log to the same audit trail,
   since a hook running unattended (no interactive prompt) at the end of
   `3va install` is exactly the scenario audit logging exists for.
3. Should a hook be able to request `SpawnProcess`/`FFI` at all, or should
   hooks be capped to a narrower capability subset than `3va run` allows?

---

## 7.2 Managing versions of itself

### What's being proposed
A `"3va": "2.4.0"` pin field (corepack-style) plus `3va self update` /
`3va self use <version>`, replacing the running binary after verifying a
downloaded release against `SignatureVerifier`
(`crates/pm/src/signature_verifier.rs`).

### The invariant that must hold
> The running binary is never replaced with bytes that haven't been
> verified against a trust root the user (not the downloaded release
> itself) established.

### Where this can go wrong
- **Circular trust.** `SignatureVerifier::verify_tarball`/`verify_from_registry`
  verify bytes against an *integrity hash*, but that hash has to come from
  somewhere the attacker doesn't control. For npm packages, the hash comes
  from registry metadata over HTTPS — an acceptable trust root for
  *dependencies* (matches the existing threat model: 3va already trusts the
  registry to say what a package's correct hash is). For **replacing the
  3va binary itself**, trusting "whatever hash the download server returns
  right now" is circular: a compromised distribution point could serve a
  malicious binary and a matching hash together. Self-update needs either
  (a) a hardcoded/pinned public key baked into the binary at compile time
  that verifies a real signature (not just a hash) on releases, or (b) hash
  pinning via a separate, out-of-band channel (e.g. the hash is fetched from
  a different host/mechanism than the binary itself, such as being
  committed to the `3va` GitHub releases page metadata via `gh` and checked
  against `.github/workflows/release.yml`'s already-published `.sha256`).
  This needs a concrete key-management decision before any code exists —
  it is explicitly flagged in `06-pm-feature-parity.md` as *not* this plan's
  call to make.
- **Replacing a running executable.** On Unix, overwriting the currently
  executing binary's inode is safe (the OS keeps the old inode alive for the
  running process); on Windows, the file is typically locked while running,
  so `3va self update` likely needs a rename-then-replace-on-next-launch
  dance. This is a correctness risk, not just a security one, but a failed
  self-replace has security implications too (a half-written binary is
  worse than no update).
- **Downgrade attacks.** `3va self use <older-version>` is a legitimate
  feature (pin to a known-good version) but is indistinguishable from an
  attacker forcing a downgrade to a version with a known vulnerability
  unless downgrades require an explicit, separate confirmation from
  `self update`.

### Open questions for the reviewer
1. Where does the release signing key live, and who holds the private key?
   (This determines whether option (a) above is even available.)
2. Should `3va self update` require the same kind of explicit
   `--allow-*`-style grant as any other capability, given it's arguably the
   most powerful possible capability (arbitrary code replacing the runtime
   itself)?
3. Does `3va self use <version>` need a confirmation prompt when the target
   version is older than the running one?

---

## 7.3 Plug'n'Play

### What's being proposed
Replace `node_modules` directory-walk resolution with a generated
`.3va/pnp.json` resolution map (deliberately data, not executable JS, unlike
Yarn's `.pnp.cjs`) that module resolution consults instead of walking
directories. Two call sites need this today, not one: `crates/js/src/esm.rs`'s
`find_in_node_modules`/`resolve_node_module_esm` (specifier resolution,
shared by every file regardless of `import`/`require` syntax — see 7.3's
scope-inference finding below for why source syntax doesn't create a second
runtime), and `crates/js/src/builtins/modules.rs`'s own `require()`
directory resolution, which is also where `__pkgScopeFor` reads the
resolved path back out for scope inference.

### The invariant that must hold
> Whatever currently gates a module's filesystem access per-dependency must
> keep gating it exactly the same way when resolution goes through
> `pnp.json` instead of directory walking.

### Where this can go wrong
- **This is the risk the roadmap doc calls out by name, and it's confirmed,
  not speculative.** `crates/js/src/builtins/modules.rs`'s `require_js`
  (~line 2695) defines `__pkgScopeFor(dir)`, which extracts the innermost
  `node_modules/<pkg>` segment from the *directory path* of the module doing
  the requiring via a regex (`/[\/\\]node_modules[\/\\](@[^\/\\]+[\/\\][^\/\\]+|[^\/\\]+)/g`),
  then calls `__setCallerScope(scopeName)` → `vvva_permissions::set_current_scope`
  right before handing a capability-gated builtin (`fs`, `net`, `tls`,
  `dgram`, `child_process` — see `__SCOPE_GATED_MODULES`) to that module.
  Scope inference is **entirely path-based, today**. If PnP resolution stops
  walking `node_modules` and instead maps a package name straight to an
  arbitrary location via `pnp.json`, the resolved `dir` no longer matches
  `__pkgScopeFor`'s regex, the function falls through to its `|| '.'`
  default, and every dependency silently collapses to `ROOT_SCOPE` — a
  scoped grant declared for one dependency in
  `package.json["3va"].permissions.<name>` would then apply to *every*
  dependency, and (worse) every dependency would receive whatever grants the
  app itself has, since `ROOT_SCOPE` is the unscoped, always-applies grant
  set. **This must be fixed as part of the same change that introduces PnP,
  not after**: `__pkgScopeFor` needs a PnP-aware equivalent that maps a
  resolved (non-`node_modules`) path back to the package name that owns it,
  using `pnp.json`'s own map in reverse (or by tagging scope at resolution
  time instead of re-deriving it from a path per capability check).
- **`pnp.json` as a new trust boundary.** The map itself must be generated
  only from packages 3va already verified via `SignatureVerifier` during
  install — it must not be hand-editable in a way that lets a project (or a
  compromised `postinstall`-equivalent, if hooks land first) redirect a
  package name to an arbitrary filesystem path outside the verified store,
  which would be a sandbox escape independent of the scoping issue above.
- **`esm.rs` is resolution-only, not a second execution path — verified.**
  There is no `v8::Module`/`ScriptCompiler::CompileModule`/module-resolve
  callback anywhere in `crates/js/src/`: every file, `import`-based or not,
  runs as a plain V8 `Script`. Static `import`/`export` is desugared to
  CommonJS *before* execution — at the entry point via
  `transpiler::transpile_to_cjs()` (`crates/js/src/lib.rs:302-326`), and for
  anything pulled in via `require()` (including transitively, from deep
  inside `node_modules`) via the same `is_esm` check inside the native
  `__readFile` callback (`crates/js/src/builtins/modules.rs:250-274`). So
  `import X from 'mod'` becomes `var X = require('mod')`
  (`transpiler::static_esm_to_cjs`, `transpiler.rs:558+`) and re-enters the
  *same* `__pkgScopeFor`/`__setCallerScope` machinery as any CJS
  dependency — confirmed for both "ESM import of a Node builtin" and "ESM
  import of a third-party dependency that itself uses fs/net", since a
  dependency's own file gets the identical `is_esm` transpile-to-CJS
  treatment before its internal `require()` calls run. **No PnP-specific
  fix is needed here** — the earlier draft of this review speculated ESM
  might be unscoped; it isn't, because there's no separate ESM runtime to
  have a gap in. (Two unrelated findings from confirming this, left for a
  separate fix, not blocking PnP: dynamic `import(...)` transpiles to
  `__importAsync(...)`, which is never defined anywhere — `transpiler.rs`
  lines 503/522 are the only references in the repo — so it fails closed
  with a `ReferenceError` rather than actually working; and there is no
  end-to-end test that drives the real JS engine through an ESM-imported
  `node_modules` dependency with a scoped grant, only specifier-resolution
  tests and direct `set_current_scope` unit tests — worth adding regardless
  of PnP, as a regression guard for the `__pkgScopeFor` path itself.)
- **CommonJS `require()`'s scope logic in `modules.rs`** (`inject_require`,
  the `require_js` JS shim, `__pkgScopeFor`) is therefore the *only* place
  PnP needs to add an equivalent — not two independent paths as an earlier
  draft of this review assumed.

### Open questions for the reviewer
1. `__pkgScopeFor` needs a PnP-aware replacement (see above) that maps a
   `pnp.json`-resolved path back to the owning package name — this is a
   single, well-located change now that it's confirmed to be the only
   scope-inference site, not an open-ended audit across two runtimes.
2. Should PnP mode require `--allow-read` grants to be expressed differently
   (since there's no `node_modules/<pkg>/` path to scope a `FileRead`
   capability to under PnP)?
3. Is PnP worth the resolver complexity given 3va already has a working
   isolated CAS layout ([A5](06-pm-feature-parity.md#62-phase-a--low-risk-target-220))
   and a hoisted fallback? What concrete problem does PnP solve for 3va
   users that hoisted/isolated don't already cover?

---

## 7.4 Sequencing note

None of 7.1–7.3 block each other technically. 7.1 (hooks) remains the
smallest surface and the most concretely scoped of the three, so it's the
reasonable first candidate once its open questions are answered. 7.3 (PnP)
initially looked like it had the largest unresolved surface — a suspected
second, unscoped ESM execution path — but that was investigated and ruled
out (§7.3: ESM desugars to CommonJS before execution, so there is only one
scope-inference site, `__pkgScopeFor` in `modules.rs`, not two). PnP's
remaining open question (§7.3, item 1) is now a single well-located
implementation task, not an open-ended audit, which meaningfully lowers its
risk relative to the original draft of this review. 7.2 (self-versioning)
still has the least-resolved open question of the three: the release
signing/key-management decision in §7.2 is explicitly out of scope for this
review to decide and blocks implementation regardless of sequencing.
