# 05 - POST-QUANTUM CRYPTOGRAPHY

## 5.1 Quantum-Resistant Cryptography

3va is designed to support algorithms resistant to quantum attacks.

## 5.2 Algorithms

### 5.2.1 Key Encapsulation

| Algorithm | Status | Description |
|-----------|--------|-------------|
| Kyber | Ready | NIST PQC standard |
| ML-KEM | Ready | ML-KEM (Kyber) |
| BIKE | Future | Code-based |
| HQC | Future | Hamming Quasi-Cyclic |

### 5.2.2 Digital Signatures

| Algorithm | Status | Description |
|-----------|--------|-------------|
| Dilithium | Ready | NIST PQC standard |
| ML-DSA | Ready | Digital signature |
| SPHINCS+ | Ready | Hash-based |
| Falcon | Future | Lattice-based |

## 5.3 Hybrid TLS

```javascript
// Hybrid TLS connection
const tls = require("tls");

// Post-quantum algorithms
const cipherSuites = [
  "kyber512+aes-256-gcm",
  "p256-kyber512+aes-256-gcm",
  "aes-256-gcm" // Classic fallback
];

const socket = tls.connect({
  ciphers: cipherSuites.join(":")
});
```

## 5.4 Signatures

```javascript
const { quantumSign, quantumVerify } = require("3va/crypto");

// Sign with post-quantum algorithm
const signature = await quantumSign(data, "dilithium2");

// Verify
const valid = await quantumVerify(signature, data, "dilithium2");
```

## 5.5 Timeline

| Phase | Algorithms | Status |
|-------|------------|--------|
| 2026 | Kyber + Dilithium | Ready |
| 2027 | ML-KEM + ML-DSA | Ready |
| 2028 | Falcon | Future |
| 2029 | Post-quantum TLS 1.4 | Planned |

## 5.6 Compatibility

```javascript
// Hybrid configuration
module.exports = {
  crypto: {
    hybridMode: true,           // Classic + PQ
    preferPostQuantum: false,    // Prefer PQ
    minSecurityLevel: "128"     // security bits
  }
};
```

---

*Post-quantum cryptography compliant with NIST PQC standards.*
