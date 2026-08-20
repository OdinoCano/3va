# 05 - POST-QUANTUM CRYPTOGRAPHY

## 5.1 Overview

The `vvva_crypto` crate provides 3va's post-quantum cryptography layer.  It
implements four quantum-resistant primitives: Lamport OTS, HKDF-SHA256,
ML-KEM-768, and ML-DSA-65.  Real in-handshake hybrid PQ-TLS (RFC 10024
X25519MLKEM768) is live in the JS networking layer as `tls.pqConnect()` — see
§5.3 and `docs/10-security/06-pq-tls-hybrid-design.md` for the full design.
Note this TLS path uses rustls' own aws-lc-rs ML-KEM implementation, not
`vvva_crypto`; `vvva_crypto`'s ML-KEM/ML-DSA are not currently exposed to JS
(§5.4).

## 5.2 Implemented Algorithms

### 5.2.1 Lamport One-Time Signatures (`LamportKeypair`)

Hash-based (SHA-256). Security reduces to SHA-256 preimage hardness — genuinely
post-quantum.

**Limitation:** Each key pair must sign at most one message. Signing a second
message with the same key reveals enough of the private key to forge signatures.

```rust
use vvva_crypto::lamport::LamportKeypair;

let keypair = LamportKeypair::generate();
let msg = b"hello";
let sig = keypair.sign(msg);
assert!(keypair.public_key().verify(msg, &sig));
```

### 5.2.2 HKDF Key Derivation (`hkdf_expand`)

HKDF-SHA256. Quantum-resistant key derivation from a shared secret.

```rust
use vvva_crypto::hkdf_expand;

let okm = hkdf_expand(b"input_key_material", b"context_info", 32);
```

### 5.2.3 ML-KEM-768 — Key Encapsulation (FIPS 203 / Kyber)

ML-KEM-768 is a lattice-based Key Encapsulation Mechanism standardized in NIST
FIPS 203.  It provides IND-CCA2 security under the Module Learning With Errors
(MLWE) assumption, which is believed to be hard for quantum computers.

**Key sizes (ML-KEM-768):**

| Artifact | Size |
|----------|------|
| Encapsulation key (public) | 1 184 bytes |
| Decapsulation key (seed) | 64 bytes |
| Ciphertext | 1 088 bytes |
| Shared secret | 32 bytes |

```rust
use vvva_crypto::{MlKemKeypair, MlKemCiphertext, encapsulate, decapsulate};

// Key generation
let kp = MlKemKeypair::generate();

// Sender encapsulates
let (ct, ss_send) = encapsulate(&kp.ek);

// Transport: send ct.to_hex() or ct raw bytes via MlKemCiphertext::from_bytes()

// Recipient decapsulates
let ss_recv = decapsulate(&kp.dk, &ct);
assert_eq!(ss_send.0, ss_recv.0);
```

**`MlKemCiphertext` constructors:**

| Method | Input | Use case |
|--------|-------|----------|
| `from_hex(s)` | Hex string | Stored/serialized ciphertexts |
| `from_bytes(b)` | Raw `&[u8]` | Wire-received bytes (no hex round-trip) |

Decapsulation with the wrong key does not fail explicitly — it returns a
different shared secret (implicit rejection per the FIPS 203 spec), preventing
oracle attacks.

### 5.2.4 ML-DSA-65 — Digital Signatures (FIPS 204 / Dilithium)

ML-DSA-65 is a lattice-based digital signature scheme standardized in NIST FIPS
204.  It is stateless and safe to use for multiple messages with the same key.

**Key sizes:**

| Object | Size |
|--------|------|
| Signing key (seed) | 32 bytes |
| Verifying key | 1 952 bytes |
| Signature | 3 309 bytes |

```rust
use vvva_crypto::{generate_signing_key, sign, verify};
use vvva_crypto::{signing_key_to_hex, signing_key_from_hex};
use vvva_crypto::{verifying_key_to_hex, verifying_key_from_hex};

let sk = generate_signing_key();
let vk_hex = verifying_key_to_hex(&sk);
let sig = sign(&sk, b"my message").unwrap();

let vk = verifying_key_from_hex(&vk_hex).unwrap();
verify(&vk, b"my message", &sig).unwrap();
```

## 5.3 Hybrid PQ-TLS (`tls.pqConnect`)

**Status: ✅ Implemented (client-only)** — `crates/js/src/builtins/tcp.rs`
+ `crates/js/src/builtins/modules.rs`. See
`docs/10-security/06-pq-tls-hybrid-design.md` for the full design rationale,
prior-art comparison, and interop test evidence.

