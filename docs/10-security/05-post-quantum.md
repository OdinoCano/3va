# 05 - CRIPTOGRAFÍA POST-CUÁNTICA

## 5.1 Criptografía Resistente

3va está diseñado para soportar algoritmos resistentes a ataques cuánticos.

## 5.2 Algorithms

### 5.2.1 Key Encapsulation

| Algorithm | Status | Descripcion |
|-----------|--------|-------------|
| Kyber | Ready | NIST PQC standard |
| ML-KEM | Ready | ML-KEM (Kyber) |
| BIKE | Future | Code-based |
| HQC | Future | Hamming Quasi-Cyclic |

### 5.2.2 Firmas Digitales

| Algorithm | Status | Descripcion |
|-----------|--------|-------------|
| Dilithium | Ready | NIST PQC standard |
| ML-DSA | Ready | Digital signature |
| SPHINCS+ | Ready | Hash-based |
| Falcon | Future | Lattice-based |

## 5.3 TLS Híbrido

```javascript
// Conexión TLS híbrida
const tls = require("tls");

// Algorithms post-cuanticos
const cipherSuites = [
  "kyber512+aes-256-gcm",
  "p256-kyber512+aes-256-gcm",
  "aes-256-gcm" // Fallback clásico
];

const socket = tls.connect({
  ciphers: cipherSuites.join(":")
});
```

## 5.4 Firmas

```javascript
const { quantumSign, quantumVerify } = require("3va/crypto");

// Firmar con algoritmo post-cuántico
const signature = await quantumSign(data, "dilithium2");

// Verificar
const valid = await quantumVerify(signature, data, "dilithium2");
```

## 5.5 Timeline

| Phase | Algorithms | Status |
|-------|------------|--------|
| 2026 | Kyber + Dilithium | Ready |
| 2027 | ML-KEM + ML-DSA | Ready |
| 2028 | Falcon | Future |
| 2029 | Post-quantum TLS 1.4 | Planned |

## 5.6 Compatibilidad

```javascript
// Configuración de híbridos
module.exports = {
  crypto: {
    hybridMode: true,           // Clásico + PQ
    preferPostQuantum: false,    // Preferir PQ
    minSecurityLevel: "128"     // bits de seguridad
  }
};
```

---

*Criptografía post-cuántica conforme a NIST PQC standards.*