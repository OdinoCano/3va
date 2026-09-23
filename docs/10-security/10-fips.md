# 10 — FIPS 140-3

3va tiene un build FIPS que hace pasar toda la criptografía del runtime y todo
TLS por el **AWS-LC FIPS Cryptographic Module**, a través de `aws-lc-rs`
(feature `fips`) y `rustls` (`default_fips_provider`).

```sh
cargo build --release --locked --features fips --bin 3va
```

Requisitos de compilación: CMake, Go ≥ 1.18 y un compilador C/C++. Son los
mismos que exige `aws-lc-fips-sys`.

En runtime, `crypto.getFips()` devuelve `1` y `crypto.fips === true`. El modo
queda fijo al compilar, igual que el `--force-fips` de Node:
`crypto.setFips(false)` lanza una excepción.

## Estado de validación

| Módulo | Versión | Estado CMVP |
|--------|---------|-------------|
| AWS-LC FIPS | 4.0 (`aws-lc-fips-sys` 0.14.x) | **En proceso** (Modules In Process List) |
| AWS-LC FIPS | 3.x (`aws-lc-fips-sys` 0.13.x) | Validado: certificados #5298 (dinámico) y #5314 (estático) |

3va usa la **4.0**. rustls 0.23.45 es la primera versión con el parche de
RUSTSEC-2026-0285 / CVE-2025-61730 y exige `aws-lc-rs` ≥ 1.18, que solo
enlaza con `aws-lc-fips-sys` 0.14 (AWS-LC FIPS 4.0). Volver a 3.x implicaría
enviar un TLS con una vulnerabilidad conocida. Cuando NIST emita el certificado
de la 4.0, esta tabla se actualiza sin cambios de código.

**Qué se puede afirmar:** "3va, compilado con `--features fips`, usa
exclusivamente el módulo AWS-LC FIPS 4.0 para sus servicios criptográficos".
**Qué no se puede afirmar:** que 3va "está certificado FIPS". Los certificados
CMVP se emiten a módulos criptográficos, no a las aplicaciones que los usan.

Plataformas: AWS limita los builds FIPS estáticos a Linux. Los binarios FIPS se
publican solo para `x86_64-unknown-linux-gnu` y `aarch64-unknown-linux-gnu`.

## Frontera criptográfica

| Superficie | Build normal | Build FIPS |
|------------|--------------|------------|
| `crypto` (hash, HMAC, PBKDF2, AES-CBC/CTR/GCM, RSA, ECDSA, ECDH, keygen, random) | RustCrypto | AWS-LC FIPS ([crypto_fips.rs](../../crates/js/src/builtins/crypto_fips.rs)) |
| `crypto.subtle` | RustCrypto | AWS-LC FIPS (mismas primitivas nativas) |
| `tls.connect`, `net` + TLS, FTP(S), IMAP, POP3, IRC, MQTT | native-tls (OpenSSL / SChannel / Security.framework) | rustls + AWS-LC FIPS ([tls.rs](../../crates/js/src/builtins/tls.rs)) |
| `tls.pqConnect` (X25519MLKEM768) | rustls + aws-lc-rs | rustls + AWS-LC FIPS. El grupo híbrido está aprobado |
| `fetch`, `EventSource` | ureq + rustls/ring | ureq + rustls + AWS-LC FIPS |
| `WebSocket` (`wss://`) | tungstenite + native-tls | tungstenite + rustls + AWS-LC FIPS |
| gRPC (tonic) | proveedor rustls por defecto del proceso | proveedor FIPS instalado como default del proceso |

Al arrancar, un build FIPS ejecuta `aws_lc_rs::try_fips_mode()`, que corre los
self-tests de encendido del módulo. Si fallan, el runtime no arranca.

## Rechazado en el build FIPS

Todo lo que no es un servicio aprobado del módulo lanza `ERR_CRYPTO_FIPS_FORCED`:

- `md5` (hash). `getHashes()` ya no lo lista.
- `scrypt` / `scryptSync`
- `DiffieHellman` / `createDiffieHellman` / `getDiffieHellman`: el DH de campo finito está implementado en JavaScript con BigInt. Usa `createECDH`.
- Ed25519
- RSA con módulo distinto de 2048, 3072, 4096 u 8192 bits
- Firmas RSA con SHA-1 o SHA-224. La **verificación** RSA-PKCS#1 con SHA-1 se mantiene: SP 800-131A la permite para firmas heredadas.
- PBKDF2 con SHA-224
- SSH/SFTP (russh sobre ring, curve25519/chacha20) y WebRTC (DTLS/SRTP en Rust puro)

## Fuera de la frontera

- **Package manager (`3va install`, `3va audit`)**: descarga por reqwest/native-tls y verifica firmas npm con RustCrypto. Es tooling de desarrollo, no crypto que protege datos de la aplicación en ejecución.
- **`vvva_crypto`** (ML-KEM, ML-DSA y firmas Lamport en Rust puro): ningún builtin del runtime lo usa.
- **Usos no criptográficos**, que FIPS no regula: SHA-1 en el handshake de WebSocket (`Sec-WebSocket-Accept`), hashes de caché e integridad de módulos.
- El binario FIPS todavía **enlaza** OpenSSL (native-tls), ring y RustCrypto como dependencias transitivas, pero ninguna ruta del runtime los invoca para servicios criptográficos.