Earlier versions of this doc described a *post-handshake* scheme: a normal
classical TLS handshake, followed by a hand-rolled ML-KEM exchange sent as
ordinary encrypted application data after the handshake completed. That
secret was never mixed into the TLS session keys and provided no real
security benefit — it has been replaced.

`tls.pqConnect()` now performs **real in-handshake hybrid key exchange**,
per [RFC 10024](https://www.rfc-editor.org/rfc/rfc10024.html) ("Post-Quantum
Traditional (PQ/T) Hybrid Key Agreement Mechanisms for TLS 1.3"), group
`X25519MLKEM768` (codepoint `0x11EC`). The ML-KEM-768 shared secret is
combined with the X25519 shared secret *inside* the TLS 1.3 key schedule
(RFC 8446 §7.1) as the connection's own (EC)DHE input — there is no separate
secret handed to JS, because the whole point of a hybrid group is that the
PQ contribution only ever exists inside the TLS session's own key material.

Implemented via [`rustls`](https://docs.rs/rustls) with the `aws_lc_rs` +
`prefer-post-quantum` features (native support since rustls 0.23.22), not
`vvva_crypto` — the TLS handshake needs a TLS stack with hybrid-group
support, which `vvva_crypto`'s standalone ML-KEM implementation doesn't
provide on its own.

**JS API:**

```js
const tls = require('tls');

const s = tls.pqConnect(443, 'example.com');
s.on('secureConnect', () => {
  console.log(s.negotiatedGroup); // "X25519MLKEM768" or "X25519" (classical fallback)
  console.log(s.pqNegotiated);    // true only if the hybrid PQ group was actually used
});

// Self-signed/private-CA endpoints (mirrors Node's tls `ca` option):
tls.pqConnect(port, host, { ca: caPemString });
```

A server that doesn't support `X25519MLKEM768` still negotiates successfully
(plain classical `X25519` — RFC 10024 hybrid groups always carry a classical
component), so this call never fails a connection that plain `tls.connect()`
would have succeeded at; check `pqNegotiated` to know which happened.

**Known limitations (stated, not hidden):**

- **Client-only.** 3va has no TLS server termination at all (`https.createServer`
  doesn't terminate TLS today) — a pre-existing gap, not introduced by this change.
- **Scoped to this one client path.** `tls.connect()` (classical), WebSocket
  `wss://`, and gRPC (`tonic`) TLS remain on `native-tls`/classical `rustls`
  respectively — migrating those wasn't needed to deliver real PQ-TLS and
  would have meant touching TLS paths that are already in production use for
  unrelated protocols.
- `X25519MLKEM768`/RFC 10024 is freshly standardized (August 2026) —
  treat it as new, not decades-battle-tested.

## 5.4 JS Crypto API — PQ Surface

**Status: not implemented.** There is no `require('crypto').pq` namespace in
the JS runtime today — `vvva_crypto`'s ML-KEM-768/ML-DSA-65 primitives (§5.2)
exist only on the Rust side. A prior version of this doc described a
`pq.kem`/`pq.dsa` JS API as if it existed; it does not. Wiring these up to JS
is tracked as future work, not part of the PQ-TLS change in §5.3.

## 5.5 Not-Yet-Available Algorithms

| Algorithm | Standard | Status |
|-----------|----------|--------|
| SLH-DSA-SHA2-128s | FIPS 205 (SPHINCS⁺) | Planned — `slh-dsa` crate not yet bundled |
| BIKE, HQC | Code-based KEM | Future |

## 5.6 Roadmap

| Version | Feature | Status |
|---------|---------|--------|
| v0.1.0 | Lamport OTS + HKDF | ✅ Done |
| v0.2.0 | ML-KEM-768, ML-DSA-65 | ✅ Done |
| v0.3.0 | Post-handshake PQ key exchange bolt-on (`__pqTlsConnect`, superseded) | ⚠️ Replaced |
| current | Real in-handshake hybrid PQ-TLS, RFC 10024 X25519MLKEM768 (`tls.pqConnect`, client-only) | ✅ Done |
| Future | `require('crypto').pq` JS bindings for `vvva_crypto` | 📋 Planned |
| Future | PQ-TLS server support | 📋 Planned |
| Future | SLH-DSA-SHA2-128s (SPHINCS⁺) | 📋 Planned |
| Future | BIKE, HQC (code-based KEM) | 📋 Future |

---

*Implemented in `crates/crypto/src/` (`kem.rs`, `dsa.rs`, `lamport.rs`, `hkdf.rs`)
and `crates/js/src/builtins/tcp.rs` + `modules.rs` (PQ-TLS binding). Design
rationale: `docs/10-security/06-pq-tls-hybrid-design.md`.*
