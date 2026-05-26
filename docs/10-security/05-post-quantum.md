# 05 - POST-QUANTUM CRYPTOGRAPHY

## 5.1 Overview

The `vvva_crypto` crate provides the cryptography layer for 3va. It implements four quantum-resistant primitives: Lamport OTS, HKDF-SHA256, ML-KEM-768, and ML-DSA-65.

## 5.2 Implemented Algorithms

### 5.2.1 Lamport One-Time Signatures (`LamportKeypair`)

Hash-based (SHA-256). Security reduces to SHA-256 preimage hardness — genuinely post-quantum.

**Limitation:** Each key pair must sign at most one message. Signing a second message with the same key reveals enough of the private key to forge signatures.

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

ML-KEM-768 is a lattice-based Key Encapsulation Mechanism standardized in NIST FIPS 203. It provides IND-CCA2 security under the Module Learning With Errors (MLWE) assumption, which is believed to be hard for quantum computers.

**Key sizes:**

| Object | Size |
|--------|------|
| Encapsulation key (public) | 1184 bytes |
| Decapsulation key (seed) | 64 bytes |
| Ciphertext | 1088 bytes |
| Shared secret | 32 bytes |

```rust
use vvva_crypto::{MlKemKeypair, encapsulate, decapsulate};
use vvva_crypto::{encapsulation_key_from_hex, decapsulation_key_from_hex};

// Key generation
let keypair = MlKemKeypair::generate();

// Sender encapsulates
let ek_hex = keypair.encapsulation_key_hex();
let ek = encapsulation_key_from_hex(&ek_hex).unwrap();
let result = encapsulate(&ek).unwrap();
// result.ciphertext_hex() — send to recipient
// result.shared_secret_hex() — sender's shared secret

// Recipient decapsulates
let dk = decapsulation_key_from_hex(&keypair.decapsulation_key_hex()).unwrap();
let ss = decapsulate(&dk, &result.ciphertext()).unwrap();
// ss.shared_secret_hex() == result.shared_secret_hex()
```

Decapsulation with the wrong key does not fail explicitly — it returns a different shared secret (implicit rejection per the FIPS 203 spec), preventing oracle attacks.

### 5.2.4 ML-DSA-65 — Digital Signatures (FIPS 204 / Dilithium)

ML-DSA-65 is a lattice-based digital signature scheme standardized in NIST FIPS 204. It is stateless and safe to use for multiple messages with the same key.

**Key sizes:**

| Object | Size |
|--------|------|
| Signing key (seed) | 32 bytes |
| Verifying key | 1952 bytes |
| Signature | 3309 bytes |

```rust
use vvva_crypto::{generate_signing_key, sign, verify};
use vvva_crypto::{signing_key_to_hex, signing_key_from_hex};
use vvva_crypto::{verifying_key_to_hex, verifying_key_from_hex};

// Key generation
let sk = generate_signing_key();

// Sign
let vk_hex = verifying_key_to_hex(&sk);
let sk_hex = signing_key_to_hex(&sk);
let sig = sign(&sk, b"my message").unwrap();

// Verify
let vk = verifying_key_from_hex(&vk_hex).unwrap();
verify(&vk, b"my message", &sig).unwrap(); // Ok(())
```

Unlike Lamport OTS, ML-DSA-65 keys can sign an unlimited number of messages.

## 5.3 Not-Yet-Available Algorithms

| Algorithm | Standard | Status |
|-----------|----------|--------|
| SLH-DSA-SHA2-128s | FIPS 205 (SPHINCS⁺) | Planned — `slh-dsa` crate not yet bundled |

## 5.4 Planned — Hybrid TLS (Future)

> **Status: FUTURE** — depends on both ML-KEM and a custom TLS layer.

```
Classic ECDH  +  ML-KEM-768  →  combined shared secret  →  AES-256-GCM
```

Hybrid mode: if the peer doesn't support PQC, falls back to classic ECDH only.

## 5.5 Roadmap

| Version | Feature | Status |
|---------|---------|--------|
| v0.1.0 | Lamport OTS + HKDF | ✅ Implemented |
| v0.2.0 | ML-KEM-768, ML-DSA-65 | ✅ Implemented |
| Future | SLH-DSA-SHA2-128s (SPHINCS⁺) | 📋 Planned |
| Future | Post-quantum hybrid TLS | 📋 Planned |
| Future | BIKE, HQC (code-based KEM) | 📋 Future |

---

*Implemented in `crates/crypto/src/` (`lib.rs`, `lamport.rs`, `hkdf.rs`, `kem.rs`, `dsa.rs`).*
