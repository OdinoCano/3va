# 06 - Security Hardening v2.0.0

## 6.1 RUSTSEC-2023-0071 (RSA Marvin Attack) Resolution

### Current state (v1.0.0)

`SECURITY.md` documents an explicit acceptance of RUSTSEC-2023-0071. The `rsa 0.9` crate is affected. The rationale accepted in v1.0.0 was: private-key operations are only invoked by user code that explicitly creates a `KeyObject`; timing oracles require an attacker who can observe many decryptions of chosen ciphertexts, which requires the script to be doing RSA decryption in a loop exposed to untrusted input.

### v2.0.0 resolution

v2.0.0 resolves the Marvin Attack advisory by migrating the RSA backend:
- **Primary Path (Option A):** Upgrade to `rsa 0.10+` when the upstream fix for RUSTSEC-2023-0071 lands and the advisory is officially withdrawn. This is the preferred way as it preserves pure Rust dependencies.
- **Fallback Path (Option B):** If no upstream fix is available before the release candidate, RSA operations will be delegated to `openssl` via `openssl-sys` (using a build-time feature flag `--features=openssl-crypto`).

This decision and the chosen implementation path will be reflected in `SECURITY.md` before v2.0.0-rc.1.

---

## 6.2 SLSA Level 2 Supply Chain Attestations

v2.0.0 release binaries will carry SLSA level 2 provenance:

- **Build on GitHub Actions** — no local/developer builds published to GitHub Releases.
- **`cosign` binary signatures** — each release artifact signed with Sigstore keyless signing.
- **SBOM** — CycloneDX or SPDX SBOM attached to each GitHub Release.
- **Provenance attestation** — `slsa-github-generator` action generates SLSA provenance JSON; published alongside binaries.

**Verification:**

```bash
# Verify a downloaded binary
cosign verify-blob \
  --certificate 3va-linux-x86_64.tar.gz.crt \
  --signature   3va-linux-x86_64.tar.gz.sig \
  3va-linux-x86_64.tar.gz
```

---

## 6.3 Automated Dependency Audit

A weekly GitHub Actions workflow will run:

```yaml
- cargo audit --deny warnings
- cargo deny check advisories licenses bans
- cargo outdated --exit-code 1  # flag major version drift
```

On any finding, the workflow opens a GitHub Issue tagged `security/dependency` with the advisory ID, affected crate, and a suggested upgrade. Issues are automatically closed when the dependency is updated.

---

## 6.4 Content Security Policy for `3va dev`

The development server (`3va dev`) will inject a default `Content-Security-Policy` header in all HTML responses:

```
Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:;
```

The policy is configurable in `3va.config.ts`:

```ts
export default {
  dev: {
    csp: {
      defaultSrc: ["'self'"],
      scriptSrc: ["'self'", "'unsafe-inline'", 'cdn.example.com'],
      connectSrc: ["'self'", 'ws:', 'wss:'],
    },
  },
};
```

Pass `--no-csp` to disable the header (e.g. when proxying behind a framework's dev server).

---

## 6.5 `require('crypto').pq` API Alignment

v1.0.0 exposes:

```js
const { pq } = require('crypto');
pq.kem.generateKeypair()
pq.dsa.sign(privateKey, message)
```

v2.0.0 aligns naming with the emerging Web Crypto PQ proposals (draft):

```js
const { pq } = require('crypto');
// KEM
pq.kem.generateKeyPair()            // was: generateKeypair (capital P)
pq.kem.encapsulate(publicKey)
pq.kem.decapsulate(privateKey, ciphertext)

// DSA
pq.dsa.generateKeyPair()            // was: generateKeypair
pq.dsa.sign({ key: privateKey, data: message })     // was: sign(key, data)
pq.dsa.verify({ key: publicKey, data, signature })  // was: verify(key, data, sig)
```

**Migration:** A codemod is provided:

```bash
3va codemod --from=1 --to=2 src/
```

The old names (`generateKeypair`, positional `sign(key, data)`) are kept as deprecated aliases through v2.x and removed in v3.0.0.

---

## 6.6 Post-Quantum Web Crypto (`crypto.subtle`) Integration

To align with emerging Web Crypto standards and facilitate PQ adoption in modern runtimes, v2.0.0 introduces experimental support for standard `crypto.subtle` operations utilizing post-quantum algorithms:

### 6.6.1 Algorithm Identifiers

We register two experimental algorithm names in the SubtleCrypto pipeline:
- `ML-KEM-768`: Support for key generation and key encapsulation/decapsulation via `subtle.generateKey`, `subtle.importKey`/`subtle.exportKey` (raw/JWK), and `subtle.deriveBits` / `subtle.deriveKey`.
- `ML-DSA-65`: Support for key generation, signing, and verification via `subtle.generateKey`, `subtle.sign`, and `subtle.verify`.

### 6.6.2 Example Usage

```js
// Generate ML-DSA-65 signing keys
const keyPair = await crypto.subtle.generateKey(
  { name: 'ML-DSA-65' },
  true,
  ['sign', 'verify']
);

// Sign a message
const signature = await crypto.subtle.sign(
  { name: 'ML-DSA-65' },
  keyPair.privateKey,
  new TextEncoder().encode('message')
);

// Verify signature
const isValid = await crypto.subtle.verify(
  { name: 'ML-DSA-65' },
  keyPair.publicKey,
  signature,
  new TextEncoder().encode('message')
);
```

This experimental integration runs in parallel to the `crypto.pq` Node-style module and helps prepare 3va for upstream Web Crypto PQ standards.
