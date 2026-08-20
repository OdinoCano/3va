# 06 - REAL IN-HANDSHAKE HYBRID PQ-TLS: DESIGN

## 6.1 Problem statement

Prior to this change, `__pqTlsConnect` (3va v0.3.0 onward) implemented what
`docs/10-security/05-post-quantum.md` called "Hybrid PQ-TLS." It was not
hybrid TLS in any standards sense:

1. A complete, ordinary **classical** `native-tls` handshake runs first
   (RSA/ECDHE, whatever the OS TLS library negotiates — no PQ involvement).
2. *After* the handshake completes, the client generates an ML-KEM-768
   keypair and writes the encapsulation key as plain **encrypted application
   data** over the now-established classical channel. The server responds
   with a ciphertext the same way.
3. Both sides derive a 32-byte ML-KEM shared secret and hand it back to the
   application (`pqSharedSecret`, a hex string returned to JS).

That secret was never mixed into the TLS session's own key schedule. It had
no cryptographic effect on the TLS connection itself: an adversary who
breaks the classical handshake (now, or by recording today's traffic and
breaking it once a quantum computer exists — "harvest now, decrypt later")
gets the entire plaintext regardless of whether the post-handshake ML-KEM
step happened. The scheme also had zero test coverage of any kind, and
`__pqTlsConnect` had zero callers anywhere in the JS layer — it was
orphaned, dead code, not a shipped feature despite the docs claiming
"✅ Done."

## 6.2 The real mechanism: RFC 10024

**[RFC 10024](https://www.rfc-editor.org/rfc/rfc10024.html), "Post-Quantum
Traditional (PQ/T) Hybrid Key Agreement Mechanisms for TLS 1.3"** (IETF TLS
Working Group, Proposed Standard, published August 2026) defines hybrid
key-exchange groups for TLS 1.3's `key_share` extension. This project uses
its `X25519MLKEM768` group (IANA `NamedGroup` codepoint `0x11EC`):

- Client's `key_exchange` value = ML-KEM-768 encapsulation key (1184 bytes)
  concatenated with the client's X25519 ephemeral share (32 bytes) — 1216
  bytes total.
- Shared secret = ML-KEM-768 shared secret (32 bytes) concatenated with the
  X25519 shared secret (32 bytes) — 64 bytes, fed directly into the existing
  TLS 1.3 key schedule (RFC 8446 §7.1) as the connection's own (EC)DHE
  input.

There is no side-channel exchange and nothing extra for the application to
do — the PQ contribution only ever exists *inside* the TLS session's key
material, which is why `tls.pqConnect()` no longer hands JS a separate
"shared secret" the way `__pqTlsConnect` did (see §6.5).

Because the group always carries a classical (X25519) component, a peer
that doesn't support the hybrid group simply doesn't get offered it and the
connection falls back to ordinary classical negotiation — there is no
separate fallback code path to write or fail.

## 6.3 Why rustls, not a native-tls patch

