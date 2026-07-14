# 06 - Package Manager Feature Parity (pnpm / Yarn / npm)

Gap analysis against the standard pnpm/Yarn/npm feature matrix, and how each
missing feature gets added without reopening the one thing 3va refuses to
compromise on: **no implicit code execution at install time.**

Every item below is designed so it either (a) requires no code execution at
all, or (b) routes through the same deny-by-default permission sandbox that
`3va run` already uses — never a silent exec hook.

---

## 6.1 Gap summary

| Feature | Have it? | Plan |
|---|---|---|
| Overrides / resolutions | ✅ | Phase A — shipped |
| Listing licenses | ✅ | Phase A — shipped |
| SBOM generation | ✅ | Phase A — shipped |
| Autoinstalling peers | ✅ | Phase A — shipped |
| Hoisted `node_modules` | ✅ | Phase A — shipped |
| Config dependencies | ✅ | Phase A — shipped |
| Zero-Installs | ✅ | Phase B — shipped |
| Patching dependencies | ✅ | Phase B — shipped |
| Dynamic package execution (`dlx`) | ✅ | Phase B — shipped |
| JSR native registry support | partial (npm-compat shim) | kept as-is — see 6.5 |
| Auto-install before script run | ✅ | Phase B — shipped |
| Hooks | ❌ | Phase C |
| Managing versions of itself | ❌ | Phase C |
| Plug'n'Play | ❌ | Phase C |
| Managing runtimes | N/A | rejected — see 6.5 |
| Side-effects cache | N/A | rejected — see 6.5 |

---

## 6.2 Phase A — low risk, target 2.2.0

No new execution surface. Pure manifest/resolver/store additions.

| Item | Approach |
|---|---|
| **Overrides / resolutions** | Add `overrides` map to `PackageManifest` (`manifest.rs`); `Resolver` (`resolver.rs`) checks it before resolving any transitive version, same precedence rule as npm. |
| **`3va licenses`** | New CLI command reads `license`/`licenses` field already present in each package's `package.json` in the store (`store.rs` already caches full metadata). No new fetch. |
| **`3va sbom`** | New CLI command emits CycloneDX JSON from the resolved `Lockfile`. Pure data transform, zero network. |
| **Autoinstalling peers** | `resolver.rs` already parses `peerDependencies` (line ~218) but only records them. Extend `install_package`/`install_from_manifest` to resolve+install missing peers under the *same* `--allow-net` grant the user already gave — no new permission prompt, no silent widening of scope. |
| **Hoisted `node_modules`** | New `--node-linker=hoisted` flag on `install`/manifest `3va.nodeLinker` key. Alternate writer in `store.rs` that copies/hardlinks into a flat top-level `node_modules` instead of the symlink-per-package CAS layout. Isolated (current) stays the default. |
| **Config dependencies** | `configDependencies` manifest field: packages installed into a side store (`.3va/config-deps/`) for build tooling to read, never linked into `node_modules`, never executed. |

## 6.3 Phase B — medium effort, target 2.4.0

Still no implicit execution — `dlx` and hooks *look* like execution but are
explicit, user-invoked, and go through the existing permission prompt exactly
like `3va run` does for any other script.

| Item | Approach |
|---|---|
| **Zero-Installs** | Commit compressed store archives under `.3va/cache/` (like Yarn's `.yarn/cache`). `3va install` checks this cache before touching the network at all — this *strengthens* the security story, since a checked-in, hash-verified cache means zero supply-chain exposure on CI/fresh clones. |
| **Patching dependencies** | `3va patch <pkg>` opens an editable copy from the store; `3va patch-commit <pkg>` diffs it against the pristine version and stores the diff under `patches/`. Diffs are pure file diffs applied at install time — no script, no exec, reviewable in PRs like any other file. |
| **`3va dlx <pkg>`** | Fetches into the store (same signature verification as `install`) and runs it through the *existing* `3va run` sandbox — deny-by-default, prompts for any capability, same as running a local script. Not a special case. |
| **Auto-install before script run** | `3va run`/`3va test`/`3va dev` compare a hash of `package.json`'s dependency fields against the hash recorded by the last successful install; on mismatch, prompt once ("Dependencies changed — install now? [y/N]") before running — never implicit, never silent. |

## 6.4 Phase C — larger design, target 3.x

These need an explicit security model written down *before* implementation,
not just a flag.

| Item | Approach |
|---|---|
| **Hooks** | Lifecycle hooks (`postInstall`, `preInstall`, etc.) declared in `"3va": { "hooks": {...} }` in `package.json` — but a hook is just a named script that gets executed via `3va run --allow-*` with whatever capabilities the *project's own* manifest permissions grant (see [`docs/06-permissions/06-package-json-permissions.md`](../06-permissions/06-package-json-permissions.md)). No dependency's hook ever runs — only the top-level project's. This is the one non-negotiable rule; violating it reopens the exact supply-chain hole 3va exists to close. |
| **Managing versions of itself** | `"3va": "2.4.0"` pin field (corepack-style) + `3va self update`/`3va self use <version>`. Downloaded release binaries are verified against the project's `SignatureVerifier` (already used for packages) before replacing the running binary — never an unverified self-replace. |
| **Plug'n'Play** | Replace `node_modules` resolution with a generated resolution map (`.3va/pnp.json`, not executable JS like Yarn's `.pnp.cjs`, to avoid adding a new code-execution path). `esm.rs`/CommonJS `require()` consult this map instead of walking directories. The permission check that currently gates access per dependency must move into the resolution hook itself, so PnP doesn't accidentally bypass deny-by-default. |

## 6.5 Rejected — out of scope by design

| Item | Why not |
|---|---|
| **Managing runtimes** | 3va *is* the runtime (single static binary). There is no separate Node/Bun/Deno version to manage underneath it — the feature has no object to act on. |
| **Side-effects cache** | This cache exists in pnpm to memoize the filesystem side effects of install/build scripts. 3va disables those scripts unconditionally (see README § Package Manager), so there are no side effects to cache. Adding this would require first reintroducing script execution — a regression, not a feature. |
| **JSR native registry support** | Investigated for 2.4.0 and kept as the `npm.jsr.io` shim instead. JSR's native per-version metadata (`jsr.io/@scope/name/<version>_meta.json`) has no single-tarball `dist` field at all — a package is a `manifest` of individual source files, each with its own checksum, plus an `exports` map; there's no `package.json`. Every install-path primitive in `store.rs`/`lib.rs` (`store_tarball`, `extract_tarball`, `link_to_virtual_store`, dependency parsing) assumes one tarball → one `package.json`. Supporting JSR natively means a second package shape threaded through the whole pipeline and the ESM resolver, for a registry `npm.jsr.io` already serves faithfully — that's JSR's own officially-maintained npm-compatibility layer, not a workaround, and it already gets the same signature verification (`signature_verifier.rs`) as any other install. Revisit only if JSR's npm-compat layer is deprecated or a real correctness gap shows up in practice. |

---

## 6.6 Sequencing

```
2.2.0  Phase A — shipped (overrides, licenses, sbom, peer autoinstall, hoisted linker, config deps)
2.4.0  Phase B — shipped (zero-installs, patch, dlx, auto-install-before-run); JSR native kept as-is, see 6.5
3.x    Phase C (hooks, self-version-management, PnP)
```

Phase A items are additive and independently shippable — no ordering
dependency between them. Phase B's `dlx` should land after Phase A's
autoinstalling-peers work, since both reuse the same resolver path. Phase C
items each require a written security-model review (per `CODEOWNERS`, same
gate as any change to the permissions engine) before implementation starts,
not just at PR review time.
