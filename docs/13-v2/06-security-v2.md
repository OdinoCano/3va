# 06 - Security Hardening v2.0.0

## 6.1 RUSTSEC-2023-0071 (RSA Marvin Attack) Resolution

### Current state (v1.0.0)

`SECURITY.md` documents an explicit acceptance of RUSTSEC-2023-0071. The `rsa 0.9` crate is affected. The rationale accepted in v1.0.0 was: private-key operations are only invoked by user code that explicitly creates a `KeyObject`; timing oracles require an attacker who can observe many decryptions of chosen ciphertexts, which requires the script to be doing RSA decryption in a loop exposed to untrusted input.

### v2.0.0 resolution

v2.0.0 will migrate RSA to one of the following (decision pending upstream fixes):

- **Option A:** Upgrade to `rsa 0.10+` once the upstream fix for RUSTSEC-2023-0071 lands and the advisory is withdrawn.
- **Option B:** Delegate RSA operations to `openssl` via `openssl-sys` (constant-time guarantees by the OpenSSL maintainers) using a feature flag `--features=openssl-crypto`.
- **Option C:** Restrict `createSign`/`createVerify`/`privateDecrypt` to ECDSA-only and remove RSA support entirely, offering ML-DSA-65 (post-quantum) as the long-term replacement.

The chosen option will be documented in `SECURITY.md` before v2.0.0-rc.1.

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
