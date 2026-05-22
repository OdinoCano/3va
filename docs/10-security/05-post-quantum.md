# 05 - POST-QUANTUM CRYPTOGRAPHY

## 5.1 Overview

The `vvva_crypto` crate provides the cryptography layer for 3va. It implements two genuinely quantum-resistant primitives and exposes a placeholder interface for NIST PQC algorithms that are not yet bundled.

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

## 5.3 Not-Yet-Available NIST PQC Algorithms

The following algorithms are defined in `Algorithm` but return `CryptoError::NotAvailable` — the required RustCrypto crates are not yet bundled in this workspace.

| `Algorithm` variant | Standard | Required crate |
|---------------------|----------|----------------|
| `MlKem768` | ML-KEM / FIPS 203 (Kyber) | `ml-kem = "0.3"` |
| `MlDsa65` | ML-DSA / FIPS 204 (Dilithium) | `ml-dsa = "0.1"` |
| `SlhDsaSha2128s` | SLH-DSA / FIPS 205 (SPHINCS⁺) | `slh-dsa` |

Calling any of these returns:
```
CryptoError::NotAvailable("ML-KEM and ML-DSA require the `ml-kem`/`ml-dsa` RustCrypto crates...")
```

These are planned for v0.3.0 once the upstream crates stabilize.

## 5.4 Planned — NIST PQC (v0.3.0)

> **Status: PENDING** — the `Algorithm` enum variants exist and the interface is defined, but all calls return `CryptoError::NotAvailable` until the crates are added.

Planned usage once ML-KEM/ML-DSA crates are bundled:

```rust
// PLANNED — returns NotAvailable today
use vvva_crypto::Algorithm;

// Key encapsulation (ML-KEM-768 / Kyber)
let (ciphertext, shared_secret) = ml_kem_encapsulate(&public_key, Algorithm::MlKem768)?;
let decapsulated = ml_kem_decapsulate(&private_key, &ciphertext, Algorithm::MlKem768)?;

// Digital signature (ML-DSA-65 / Dilithium)
let sig = ml_dsa_sign(&private_key, message, Algorithm::MlDsa65)?;
ml_dsa_verify(&public_key, message, &sig, Algorithm::MlDsa65)?;

// Hash-based signature (SLH-DSA / SPHINCS+)
let sig = slh_dsa_sign(&private_key, message, Algorithm::SlhDsaSha2128s)?;
```

## 5.5 Planned — Hybrid TLS (Future)

> **Status: FUTURE** — depends on both ML-KEM and a custom TLS layer.

```
Classic ECDH  +  ML-KEM-768  →  combined shared secret  →  AES-256-GCM
```

Hybrid mode: if the peer doesn't support PQC, falls back to classic ECDH only.

## 5.6 Roadmap

| Version | Feature | Status |
|---------|---------|--------|
| v0.1.0 | Lamport OTS + HKDF | ✅ Implemented |
| v0.3.0 | ML-KEM-768, ML-DSA-65, SLH-DSA | 📋 Planned |
| Future | Post-quantum hybrid TLS | 📋 Planned |
| Future | BIKE, HQC (code-based KEM) | 📋 Future |

---

*Implemented in `crates/crypto/src/` (`lib.rs`, `lamport.rs`, `hkdf.rs`).*
