# Security Audit — 3va 2.10.0 (2026-09-28)

**Audited commit:** `fc5e861` (v2.10.0) · **Scope:** runtime permission system, package manager, servers, CLI
**Method:** code review, SAST (`cargo-audit`, `semgrep`, `cargo-geiger`) and dynamic tests against the real
`3va` binary with local control servers. Every finding below was re-verified against the binary before
it was fixed, and each fix has a regression test that exercises the enforcement path actually used at
runtime (not a helper or dead code).

This document describes each issue and its fix. Step-by-step reproduction material is deliberately left
out until the fixes are released.

## Summary

The design intent is sound (deny-by-default, no `sh -c` in lifecycle scripts, tarball zip-slip and
symlink rejection, request-smuggling-resistant HTTP server). The problems were in enforcement:

- **Filesystem containment** could be escaped through `..` and symlinks.
- **Per-package permissions** were applied by a JavaScript wrapper that code could route around.
- **The package manager's chain of trust** had several independent fail-open points: unvalidated
  transitive names, missing or unknown integrity hashes, provenance keyed by the bundle itself, a
  repository `.npmrc` choosing registries, and a lockfile that did not pin bytes.
- **Tests and `doctor` checked dead code** (`VirtualFs`) or claimed protections that were never tested.

| Severity | Findings | Fixed | Partially fixed |
|---|---|---|---|---|
| Critical | 5 (VULN-01…05) | 5 | — |
| High | 10 (VULN-06…14, 22) | 10 | — |
| Medium | 8 (VULN-15…21, 23) | 8 | — |
| Informational | 6 (INFO-A…F) | 6 | — |

## Status of every finding

