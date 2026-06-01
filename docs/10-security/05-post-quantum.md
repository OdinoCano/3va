# 05 - POST-QUANTUM CRYPTOGRAPHY

## 5.1 Overview

The `vvva_crypto` crate provides 3va's post-quantum cryptography layer.  It
implements four quantum-resistant primitives: Lamport OTS, HKDF-SHA256,
ML-KEM-768, and ML-DSA-65.  Hybrid PQ-TLS (classical TLS + ML-KEM-768 key
exchange) is live in the JS networking layer.

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

## 5.3 Hybrid PQ-TLS (`__pqTlsConnect`)

**Status: ✅ Implemented** — live in `crates/js/src/builtins/tcp.rs`

`__pqTlsConnect(host, port)` establishes a connection with both classical and
post-quantum forward secrecy:

```
1. Classical TLS handshake  →  authenticates server, encrypts channel
2. ML-KEM-768 key exchange  →  adds PQ forward secrecy on top
   client → server: [4-byte length][encapsulation key (1184 B)]
   client ← server: [4-byte length][ciphertext (1088 B)]
   both sides derive: 32-byte shared secret
```

The resulting `pqSharedSecret` can be combined with the TLS session key via
HKDF to produce a hybrid key that is secure against both classical and quantum
adversaries.

**JS API:**

```js
const tls = require('tls');

const { connId, pqSharedSecret } = await tls.pqConnect('example.com', 443);
// pqSharedSecret: 64-char hex string (32 bytes)

// Combine with HKDF for a full hybrid key:
// hybrid_key = HKDF(pqSharedSecret || tlsSessionKey, "hybrid-key", 32)
```

**Implementation notes:**

- All blocking I/O (TCP connect, TLS handshake, ML-KEM round-trip) runs in
  `tokio::task::spawn_blocking` so the JS event loop is never stalled.
- The ciphertext received from the server is decoded via `MlKemCiphertext::from_bytes`
  — no unnecessary hex round-trip.
- The function is registered as an `Async` binding, consistent with all other
  async networking primitives.

## 5.4 JS Crypto API — PQ Surface

The `require('crypto').pq` namespace exposes ML-KEM and ML-DSA to JS:

```js
const { pq } = require('crypto');

// ML-KEM-768
const kp = pq.kem.generateKeypair();
// { encapsulationKey: '<hex>', decapsulationKey: '<hex>' }

const { ciphertext, sharedSecret } = pq.kem.encapsulate(kp.encapsulationKey);
const ss = pq.kem.decapsulate(kp.decapsulationKey, ciphertext);
// ss === sharedSecret (32-byte hex)

// ML-DSA-65
const dsakp = pq.dsa.generateKeypair();
const sig = pq.dsa.sign(dsakp.signingKey, 'message');
const ok  = pq.dsa.verify(dsakp.verifyingKey, 'message', sig);
```

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
| v0.3.0 | Hybrid PQ-TLS (`__pqTlsConnect`) | ✅ Done |
| Future | SLH-DSA-SHA2-128s (SPHINCS⁺) | 📋 Planned |
| Future | BIKE, HQC (code-based KEM) | 📋 Future |

---

*Implemented in `crates/crypto/src/` (`kem.rs`, `dsa.rs`, `lamport.rs`, `hkdf.rs`)
and `crates/js/src/builtins/tcp.rs` (PQ-TLS binding).*