`native-tls` is a thin cross-platform wrapper over the OS's own TLS library
(Secure Transport on macOS, SChannel on Windows, OpenSSL — vendored, in this
project's case — elsewhere). It exposes no hook for selecting or extending
which key-exchange groups a handshake offers. "Patching" real hybrid PQ
support into it would mean forking three unrelated OS-native TLS stacks,
which is not realistic for one project to maintain.

[`rustls`](https://docs.rs/rustls) has shipped native `X25519MLKEM768`
support since 0.23.22, gated behind its `aws_lc_rs` + `prefer-post-quantum`
Cargo features (verified directly against the crate's source at the pinned
version — `rustls/src/crypto/aws_lc_rs/mod.rs`'s `ALL_KX_GROUPS`, which
includes `kx_group::X25519MLKEM768` unconditionally, and reorders it first
when `prefer-post-quantum` is enabled so the client actually offers a PQ key
share eagerly instead of only after a `HelloRetryRequest`). Both `rustls`
(0.23.42) and its `aws-lc-rs` crypto backend (1.17.1) were already present
in this workspace's `Cargo.lock` *transitively*, pulled in by `tonic`'s
`tls` feature and by `russh` respectively — this change promotes an
already-audited dependency family to direct use rather than introducing an
unvetted one.

## 6.4 Scope decision (blast radius)

`__pqTlsConnect` had no callers and 3va has no TLS server at all — this was
greenfield, not a migration of working functionality. Deliberately **not**
touched:

- `tls.connect()` / `__tcpConnectTls` — stays on `native-tls`.
- WebSocket `wss://` (`tungstenite`, `native-tls` feature) — unchanged.
- gRPC TLS (`tonic`) — unchanged (still on its own classical rustls setup).
- HTTPS server — doesn't exist (`https.createServer` never terminated TLS;
  pre-existing gap, unrelated to this change).

Rewiring every other TLS consumer in the runtime onto a hybrid-PQ-capable
stack would have meant touching TLS code paths that are in real production
use for unrelated protocols, for no benefit those protocols asked for. Scope
here is: replace the fake PQ path with a real one, additively, client-only,
via a new `tls.pqConnect()`.

## 6.5 What changed, concretely

- `crates/js/src/builtins/tcp.rs`: `TcpConn` gained a `PqTls` variant
  wrapping `rustls::StreamOwned<rustls::ClientConnection, TcpStream>`.
  `pq_tls_connect_blocking` now builds a `rustls::ClientConfig` (aws-lc-rs
  provider, native OS root-cert store by default, or a caller-supplied CA
  PEM — see below), runs one real TLS handshake via
  `ClientConnection::complete_io`, and reads back
  `negotiated_key_exchange_group()` to report which group actually won.
  The old post-handshake ML-KEM read/write loop is gone entirely — there is
  nothing left to do by hand.
- `crates/js/src/builtins/modules.rs`: new `tls.pqConnect(port, host, opts)`,
  mirroring the existing `tls.connect()` wrapper's conventions. Returns a
  `TLSSocket` with `.negotiatedGroup` (e.g. `"X25519MLKEM768"` or
  `"X25519"`) and `.pqNegotiated` (boolean) set once connected. The old raw
  `__pqTlsConnect` global is now backed by the real implementation and no
  longer returns a bare error string on failure (it used
  `js_code_err`/`Error` objects, consistent with every other TCP binding in
  the file — the old code's raw-string error return was the one
  inconsistency fixed while this code was already being touched).
- `ca` option: `tls.pqConnect(port, host, { ca: pemString })`, mirroring
  Node's `tls` `ca` option, for self-signed/private-CA endpoints. Without
  it, the OS-native root store is used (via `rustls-native-certs`), same
  trust boundary as `tls.connect()` today. This exists because the interop
  tests need to talk to a local, self-signed test server — real cert
  validation is not something to bypass for testability, so instead the
  client can be told which CA to trust, same as Node's own API.
- New dependencies (root `Cargo.toml`, direct): `rustls`
  (`aws_lc_rs`, `prefer-post-quantum` features), `rustls-native-certs`,
  `rustls-pki-types`, `rustls-pemfile`.

## 6.6 Interop verification (not just 3va talking to itself)

Per this task's rules, "production" claims here are backed by tests against
a **real, independent, third-party TLS implementation** — not 3va-to-3va.

**Automated, in `crates/js/tests/pq_tls.rs`** (runs under `cargo test -p
vvva_js --test pq_tls`, part of the normal `ci.yml` `test` job — skips
gracefully, not a hard CI failure, if the runner's `openssl` is older than
3.5, since PQ support in OpenSSL's own `s_server` is not something this repo
controls):

1. `pq_tls_negotiates_hybrid_group_against_real_openssl` — spawns a real
   local `openssl s_server -tls1_3 -groups X25519MLKEM768:X25519` (OpenSSL
   3.5+ has native RFC 10024 support, no OQS provider needed — confirmed via
   `openssl version` → `OpenSSL 3.5.5 27 Jan 2026` on the machine this was
   developed on) with a freshly-generated self-signed leaf cert, connects
   via `tls.pqConnect()`, and asserts the handshake succeeds **and** that
   `X25519MLKEM768` is the group actually negotiated on the wire.
2. `pq_tls_falls_back_to_classical_against_non_pq_server` — same setup but
   the server only offers `-groups X25519` (no PQ) — asserts the connection
   still succeeds, with plain `X25519` negotiated. This is the graceful
   degradation the hybrid design guarantees, verified against a real
   non-PQ-capable independent server, not simulated.

Both passed in local development:

```
running 3 tests
test openssl_probe_does_not_panic ... ok
test pq_tls_negotiates_hybrid_group_against_real_openssl ... ok
test pq_tls_falls_back_to_classical_against_non_pq_server ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Manual, one-time, against a real public production server** (not wired
into CI — an external network dependency in CI would be flaky and outside
this project's control; re-run manually with `cargo test -p vvva_js --test
pq_tls manual_real_world_check_google -- --ignored --nocapture`):

3va's own `tls.pqConnect()` against `www.google.com:443` (Google's edge is
widely known to default to `X25519MLKEM768`), last verified 2026-08-19:

```
GOOGLE REAL-WORLD RESULT: {"group":"X25519MLKEM768","pq":true}
```

Independently cross-checked the same target with the system's own OpenSSL
client (not 3va's code at all), same result:

```
$ openssl s_client -connect www.google.com:443 -groups X25519MLKEM768:X25519 -tls1_3 </dev/null
Negotiated TLS1.3 group: X25519MLKEM768
Protocol: TLSv1.3
```

## 6.7 Known limitations (stated, not hidden)

- **Client-only.** No PQ-TLS server — 3va doesn't terminate TLS on the
  server side at all today; this is a pre-existing gap, not something this
  change was scoped to fix.
- **One code path.** `tls.connect()`, WebSocket `wss://`, and gRPC TLS stay
  classical. Migrating those wasn't necessary to deliver real PQ-TLS and
  would have meant touching TLS code that's in production use for unrelated
  protocols.
- **`aws-lc-rs` vendors C/assembly** (the AWS-LC library). This project has
  existing known cross-compilation gaps for Android arm64 / Linux arm64
  cross builds (from the earlier rquickjs→V8 migration) — `aws-lc-rs` was
  not specifically verified to build cleanly on every platform this project
  ships prebuilt binaries for; if a future release surfaces a cross-compile
  failure tied to it, that's the first place to look.
- **RFC 10024 is freshly standardized** (August 2026, Proposed Standard
  status) — new, not decades-battle-tested, though `X25519MLKEM768` itself
  has been widely deployed pre-standardization by Chrome and Cloudflare
  since 2024.
- **No `require('crypto').pq` JS bindings.** `vvva_crypto`'s ML-KEM-768/
  ML-DSA-65 exist only on the Rust side; wiring them to JS is unrelated,
  future work, not touched by this change.

## 6.8 Non-goals

- No PQ-TLS server implementation.
- No migration of `tls.connect()`, `wss://`, or gRPC TLS off their existing
  classical stacks.
- No new `require('crypto').pq` JS API surface.
- No attempt to patch/fork `native-tls` or the underlying OS TLS libraries.
