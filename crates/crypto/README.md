# vvva_crypto

Post-quantum cryptography primitives for 3va.

## Algorithms

| Algorithm | Type | Module |
|-----------|------|--------|
| ML-KEM-768 (CRYSTALS-Kyber) | KEM | `kem` |
| ML-DSA-65 (CRYSTALS-Dilithium) | Signature | `dsa` |
| Lamport one-time signatures | Signature | `lamport` |
| HKDF-SHA256 | KDF | `hkdf` |

## Key functions

```rust
// Key encapsulation
let kp = vvva_crypto::kem::MlKemKeypair::generate();
let (ct, ss) = vvva_crypto::encapsulate(&kp.encapsulation_key())?;
let ss2 = vvva_crypto::decapsulate(&kp.decapsulation_key(), &ct)?;

// Signatures
let sk = vvva_crypto::generate_signing_key();
let sig = vvva_crypto::sign(&sk, b"message")?;
vvva_crypto::verify(&sk.verifying_key(), b"message", &sig)?;
```

## Docs

`docs/10-security/`