| ID | Sev. | Issue | Status | Regression test(s) |
|---|---|---|---|---|
| VULN-01 | Crit | Filesystem sandbox escape via `..` / symlinks | Fixed | `permissions::capability::tests::{dotdot_traversal_is_not_covered_by_grant, dotdot_after_symlink_resolves_like_the_kernel, dotdot_in_not_yet_existing_tail_fails_closed, symlinked_escape_outside_grant_is_rejected, write_target_that_does_not_exist_yet_is_checked_against_real_parent, grant_under_symlink_matches_real_location}` + `js::builtins::secure_fs::tests::{a_symlink_swapped_in_after_the_check_is_caught_at_open, writes_never_create_or_truncate_outside_the_grant, path_ops_run_in_the_checked_directory}` |
| VULN-02 | Crit | `pack`/`publish` included credentials | Fixed | `cli::tests::pack_excludes_credentials_even_when_files_field_lists_them` |
| VULN-03 | Crit | Per-package `deny-*` bypassable | Fixed | `js/tests/permission_enforcement.rs`: `scoped_deny_net_*`, `scoped_grant_*`, `scoped_deny_net_follows_bound_builtins_used_as_callbacks` (11 tests) |
| VULN-04 | Crit | Transitive package names used as paths | Fixed | `pm::vuln04_path_traversal_tests` (3) |
| VULN-05 | Crit | Install without integrity (fail-open) | Fixed | `pm::tests::{install_refuses_package_without_integrity_hash, install_refuses_unknown_integrity_algorithm, install_accepts_package_with_valid_integrity_hash}` |
| VULN-06 | High | `dns` had no permission check | Fixed | `dns_lookup_blocked_without_net_grant`, `dns_lookup_allowed_with_net_grant`, `scoped_deny_net_gates_dns_builtin` |
| VULN-07 | High | `allow-net` bypass through URL parser differential | Fixed | `js::builtins::fetch::tests::destination_matches_what_is_dialed` |
| VULN-08 | High | `doctor` claimed untested protections | Fixed | `cli/tests/doctor_checks.rs`, `cli::tests::doctor_self_checks_actually_verify` |
| VULN-09 | High | Provenance verified with the bundle's own key | Fixed | `pm::provenance::tests::{real_npm_provenance_fixture_verifies, self_signed_certificate_is_rejected_by_chain, fixture_leaf_chains_to_pinned_fulcio_root, chain_rejects_cert_not_valid_at_tlog_time, fixture_rekor_entry_verifies, tampered_rekor_set_is_rejected, foreign_rekor_log_is_rejected, rekor_entry_must_record_this_signature_certificate_and_payload, a_leaf_cannot_act_as_a_ca, signer_pins_are_trust_on_first_use, repository_references_normalize_alike}` |
| VULN-10 | High | Dependency granted its own lifecycle permissions | Fixed | `pm::trust::tests::lifecycle_permissions_come_from_the_project_not_the_dependency` |
| VULN-11 | High | Repository `.npmrc` redirected installs, plaintext accepted | Fixed | `pm::tarball_url_tests::*`, `pm::tests::scoped_pin_resolves_only_against_private_registry` |
| VULN-12 | High | `3va ci` did not install from the lockfile | Fixed | `pm::lock_binding_tests` (2) |
| VULN-13 | High | Any network grant allowed listening on all interfaces | Fixed | `permissions::capability::tests::wildcard_bind_needs_an_explicit_grant_or_stays_on_loopback` |
| VULN-14 | High | `store verify` verified nothing; hard-link store poisoning | Fixed | `pm::store::tests::{verify_detects_files_changed_after_storing, entries_without_a_digest_are_unverifiable_not_intact, break_hardlinks_gives_a_private_copy}` |
| VULN-15 | Med | Malware scan bypasses | Fixed | `pm::malware_scanner::tests::non_utf8_and_hidden_or_bundled_code_is_still_scanned` |
| VULN-16 | Med | `bin` path traversal and command shadowing | Fixed | `pm::bins::tests::bins_cannot_escape_or_shadow_other_packages` |
| VULN-17 | Med | Socket builtins ignored port-scoped grants; port truncation | Fixed | `port_scoped_grant_limits_tcp_connect`, `js::builtins::port_tests` |
| VULN-18 | Med | Check by name, connect by (re-resolved) IP | Fixed | `permissions::capability::tests::resolved_addresses_are_checked_against_ip_denies` |
| VULN-19 | Med | `dgram` listened without permission | Fixed | covered by `bind_host` tests; verified end to end |
| VULN-20 | Med | `audit` passed on incomplete scans | Fixed | verified end to end |
| VULN-21 | Med | Store/cache/`~/.npmrc` permissions followed the umask | Fixed | `pm::store::tests::store_root_and_files_are_not_writable_by_others` |
| VULN-22 | High | `node:sqlite` opened database files by path without any permission check | Fixed | `sqlite_open_blocked_without_fs_grant`, `sqlite_open_allowed_with_fs_grant`, `sqlite_sql_cannot_open_files_outside_the_grant` |
| VULN-23 | Med | FFI calls returning small integers or `void` overwrote the stack | Fixed | `ffi_module::{ffi_call_abs_i32, ffi_call_void_return}` (crashed with SIGSEGV before the fix) |
| INFO-A | Info | CI showed "0 vulnerabilities" with 4 suppressed advisories | Fixed | CI prints the suppressed list |
| INFO-B | Info | Invalid `.semgrep/semgrep.yml` | Fixed (removed; CI uses `.semgrep/rules/`) | — |
| INFO-C | Info | `permissions learn/suggest` pointed at a config file that is never read | Fixed | verified end to end |
| INFO-D | Info | Migration docs described a codemod that does not exist | Fixed | docs + backup warning |
| INFO-E | Info | Project's own `postinstall` dropped silently | Fixed | warning on install and in `doctor --compat` |
| INFO-F | Info | `http.Server` timeout properties had no effect | Fixed | properties report the real value and warn on set |

## Critical

### VULN-01 — Filesystem sandbox escape

`path_covered_by` compared paths with `starts_with` without resolving `..`, so a path under a granted
directory could name a file outside it. The first fix collapsed `..` lexically *before* resolving
symlinks. The kernel does the opposite (`link/..` is the parent of the link's target), so a script with
read/write access to a directory could create a symlink there and still escape.

**Fix.** `resolve_physically` canonicalizes the longest existing prefix of the raw path (the kernel's own
resolution of symlinks and `..`) and appends only the not-yet-existing tail. If that tail contains `..`,
the check fails closed, because after `mkdir -p` the kernel would resolve it through whatever exists by
then. Both the target and the grant go through the same function.

**Residual (closed on Linux).** Check-then-open was still a TOCTOU window (for example, a worker thread
swapping a symlink between the check and the open). The new `secure_fs` module closes it on Linux: every
operation acts on an object already pinned by a file descriptor whose real location
(`/proc/self/fd/N`) is what gets checked — reads/writes open inside a checked parent with `O_NOFOLLOW`,
and path operations (unlink, rename, chmod, chown, utimes, …) run on the pinned descriptor. All fs
builtins route through it, including `watchPollStat`, `readlink`, `chown`, `lchown`, `utimes` and
`lutimes`. `existsSync` and `accessSync` no longer answer for paths outside the grant. Other platforms
keep check-then-act (documented in the module).

### VULN-02 — `pack` / `publish` included credentials

`.env`, `.npmrc`, private keys and similar files were packed and published.

**Fix.** `collect_pack_files` always excludes credential files (`.env*`, `.npmrc`, `.netrc`, `.pypirc`,
`*.pem|key|p12|pfx|jks|ppk`, `id_*` SSH keys, `.aws/`, `.ssh/`, `.3va/`), even when `files` lists them,
and warns for each one. `publish` uploads the output of `pack`.

### VULN-03 — Per-package `deny-*` rules were bypassable

Package scoping was applied only when a package called a function from a fixed list of modules, obtained
through the `require()` wrapper. Global APIs (`fetch`), constructors (`new net.Socket()`), the raw native
bindings, changing the wrapper's globals, or handing a builtin to a promise or timer all ran in the root
scope. The same mechanism also let a package borrow another package's **grants** by setting the scope
name itself.

**Fix.** The runtime now reads the V8 stack when a native binding checks a permission, so JavaScript
cannot forge or hide the package that is calling:

- Any `node_modules/<pkg>` frame on the stack applies that package's `deny-*` rules.
- A package-scoped **grant** applies only if every package on the stack holds it (stack inspection).
- Scopes carry across deferred work: timers record the scopes active when they are scheduled, and a V8
  promise hook tags each promise with a `v8::Private`, which JavaScript cannot read or clear.
- The entry script's own package is recorded in Rust, not in a writable JS global.
- `fetch` kept a process-wide `PermissionState` (the first engine's); it is now per thread like the rest.

Stack walks and promise tagging only run once a project declares package-scoped rules.

**Residual (closed).** Callbacks that native code triggers from event sources a package did not create
ran without that package's scopes. A pre-bound builtin (`fetch.bind(null, url)`) handed to someone else
as a callback re-applies the binder's deny scopes on every call: `Function.prototype.bind` is guarded at
engine init so a bound function carries the caller's deny rules with it, no matter who runs it later.

### VULN-04 — Transitive package names used as filesystem paths

Names from registry metadata were used in `node_modules.join(name)` and in `remove_dir_all` without
validation.

**Fix.** Transitive names are validated with `is_valid_package_name`, and `safe_node_modules_path`
guards every place that creates or removes a directory under `node_modules`.

### VULN-05 — Integrity missing = install unverified

**Fix.** Only a verified digest lets a package install, on both install paths (`install` and `update`).
A missing hash, missing metadata or an algorithm the verifier can't check (for example `sha1-`) aborts
the install.

## High

### VULN-06 — `dns` without permission checks

**Fix.** `__dnsLookup`/`__dnsQuery` check `Capability::Network(hostname)`, and the deferred lookups run
under the caller's scope.

### VULN-07 — `allow-net` bypass through a URL parser differential

The permission check split URLs by hand, while the HTTP client uses the WHATWG parser, so the two could
see different hosts.

**Fix.** `destination_from_url` uses `url::Url`. `fetch`, `EventSource` and `WebSocket` canonicalize the
URL first and then check and connect with the same string. This also closes a plaintext-gate bypass
through URLs written without `//`.

### VULN-08 — `doctor` claimed untested protections

**Fix.** `doctor` reports only checks it actually runs against the real `PermissionState` and V8:
deny-by-default, path containment (including the `..`-plus-symlink case), deny-over-grant, and code
evaluation. The summary fails if any of them fails.

### VULN-09 — Provenance verified with the bundle's own key

The DSSE signature is checked against a key taken from the certificate in the bundle itself, and the
certificate is not chained to the Fulcio root. Whoever serves the bundle chooses the key.

**Fixed.** A passing check now means the signer is verified, not just that the bundle is internally
consistent:

- The attestation's `subject.digest.sha512` must match the downloaded tarball.
- The leaf certificate must chain to a pinned Fulcio root (through the pinned `sigstore-intermediate`),
  and must have been valid at the moment Rekor logged the entry (`integratedTime`). A self-signed or
  otherwise un-issued certificate is rejected outright.
- The Rekor transparency-log entry must be from the pinned public log (`logID`), its Signed Entry
  Timestamp must verify against the pinned Rekor key, and — when the bundle carries an inclusion proof —
  the checkpoint signature and the Merkle inclusion proof must bind the entry to the log's signed tree
  head.
- The Rekor entry must record *this* attestation: its body (`intoto` 0.0.2 or `dsse` 0.0.1) must hold
  the payload's SHA-256, this DSSE signature and this leaf certificate. Otherwise a valid entry copied
  from another bundle would "log" a forged one, and its `integratedTime` is what the certificate's
  validity is checked against.
- Only CA certificates may issue: a Fulcio leaf (not a CA, code-signing EKU) can't mint further
  certificates. The retired V0 root only counts for entries logged up to the end of 2022.
- **Who signed.** Chaining to Fulcio only proves *someone* authenticated to Sigstore; anyone can get a
  certificate for their own GitHub workflow. So each signer's repository (the Fulcio source-repository
  extension, or the workflow SAN) must match the `repository` the tarball's `package.json` declares —
  the check npm makes at publish time — and the signer identity is pinned on first install in
  `.3va/provenance-signers.json` (commit it). A later version signed by a different identity, or
  shipped without provenance, is refused.
- The trust anchors are pinned constants in `pm::provenance` (Fulcio root V0/V1, `sigstore-intermediate`,
  and the Rekor P-256 key); bundles from other Fulcio/Rekor instances are rejected.
- `--require-provenance` is satisfied by a verified signer whose repository matches the package's.

**Limit.** On the very first install of a package, a registry that serves both the tarball and the
attestation can still sign with its own identity if it also rewrites `repository`; the pin catches it
from the next install on.

### VULN-10 — A dependency granted its own lifecycle permissions

`sandbox_argv` built lifecycle script flags from the dependency's own `package.json`, including
unscoped `--allow-write` / `--allow-net`.

**Fix.** Lifecycle permissions come from the project's `"3va".permissions.<pkg>`, which uses the same
keys as the `3va run` flags. Only lists are accepted, so a bare `true` can't become an unscoped grant.

### VULN-11 — Repository `.npmrc` chose the registry; plaintext accepted

**Fix.**
- A scope pinned by `.npmrc` must use a registry host covered by `--allow-net`.
- Tarball hosts must be covered by `--allow-net` too.
- Registries and tarballs must use HTTPS, except on loopback.

### VULN-12 — `3va ci` did not install from the lockfile

**Fix.**
- Every install checks the registry's integrity hash against the one recorded in the lockfile for the
  same version, so a republished version with different bytes is refused.
- `3va ci` is strict: it installs the lockfile's versions for transitive packages as well, and requires
  every package to be recorded with a hash.
- Hashes in different algorithms (an old `sha1-` lock) are not compared as strings; `ci` asks for a
  re-lock instead.

### VULN-13 — Any network grant allowed listening on all interfaces

**Fix.** `PermissionState::bind_host` decides where a server may listen:
- Loopback needs any network grant.
- All interfaces (`0.0.0.0`, `::`, which is what `listen(port)` requests by default) need a grant that
  names them (`0.0.0.0`, `::` or `*`).
- With only other grants, the server is kept on loopback, and a note says so.

`server.address()` reports the address actually bound. This applies to HTTP, HTTP/2, TCP and UDP.

### VULN-14 — `store verify` verified nothing; hard-link poisoning

**Fix.**
- Storing a package records a SHA-256 tree digest next to it.
- `store verify` recomputes it: added, removed or modified files mark the entry as corrupt. Entries
  stored before digests existed are reported as unverifiable, not as intact.
- Before an allowlisted lifecycle script runs, the package's hard links to the global store are
  replaced with private copies, so the script can't modify files shared with other projects.

**Residual:** on Windows the hard links are not broken yet.

**Update:** Windows now copies every package file out of the global store before an allowlisted
lifecycle script runs, so the files the script can touch are no longer hard-linked to the store. The
residual is closed.

### VULN-22 — `node:sqlite` opened database files by path without any permission check

`__sqliteOpen` called `rusqlite::Connection::open(path)` directly: no `FileRead`/`FileWrite` check, so a
script could read and create SQLite databases anywhere on the host, sandbox or not.

**Fix.** The sqlite builtin now checks `Capability::FileRead` and `Capability::FileWrite` on the
database path before opening, throwing `EACCES` like the rest of the engine. `inject_sqlite` receives
the thread's `PermissionState` like the other network/fs builtins.

SQL can name files too, so the connection is also opened without `SQLITE_OPEN_URI` (a `file:…?…`
string is then just the file name that was checked) and with no attach slots, which makes
`ATTACH DATABASE` and `VACUUM INTO` fail. `:memory:` databases need no fs grant.

## Medium

- **VULN-15 — Malware scan bypasses.**
  - A `"trusted"` entry without a version no longer overrides a failed scan; pin `name@version`.
  - Files with invalid UTF-8 are scanned (decoded lossily) instead of passing unread.
  - Hidden directories and bundled `node_modules` are scanned; symlinks are not followed.
  - `--no-scan` remains available as an explicit opt-out by the user.
- **VULN-16 — `bin` entries.**
  - Bin names must be plain file names.
  - Targets must resolve inside the package, because the shim is made executable and `chmod` follows
    it.
  - When two packages claim the same command, the package it is named after wins, and every collision
    is reported.
- **VULN-17 — Port scoping.** Every socket builtin checks `host:port`, so port-scoped grants apply
  everywhere. Out-of-range port numbers map to 0 instead of wrapping around (70000 used to become 4464).
- **VULN-18 — Check by name, connect by IP.** The name is resolved once, addresses denied by IP (or
  IP:port) rules are dropped, and the connection goes to exactly the checked addresses. This covers TCP,
  TLS, WebSocket, `fetch`, `EventSource`, IRC, FTP, POP3, IMAP and MQTT. `EventSource` no longer follows
  redirects.
- **VULN-19 — `dgram`.** Creating a UDP socket needs a network grant, and `bind()` uses the same
  `bind_host` rule as TCP servers.
- **VULN-20 — `audit` failing open.** Failed lookups are reported as an incomplete audit, not as "no
  vulnerabilities". With `--deny`, incomplete or failed scans fail.
- **VULN-21 — umask-dependent permissions.**
  - The global store root and the `.3va-cache` directory are created with mode `0700`.
  - Stored files have their group and other write bits cleared.
  - `~/.npmrc` is written with mode `0600`.
- **VULN-23 — FFI return buffer overrun.** Found while fixing CI, not in the original audit. For
  integer returns narrower than a register (`i8`…`i32`, `u8`…`u32`) and for `void`, libffi writes a
  full `ffi_arg` (8 bytes), but `call_native` passed a buffer sized for the declared type, so each call
  wrote past it on the stack. Needs `--allow-ffi`, which already grants native code execution, but a
  plain `abs()` call could crash the process. Returns are now read into a 64-bit buffer and narrowed.

## Informational

- **INFO-A.** CI prints the advisories that `.cargo/audit.toml` suppresses. All four suppressions were
  reviewed and are justified; `rsa` is used for signing only, never for decryption.
- **INFO-B.** Removed the invalid `.semgrep/semgrep.yml`. CI loads `.semgrep/rules/`.
- **INFO-C.** `permissions learn` and `permissions suggest` print the `package.json` block that
  `3va run` actually reads.
- **INFO-D.** The migration guide now describes the codemod as it works (text rewriting, `.bak`
  backups), and the codemod warns that git does not ignore its backups.
- **INFO-E.** `install` and `doctor --compat` warn when the project's own `preinstall`, `install`,
  `postinstall` or `prepare` script is not run.
- **INFO-F.** `http.Server` `requestTimeout`, `headersTimeout`, `maxHeadersCount` and `maxConnections`
  report the limits actually in effect (from the firewall config) and warn when set.

## Discarded claims

These were reported during the audit and did not hold up on verification:
- Scope escalation through `__setCallerScope`: the wrapper re-applied the scope on each call. This
  mechanism was replaced by stack inspection anyway.
- CRLF injection through `statusMessage`.
- Request smuggling through duplicate `Content-Length`, or `Content-Length` combined with
  `Transfer-Encoding`.
- Wildcard host-matching bypass.
- Non-canonical loopback forms.
- Lamport key reuse.
- Tarball zip-slip or symlink extraction.

## Behavior changes

These fixes change behavior in ways users can notice:

1. `listen(port)` without a host binds `127.0.0.1` unless `--allow-net` includes `0.0.0.0`, `::` or `*`.
2. Package-scoped grants apply only when every package on the call stack holds them.
3. Lifecycle script permissions come from the project's `"3va".permissions.<pkg>`, never from the
   dependency.
4. An `.npmrc`-pinned registry host must be in `--allow-net`. Plaintext registries work only on
   loopback.
5. A lockfile integrity mismatch aborts the install. `3va ci` requires a complete lockfile with hashes.
6. `--require-provenance` refuses packages whose provenance can't be fully verified: no attestation,
   an unverifiable bundle format, or a bundle whose signer doesn't chain to the pinned Fulcio root /
   isn't bound to the pinned Rekor log.
7. A `"trusted"` entry without a version no longer overrides a failed malware scan.
8. `3va audit --deny` fails when the scan is incomplete.
9. `EventSource` does not follow redirects.
10. `node:sqlite` requires `FileRead` and `FileWrite` on the database path, like `fs`; `ATTACH`,
    `VACUUM INTO` and plain `VACUUM` fail. `existsSync` and `accessSync` report nothing for paths
    outside the grant instead of leaking their existence.
11. Verified provenance must come from the repository the package declares, and from the same signer
    as on first install (`.3va/provenance-signers.json`); a package that later drops provenance is
    refused.
