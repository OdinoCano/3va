// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! npm provenance / Sigstore attestation verification.
//!
//! Downloads a package's attestations from the registry provenance endpoint
//! (`/-/npm/v1/attestations/{pkg}@{version}`) and verifies the Sigstore
//! bundle against the Sigstore public-good trust anchors:
//!
//! - The DSSE envelope signature is checked against the ECDSA public key of
//!   the bundle's X.509 leaf certificate.
//! - The leaf certificate chains to a pinned Fulcio root (through the pinned
//!   `sigstore-intermediate`), and was valid at the moment Rekor logged the
//!   entry (`integratedTime`), so the signer is verified — not just the
//!   bundle's internal consistency (VULN-09).
//! - The Rekor transparency-log entry must be from the pinned public Rekor
//!   log (`logID`), its Signed Entry Timestamp verifies against the pinned
//!   Rekor key, and — when the bundle carries an inclusion proof — the
//!   checkpoint signature and the Merkle inclusion proof bind the entry to
//!   the log's signed tree head.
//! - The in-toto statement must name exactly `{pkg}@{version}` as its
//!   subject, and the subject digest must equal the SHA-512 of the tarball
//!   actually downloaded.
//!
//! Scope notes (kept honest):
//! - A missing attestation is "no provenance", not an error — unless the
//!   caller requires provenance (`--require-provenance` /
//!   `_3VA_REQUIRE_PROVENANCE=1`).
//! - An attestation present but invalid (tampered signature, wrong subject,
//!   malformed envelope, a certificate that doesn't chain to Fulcio, or a
//!   tlog entry Rekor didn't sign) IS a hard error.
//! - The trust anchors are pinned constants (`FULCIO_*`, `REKOR_*`). They are
//!   the Sigstore public-good instance's current root, intermediate and Rekor
//!   key; rotating them is a deliberate change and must ship with test
//!   updates. Bundles from other Fulcio/Rekor instances are rejected.

use anyhow::Context;
use base64::Engine;
use serde_json::Value;
#[cfg(not(feature = "fips"))]
use sha2::Sha384;
use sha2::{Digest, Sha256};

/// DSSE Protocol Encoding Algorithm ("PAE") — the byte string that is
/// actually signed. See the DSSE specification v1.
pub fn dsse_pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload_type.len() * 2 + payload.len() + 20);
    out.extend_from_slice(b"DSSEv1 ");
    out.extend_from_slice(payload_type.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload_type.as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload);
    out
}

/// Trust anchors for the Sigstore public-good instance, pinned in this crate
/// (VULN-09). The certificate chain is verified against these, never against
/// material the bundle itself supplies.
const FULCIO_ROOT_V0_DER_B64: &str = "MIIB+DCCAX6gAwIBAgITNVkDZoCiofPDsy7dfm6geLbuhzAKBggqhkjOPQQDAzAqMRUwEwYDVQQKEwxzaWdzdG9yZS5kZXYxETAPBgNVBAMTCHNpZ3N0b3JlMB4XDTIxMDMwNzAzMjAyOVoXDTMxMDIyMzAzMjAyOVowKjEVMBMGA1UEChMMc2lnc3RvcmUuZGV2MREwDwYDVQQDEwhzaWdzdG9yZTB2MBAGByqGSM49AgEGBSuBBAAiA2IABLSyA7Ii5k+pNO8ZEWY0ylemWDowOkNa3kL+GZE5Z5GWehL9/A9bRNA3RbrsZ5i0JcastaRL7Sp5fp/jD5dxqc/UdTVnlvS16an+2Yfswe/QuLolRUCrcOE2+2iA5+tzd6NmMGQwDgYDVR0PAQH/BAQDAgEGMBIGA1UdEwEB/wQIMAYBAf8CAQEwHQYDVR0OBBYEFMjFHQBBmiQpMlEk6w2uSu1KBtPsMB8GA1UdIwQYMBaAFMjFHQBBmiQpMlEk6w2uSu1KBtPsMAoGCCqGSM49BAMDA2gAMGUCMH8liWJfMui6vXXBhjDgY4MwslmN/TJxVe/83WrFomwmNf056y1X48F9c4m3a3ozXAIxAKjRay5/aj/jsKKGIkmQatjI8uupHr/+CxFvaJWmpYqNkLDGRU+9orzh5hI2RrcuaQ==";
const FULCIO_ROOT_V1_DER_B64: &str = "MIIB9zCCAXygAwIBAgIUALZNAPFdxHPwjeDloDwyYChAO/4wCgYIKoZIzj0EAwMwKjEVMBMGA1UEChMMc2lnc3RvcmUuZGV2MREwDwYDVQQDEwhzaWdzdG9yZTAeFw0yMTEwMDcxMzU2NTlaFw0zMTEwMDUxMzU2NThaMCoxFTATBgNVBAoTDHNpZ3N0b3JlLmRldjERMA8GA1UEAxMIc2lnc3RvcmUwdjAQBgcqhkjOPQIBBgUrgQQAIgNiAAT7XeFT4rb3PQGwS4IajtLk3/OlnpgangaBclYpsYBr5i+4ynB07ceb3LP0OIOZdxexX69c5iVuyJRQ+Hz05yi+UF3uBWAlHpiS5sh0+H2GHE7SXrk1EC5m1Tr19L9gg92jYzBhMA4GA1UdDwEB/wQEAwIBBjAPBgNVHRMBAf8EBTADAQH/MB0GA1UdDgQWBBRYwB5fkUWlZql6zJChkyLQKsXF+jAfBgNVHSMEGDAWgBRYwB5fkUWlZql6zJChkyLQKsXF+jAKBggqhkjOPQQDAwNpADBmAjEAj1nHeXZp+13NWBNa+EDsDP8G1WWg1tCMWP/WHPqpaVo0jhsweNFZgSs0eE7wYI4qAjEA2WB9ot98sIkoF3vZYdd3/VtWB5b9TNMea7Ix/stJ5TfcLLeABLE4BNJOsQ4vnBHJ";
const FULCIO_INTERMEDIATE_V1_DER_B64: &str = "MIICGjCCAaGgAwIBAgIUALnViVfnU0brJasmRkHrn/UnfaQwCgYIKoZIzj0EAwMwKjEVMBMGA1UEChMMc2lnc3RvcmUuZGV2MREwDwYDVQQDEwhzaWdzdG9yZTAeFw0yMjA0MTMyMDA2MTVaFw0zMTEwMDUxMzU2NThaMDcxFTATBgNVBAoTDHNpZ3N0b3JlLmRldjEeMBwGA1UEAxMVc2lnc3RvcmUtaW50ZXJtZWRpYXRlMHYwEAYHKoZIzj0CAQYFK4EEACIDYgAE8RVS/ysH+NOvuDZyPIZtilgUF9NlarYpAd9HP1vBBH1U5CV77LSS7s0ZiH4nE7Hv7ptS6LvvR/STk798LVgMzLlJ4HeIfF3tHSaexLcYpSASr1kS0N/RgBJz/9jWCiXno3sweTAOBgNVHQ8BAf8EBAMCAQYwEwYDVR0lBAwwCgYIKwYBBQUHAwMwEgYDVR0TAQH/BAgwBgEB/wIBADAdBgNVHQ4EFgQU39Ppz1YkEZb5qNjpKFWixi4YZD8wHwYDVR0jBBgwFoAUWMAeX5FFpWapesyQoZMi0CrFxfowCgYIKoZIzj0EAwMDZwAwZAIwPCsQK4DYiZYDPIaDi5HFKnfxXx6ASSVmERfsynYBiX2X6SJRnZU84/9DZdnFvvxmAjBOt6QpBlc4J/0DxvkTCqpclvziL6BCCPnjdlIB3Pu3BxsPmygUY7Ii2zbdCdliiow=";
const REKOR_SPKI_DER_B64: &str = "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE2G2Y+2tabdTV5BcGiBIx0a9fAFwrkBbmLSGtks4L3qX6yYY0zufBnhC8Ur/iy55GhWP/9A/bY2LhC30M9+RYtw==";

fn decode_b64(s: &str, what: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("{what}: invalid base64: {e}"))
}

fn fulcio_roots() -> Vec<Vec<u8>> {
    [FULCIO_ROOT_V0_DER_B64, FULCIO_ROOT_V1_DER_B64]
        .iter()
        .map(|b| decode_b64(b, "Fulcio root").expect("embedded Fulcio root is valid base64"))
        .collect()
}

fn fulcio_intermediates() -> Vec<Vec<u8>> {
    vec![
        decode_b64(FULCIO_INTERMEDIATE_V1_DER_B64, "Fulcio intermediate")
            .expect("embedded Fulcio intermediate is valid base64"),
    ]
}

fn rekor_spki() -> Vec<u8> {
    decode_b64(REKOR_SPKI_DER_B64, "Rekor key").expect("embedded Rekor key is valid base64")
}

/// Registry endpoint for a package's attestations. Returns `Ok(None)` when
/// the registry answers 404/405 — "no provenance available" (many private
/// registries don't implement the endpoint).
pub async fn fetch_attestations(
    client: &reqwest::Client,
    base_url: &str,
    pkg_name: &str,
    version: &str,
) -> anyhow::Result<Option<Value>> {
    let url = format!("{base_url}/-/npm/v1/attestations/{pkg_name}@{version}");
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .with_context(|| format!("provenance fetch failed for {pkg_name}@{version}"))?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let data: Value = resp
        .json()
        .await
        .context("provenance endpoint returned non-JSON body")?;
    Ok(Some(data))
}

// ── Minimal ASN.1 DER walking ────────────────────────────────────────────────

struct Tlv<'a> {
    tag: u8,
    /// Full encoded bytes of this TLV (tag + length + content).
    raw: &'a [u8],
    content: &'a [u8],
}

fn read_tlv(input: &[u8]) -> Option<Tlv<'_>> {
    if input.len() < 2 {
        return None;
    }
    let tag = input[0];
    let mut idx = 1;
    let first = input[idx];
    idx += 1;
    let len = if first & 0x80 == 0 {
        first as usize
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || input.len() < idx + n {
            return None;
        }
        let mut len = 0usize;
        for b in &input[idx..idx + n] {
            len = (len << 8) | *b as usize;
        }
        idx += n;
        len
    };
    if input.len() < idx + len {
        return None;
    }
    Some(Tlv {
        tag,
        raw: &input[..idx + len],
        content: &input[idx..idx + len],
    })
}

fn children(content: &[u8]) -> Vec<Tlv<'_>> {
    let mut out = Vec::new();
    let mut rest = content;
    while !rest.is_empty() {
        let Some(t) = read_tlv(rest) else { break };
        let consumed = t.raw.len();
        out.push(t);
        rest = &rest[consumed..];
    }
    out
}

/// The ECDSA public key extracted from a certificate.
pub enum EcPublicKey {
    #[cfg(not(feature = "fips"))]
    P256(p256::ecdsa::VerifyingKey),
    #[cfg(not(feature = "fips"))]
    P384(p384::ecdsa::VerifyingKey),
    /// SEC1 point, already validated by AWS-LC in `extract_ec_public_key`.
    #[cfg(feature = "fips")]
    P256(Vec<u8>),
    #[cfg(feature = "fips")]
    P384(Vec<u8>),
}

const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]; // 1.2.840.10045.2.1
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]; // 1.2.840.10045.3.1.7
const OID_P384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22]; // 1.3.132.0.34

/// Extract the SubjectPublicKeyInfo EC point from an X.509 certificate DER.
///
/// Certificate ::= SEQUENCE { tbsCertificate, ... }; within the TBSCertificate
/// sequence, SubjectPublicKeyInfo follows issuer and validity.
pub fn extract_ec_public_key(cert_der: &[u8]) -> Result<EcPublicKey, String> {
    let cert = read_tlv(cert_der).ok_or("malformed certificate: outer SEQUENCE")?;
    let cert_children = children(cert.content);
    let tbs = cert_children.first().ok_or("certificate has no TBS")?;
    if tbs.tag != 0x30 {
        return Err("TBS not a SEQUENCE".into());
    }

    for elem in children(tbs.content) {
        if elem.tag != 0x30 || elem.content.first() != Some(&0x30) {
            continue; // SPKI starts with SEQUENCE(AlgorithmIdentifier SEQUENCE ...)
        }
        return spki_public_key(&elem);
    }
    Err("no EC SubjectPublicKeyInfo found in certificate".into())
}

/// EC public key from a SubjectPublicKeyInfo SEQUENCE (not a full cert).
fn spki_public_key(spki: &Tlv<'_>) -> Result<EcPublicKey, String> {
    let spki_parts = children(spki.content);
    if spki_parts.len() != 2 {
        return Err("malformed SPKI".into());
    }
    let alg = &spki_parts[0];
    let bitstring = &spki_parts[1];
    if alg.tag != 0x30 || bitstring.tag != 0x03 || bitstring.content.first() != Some(&0x00) {
        return Err("SPKI is not an EC subjectPublicKey".into());
    }
    let alg_parts = children(alg.content);
    let (Some(oid_tlv), curve_oid) = (alg_parts.first(), alg_parts.get(1)) else {
        return Err("SPKI has no key algorithm".into());
    };
    if oid_tlv.content != OID_EC_PUBLIC_KEY {
        return Err("SPKI key is not EC".into());
    }
    let point = &bitstring.content[1..];
    let is_p256 = curve_oid.is_some_and(|c| c.tag == 0x06 && c.content == OID_P256);
    let is_p384 = curve_oid.is_some_and(|c| c.tag == 0x06 && c.content == OID_P384);
    #[cfg(not(feature = "fips"))]
    {
        if is_p256 {
            return p256::ecdsa::VerifyingKey::from_sec1_bytes(point)
                .map(EcPublicKey::P256)
                .map_err(|e| format!("invalid P-256 point: {e}"));
        }
        if is_p384 {
            return p384::ecdsa::VerifyingKey::from_sec1_bytes(point)
                .map(EcPublicKey::P384)
                .map_err(|e| format!("invalid P-384 point: {e}"));
        }
    }
    #[cfg(feature = "fips")]
    {
        use aws_lc_rs::signature::{
            ECDSA_P256_SHA256_ASN1, ECDSA_P384_SHA384_ASN1, ParsedPublicKey,
        };
        if is_p256 {
            return ParsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point)
                .map(|_| EcPublicKey::P256(point.to_vec()))
                .map_err(|e| format!("invalid P-256 point: {e}"));
        }
        if is_p384 {
            return ParsedPublicKey::new(&ECDSA_P384_SHA384_ASN1, point)
                .map(|_| EcPublicKey::P384(point.to_vec()))
                .map_err(|e| format!("invalid P-384 point: {e}"));
        }
    }
    Err("unsupported EC curve in certificate".into())
}

/// Digest used with an ECDSA certificate signature.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SigHash {
    Sha256,
    Sha384,
}

/// The parts of an X.509 certificate needed for chain verification.
struct ParsedCert<'a> {
    /// Full DER of the certificate (to recognise a pinned root).
    der: &'a [u8],
    /// The exact bytes verified by `signature`: the TBSCertificate SEQUENCE.
    tbs: &'a [u8],
    /// Raw DER of the issuer Name and subject Name (byte-exact matching).
    issuer: &'a [u8],
    subject: &'a [u8],
    key: EcPublicKey,
    hash: SigHash,
    /// DER-encoded ECDSA signatureValue.
    signature: Vec<u8>,
    /// Validity window as epoch seconds.
    not_before: i64,
    not_after: i64,
    /// (OID, value) of each extension; the value is the OCTET STRING content.
    extensions: Vec<(&'a [u8], &'a [u8])>,
}

const OID_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13]; // 2.5.29.19
const OID_EXT_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25]; // 2.5.29.37
const OID_SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11]; // 2.5.29.17
const OID_CODE_SIGNING: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x03]; // 1.3.6.1.5.5.7.3.3
// Fulcio extensions, 1.3.6.1.4.1.57264.1.x
const OID_FULCIO_ISSUER_V1: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x83, 0xbf, 0x30, 0x01, 0x01];
const OID_FULCIO_ISSUER_V2: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x83, 0xbf, 0x30, 0x01, 0x08];
const OID_FULCIO_SOURCE_REPO: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x83, 0xbf, 0x30, 0x01, 0x0c];

/// The Fulcio V0 root only issued certificates until the end of 2022.
const FULCIO_ROOT_V0_END: i64 = 1_672_531_199; // 2022-12-31T23:59:59Z

impl ParsedCert<'_> {
    fn extension(&self, oid: &[u8]) -> Option<&[u8]> {
        self.extensions
            .iter()
            .find(|(o, _)| *o == oid)
            .map(|(_, v)| *v)
    }

    /// basicConstraints cA = TRUE.
    fn is_ca(&self) -> bool {
        self.extension(OID_BASIC_CONSTRAINTS)
            .and_then(read_tlv)
            .and_then(|seq| {
                children(seq.content)
                    .into_iter()
                    .next()
                    .map(|b| b.raw.to_vec())
            })
            .is_some_and(|b| b == [0x01, 0x01, 0xff])
    }

    fn has_code_signing_eku(&self) -> bool {
        self.extension(OID_EXT_KEY_USAGE)
            .and_then(read_tlv)
            .is_some_and(|seq| {
                children(seq.content)
                    .iter()
                    .any(|o| o.tag == 0x06 && o.content == OID_CODE_SIGNING)
            })
    }
}

/// Who signed a provenance attestation, from the Fulcio leaf certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerIdentity {
    /// OIDC issuer that authenticated the signer (e.g. GitHub Actions).
    pub issuer: String,
    /// Certificate subject alternative name (workflow URI or email).
    pub san: String,
    /// Source repository the build ran from, when Fulcio recorded it.
    pub source_repo: Option<String>,
}

impl SignerIdentity {
    /// The repository this signer vouches for: the Fulcio source-repository
    /// extension, else the repository part of a GitHub workflow SAN.
    pub fn repository(&self) -> Option<String> {
        self.source_repo.clone().or_else(|| {
            self.san
                .split_once("/.github/")
                .map(|(repo, _)| repo.to_string())
        })
    }

    /// Stable form recorded for trust-on-first-use pinning.
    pub fn pin(&self) -> String {
        format!(
            "{} {}",
            self.issuer,
            self.repository().unwrap_or_else(|| self.san.clone())
        )
    }
}

fn der_string(value: &[u8]) -> Option<String> {
    let t = read_tlv(value)?;
    String::from_utf8(t.content.to_vec()).ok()
}

fn signer_identity(leaf: &ParsedCert<'_>) -> Result<SignerIdentity, String> {
    let issuer = leaf
        .extension(OID_FULCIO_ISSUER_V2)
        .and_then(der_string)
        .or_else(|| {
            leaf.extension(OID_FULCIO_ISSUER_V1)
                .and_then(|v| String::from_utf8(v.to_vec()).ok())
        })
        .ok_or("provenance certificate has no OIDC issuer extension")?;
    let san = leaf
        .extension(OID_SUBJECT_ALT_NAME)
        .and_then(read_tlv)
        .and_then(|seq| {
            children(seq.content)
                .into_iter()
                .find(|g| g.tag == 0x86 || g.tag == 0x81)
                .and_then(|g| String::from_utf8(g.content.to_vec()).ok())
        })
        .ok_or("provenance certificate has no URI/email subject alternative name")?;
    let source_repo = leaf.extension(OID_FULCIO_SOURCE_REPO).and_then(der_string);
    Ok(SignerIdentity {
        issuer,
        san,
        source_repo,
    })
}

/// Normalizes a repository reference (`git+https://…​.git`, `git@github.com:o/r`,
/// `github:o/r`, `o/r`) to `https://host/owner/repo`, lowercase.
pub fn normalize_repository(r: &str) -> String {
    let mut s = r.trim().to_ascii_lowercase();
    for prefix in ["git+", "git://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = if prefix == "git://" {
                format!("https://{rest}")
            } else {
                rest.to_string()
            };
        }
    }
    if let Some(rest) = s.strip_prefix("git@") {
        s = format!("https://{}", rest.replacen(':', "/", 1));
    } else if let Some(rest) = s.strip_prefix("github:") {
        s = format!("https://github.com/{rest}");
    } else if !s.contains("://") && s.split('/').count() == 2 {
        s = format!("https://github.com/{s}");
    }
    s = s
        .replace("ssh://git@", "https://")
        .replace("http://", "https://");
    let s = s.trim_end_matches('/');
    s.strip_suffix(".git").unwrap_or(s).to_string()
}

/// Days since 1970-01-01 for a proleptic-Gregorian date (Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { (m - 3) as i64 } else { m as i64 + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Parse a DER UTCTime (0x17) or GeneralizedTime (0x18) to epoch seconds.
fn parse_time(tlv: &Tlv<'_>) -> Option<i64> {
    let s = std::str::from_utf8(tlv.content).ok()?;
    let s = s.strip_suffix('Z').or_else(|| s.strip_suffix('z'))?;
    let (year, rest) = if tlv.tag == 0x18 {
        let d = s.get(..14)?;
        (d[..4].parse::<i64>().ok()?, &d[4..])
    } else if tlv.tag == 0x17 {
        let d = s.get(..12)?;
        let yy = d[..2].parse::<i64>().ok()?;
        (if yy < 50 { 2000 + yy } else { 1900 + yy }, &d[2..])
    } else {
        return None;
    };
    let mon = rest.get(0..2)?.parse::<u32>().ok()?;
    let day = rest.get(2..4)?.parse::<u32>().ok()?;
    let hh = rest.get(4..6)?.parse::<u32>().ok()?;
    let mm = rest.get(6..8)?.parse::<u32>().ok()?;
    let ss = rest.get(8..10)?.parse::<u32>().ok()?;
    Some(days_from_civil(year, mon, day) * 86400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64)
}

/// Parse a certificate DER into the parts chain verification needs.
fn parse_cert(der: &[u8]) -> Result<ParsedCert<'_>, String> {
    let cert = read_tlv(der).ok_or("malformed certificate: outer SEQUENCE")?;
    if cert.tag != 0x30 {
        return Err("certificate outer is not a SEQUENCE".into());
    }
    let mut parts = children(cert.content).into_iter();
    let tbs = parts.next().ok_or("certificate has no TBS")?;
    let sig_alg = parts
        .next()
        .ok_or("certificate has no signature algorithm")?;
    let sig_value = parts.next().ok_or("certificate has no signature value")?;
    if tbs.tag != 0x30 || sig_alg.tag != 0x30 || sig_value.tag != 0x03 {
        return Err("malformed certificate structure".into());
    }

    // TBS ::= SEQUENCE { [0] version?, serialNumber, signature, issuer,
    //                    validity, subject, subjectPublicKeyInfo, ... }
    let mut tit = children(tbs.content).into_iter();
    let first = tit.next().ok_or("empty TBSCertificate")?;
    let (serial, _tbs_sig, issuer, validity, subject, spki) = if first.tag == 0xa0 {
        (
            tit.next(),
            tit.next(),
            tit.next(),
            tit.next(),
            tit.next(),
            tit.next(),
        )
    } else {
        (
            Some(first),
            tit.next(),
            tit.next(),
            tit.next(),
            tit.next(),
            tit.next(),
        )
    };
    let _serial = serial
        .filter(|t| t.tag == 0x02)
        .ok_or("TBS missing serialNumber")?;
    let _tbs_sig = _tbs_sig
        .filter(|t| t.tag == 0x30)
        .ok_or("TBS missing signature")?;
    let issuer = issuer
        .filter(|t| t.tag == 0x30)
        .ok_or("TBS missing issuer")?;
    let validity = validity
        .filter(|t| t.tag == 0x30)
        .ok_or("TBS missing validity")?;
    let subject = subject
        .filter(|t| t.tag == 0x30)
        .ok_or("TBS missing subject")?;
    let spki = spki.filter(|t| t.tag == 0x30).ok_or("TBS missing SPKI")?;
    let mut extensions = Vec::new();
    for t in tit {
        if t.tag != 0xa3 {
            continue;
        }
        let Some(seq) = read_tlv(t.content) else {
            continue;
        };
        for ext in children(seq.content) {
            let parts = children(ext.content);
            let oid = parts.first().filter(|o| o.tag == 0x06);
            let value = parts.iter().rev().find(|v| v.tag == 0x04);
            if let (Some(oid), Some(value)) = (oid, value) {
                extensions.push((oid.content, value.content));
            }
        }
    }

    let key = spki_public_key(&spki)?;

    let alg_oid = children(sig_alg.content)
        .first()
        .filter(|t| t.tag == 0x06)
        .map(|t| t.content)
        .unwrap_or(&[]);
    let hash = match alg_oid {
        b"\x2a\x86\x48\xce\x3d\x04\x03\x02" => SigHash::Sha256, // ecdsa-with-SHA256
        b"\x2a\x86\x48\xce\x3d\x04\x03\x03" => SigHash::Sha384, // ecdsa-with-SHA384
        _ => return Err("unsupported certificate signature algorithm".into()),
    };

    let sig_content = &sig_value.content;
    if sig_content.first() != Some(&0x00) {
        return Err("malformed signatureValue bit string".into());
    }
    let signature = sig_content[1..].to_vec();

    let vt = children(validity.content);
    let not_before =
        parse_time(vt.first().ok_or("validity missing notBefore")?).ok_or("malformed notBefore")?;
    let not_after =
        parse_time(vt.get(1).ok_or("validity missing notAfter")?).ok_or("malformed notAfter")?;

    Ok(ParsedCert {
        der,
        tbs: tbs.raw,
        issuer: issuer.raw,
        subject: subject.raw,
        key,
        hash,
        signature,
        not_before,
        not_after,
        extensions,
    })
}

/// Verify an ECDSA signature over `data` (prehashed internally per `hash`)
/// against an EC public key, using the certificate's declared digest.
fn verify_ecdsa(
    key: &EcPublicKey,
    hash: SigHash,
    data: &[u8],
    sig_der: &[u8],
) -> Result<(), String> {
    #[cfg(feature = "fips")]
    {
        use aws_lc_rs::signature::{
            ECDSA_P256_SHA256_ASN1, ECDSA_P256_SHA384_ASN1, ECDSA_P384_SHA384_ASN1,
            UnparsedPublicKey,
        };
        let (alg, point) = match (key, hash) {
            (EcPublicKey::P256(p), SigHash::Sha256) => (&ECDSA_P256_SHA256_ASN1, p),
            (EcPublicKey::P256(p), SigHash::Sha384) => (&ECDSA_P256_SHA384_ASN1, p),
            (EcPublicKey::P384(p), SigHash::Sha384) => (&ECDSA_P384_SHA384_ASN1, p),
            (EcPublicKey::P384(_), SigHash::Sha256) => {
                return Err("P-384 certificate signed with SHA-256 is unsupported".into());
            }
        };
        UnparsedPublicKey::new(alg, point)
            .verify(data, sig_der)
            .map_err(|_| "ECDSA signature mismatch".to_string())
    }
    #[cfg(not(feature = "fips"))]
    {
        use ecdsa::signature::hazmat::PrehashVerifier as _;
        use p256::ecdsa::Signature as Sig256;
        use p384::ecdsa::Signature as Sig384;
        match (key, hash) {
            (EcPublicKey::P256(vk), SigHash::Sha256) => {
                let sig =
                    Sig256::from_der(sig_der).map_err(|e| format!("bad DER signature: {e}"))?;
                vk.verify_prehash(&Sha256::digest(data), &sig)
                    .map_err(|_| "ECDSA signature mismatch".to_string())
            }
            (EcPublicKey::P256(vk), SigHash::Sha384) => {
                let sig =
                    Sig256::from_der(sig_der).map_err(|e| format!("bad DER signature: {e}"))?;
                vk.verify_prehash(&Sha384::digest(data), &sig)
                    .map_err(|_| "ECDSA signature mismatch".to_string())
            }
            (EcPublicKey::P384(vk), SigHash::Sha384) => {
                let sig =
                    Sig384::from_der(sig_der).map_err(|e| format!("bad DER signature: {e}"))?;
                vk.verify_prehash(&Sha384::digest(data), &sig)
                    .map_err(|_| "ECDSA signature mismatch".to_string())
            }
            (EcPublicKey::P384(_), SigHash::Sha256) => {
                Err("P-384 certificate signed with SHA-256 is unsupported".into())
            }
        }
    }
}

#[cfg(feature = "fips")]
fn verify_dsse_signature(
    key: &EcPublicKey,
    payload_type: &str,
    payload: &[u8],
    sig_der: &[u8],
) -> Result<(), String> {
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1, ECDSA_P384_SHA384_ASN1, UnparsedPublicKey};
    let pae = dsse_pae(payload_type, payload);
    let (alg, point) = match key {
        EcPublicKey::P256(p) => (&ECDSA_P256_SHA256_ASN1, p),
        EcPublicKey::P384(p) => (&ECDSA_P384_SHA384_ASN1, p),
    };
    UnparsedPublicKey::new(alg, point)
        .verify(&pae, sig_der)
        .map_err(|_| "DSSE signature mismatch".to_string())
}

#[cfg(not(feature = "fips"))]
fn verify_dsse_signature(
    key: &EcPublicKey,
    payload_type: &str,
    payload: &[u8],
    sig_der: &[u8],
) -> Result<(), String> {
    use p256::ecdsa::{Signature as Sig256, VerifyingKey as Vk256};
    use p384::ecdsa::{Signature as Sig384, VerifyingKey as Vk384};
    let pae = dsse_pae(payload_type, payload);
    match key {
        EcPublicKey::P256(vk) => {
            let sig = Sig256::from_der(sig_der).map_err(|e| format!("bad DER signature: {e}"))?;
            use ecdsa::signature::Verifier as _;
            Vk256::verify(vk, &pae, &sig).map_err(|_| "DSSE signature mismatch".to_string())
        }
        EcPublicKey::P384(vk) => {
            let sig = Sig384::from_der(sig_der).map_err(|e| format!("bad DER signature: {e}"))?;
            use ecdsa::signature::Verifier as _;
            Vk384::verify(vk, &pae, &sig).map_err(|_| "DSSE signature mismatch".to_string())
        }
    }
}

/// Verify the leaf certificate chains (through the bundle's own intermediates
/// and the pinned Fulcio intermediate) to a pinned Fulcio root, and that it
/// was valid at the moment Rekor accepted the entry (`as_of`, epoch seconds).
/// The signer of a Sigstore bundle is therefore verified, not just the
/// bundle's internal consistency (VULN-09).
fn verify_fulcio_chain(
    leaf_der: &[u8],
    bundle_chain: &[Vec<u8>],
    as_of: i64,
) -> Result<(), String> {
    let roots = fulcio_roots();
    let mut candidates: Vec<Vec<u8>> = bundle_chain.to_vec();
    candidates.extend(fulcio_intermediates());

    let mut current = parse_cert(leaf_der)?;
    // A signing certificate, not a CA: otherwise any Fulcio-issued leaf
    // could mint further "leaves" with whatever identity it liked.
    if current.is_ca() || !current.has_code_signing_eku() {
        return Err("provenance certificate is not a code-signing leaf".into());
    }
    if !(current.not_before <= as_of && as_of <= current.not_after) {
        return Err(format!(
            "provenance certificate not valid at tlog integration time {as_of} \
             (valid {}..{})",
            current.not_before, current.not_after
        ));
    }
    for _ in 0..4 {
        if roots.iter().any(|r| r.as_slice() == current.der) {
            let v0 = decode_b64(FULCIO_ROOT_V0_DER_B64, "Fulcio root")?;
            if current.der == v0.as_slice() && as_of > FULCIO_ROOT_V0_END {
                return Err("Fulcio V0 root was retired at the end of 2022".into());
            }
            return Ok(());
        }
        // The issuer is the candidate whose subject matches AND whose key
        // verifies the current certificate: two pinned roots share the
        // `sigstore` subject, so name-matching alone picks the wrong one.
        let mut issuer: Option<ParsedCert> = None;
        for candidate_der in candidates.iter().chain(roots.iter()) {
            let Ok(candidate) = parse_cert(candidate_der) else {
                continue;
            };
            if candidate.subject != current.issuer {
                continue;
            }
            // Only CA certificates may issue (pinned roots are CAs).
            if !candidate.is_ca() {
                continue;
            }
            if verify_ecdsa(
                &candidate.key,
                current.hash,
                current.tbs,
                &current.signature,
            )
            .is_err()
            {
                continue;
            }
            if candidate.not_before <= as_of && as_of <= candidate.not_after {
                issuer = Some(candidate);
                break;
            }
        }
        let Some(issuer) = issuer else {
            return Err(format!(
                "provenance certificate is not issued by a pinned Fulcio CA \
                 (issuer {:?})",
                String::from_utf8_lossy(current.issuer)
            ));
        };
        current = issuer;
    }
    Err("certificate chain too deep".into())
}

// ── Rekor transparency log ────────────────────────────────────────────────

/// Parse a JSON number that npm sometimes ships as a string.
fn json_u64(v: &Value, what: &str) -> Result<u64, String> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| format!("tlog entry missing {what}"))
}

/// Verify one Rekor tlog entry from a bundle against the pinned Rekor key:
/// the log identity, the Signed Entry Timestamp, and — when the bundle
/// carries an inclusion proof — the checkpoint signature and the Merkle
/// inclusion proof up to the signed tree head (VULN-09).
fn verify_rekor(entry: &Value) -> Result<(), String> {
    let spki = rekor_spki();
    let expected_log_id = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&spki));
    let log_id_b64 = entry["logId"]["keyId"]
        .as_str()
        .ok_or("tlog entry missing logId.keyId")?;
    if log_id_b64 != expected_log_id {
        return Err("tlog entry is not from the trusted Sigstore Rekor log".into());
    }
    let log_id = decode_b64(log_id_b64, "logId.keyId")?;
    let body_b64 = entry["canonicalizedBody"]
        .as_str()
        .ok_or("tlog entry missing canonicalizedBody")?;
    let integrated_time = json_u64(&entry["integratedTime"], "integratedTime")? as i64;
    let log_index = json_u64(&entry["logIndex"], "logIndex")?;
    let set_b64 = entry["inclusionPromise"]["signedEntryTimestamp"]
        .as_str()
        .ok_or("tlog entry missing signedEntryTimestamp")?;

    // SET: ECDSA-P-256/SHA-256 over the RFC8785-canonical JSON of
    // {body, integratedTime, logID(hex), logIndex} — exactly what Rekor signs.
    let set_payload = format!(
        "{{\"body\":\"{body_b64}\",\"integratedTime\":{integrated_time},\"logID\":\"{}\",\"logIndex\":{log_index}}}",
        hex::encode(&log_id)
    );
    let set_der = decode_b64(set_b64, "signedEntryTimestamp")?;
    let rekor_key = rekor_public_key()?;
    verify_ecdsa(
        &rekor_key,
        SigHash::Sha256,
        set_payload.as_bytes(),
        &set_der,
    )
    .map_err(|_| "Rekor signed entry timestamp (SET) is invalid".to_string())?;

    if let Some(proof) = entry["inclusionProof"].as_object() {
        let envelope = proof["checkpoint"]["envelope"]
            .as_str()
            .ok_or("inclusionProof missing checkpoint envelope")?;
        let sep = envelope
            .find("\n\n")
            .ok_or("malformed checkpoint: missing note separator")?;
        // Note body is everything up to and including the root-hash line's
        // newline; the signed-note signature covers exactly those bytes.
        let note_body = &envelope[..sep + 1];
        let sig_field = envelope[sep + 2..]
            .split('\n')
            .find(|l| l.contains('—'))
            .and_then(|l| l.split_whitespace().last())
            .ok_or("malformed checkpoint: no signature line")?;
        let raw = decode_b64(sig_field, "checkpoint signature")?;
        if raw.len() < 4 {
            return Err("malformed checkpoint signature".into());
        }
        let (key_hint, sig_der) = raw.split_at(4);
        if key_hint != &log_id[..4] {
            return Err("checkpoint signature key hint does not match the Rekor log".into());
        }
        verify_ecdsa(&rekor_key, SigHash::Sha256, note_body.as_bytes(), sig_der)
            .map_err(|_| "checkpoint signature is invalid".to_string())?;

        let root_b64 = note_body
            .split('\n')
            .nth(2)
            .ok_or("malformed checkpoint note")?;
        let root_hash = decode_b64(root_b64, "checkpoint root hash")?;
        let body = decode_b64(body_b64, "canonicalizedBody")?;
        let leaf_hash = merkle_leaf_hash(&body);
        verify_inclusion(proof, &leaf_hash, &root_hash)?;
    }
    Ok(())
}

/// Whether a Rekor entry records exactly this attestation: the payload's
/// SHA-256, this DSSE signature and this leaf certificate. Without it any
/// valid entry copied from another bundle would "log" a forged one, and its
/// integratedTime decides whether the certificate counts as valid.
fn entry_binds(entry: &Value, leaf_der: &[u8], sig_b64: &str, payload: &[u8]) -> bool {
    let Some(body) = entry["canonicalizedBody"]
        .as_str()
        .and_then(|b| decode_b64(b, "canonicalizedBody").ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    else {
        return false;
    };
    let payload_hash = hex::encode(Sha256::digest(payload));
    let cert_is_leaf = |v: &Value| {
        v.as_str()
            .and_then(|b| decode_b64(b, "tlog certificate").ok())
            .and_then(|pem| String::from_utf8(pem).ok())
            .map(|pem| {
                pem.lines()
                    .filter(|l| !l.starts_with("-----"))
                    .collect::<String>()
            })
            .and_then(|b| decode_b64(&b, "tlog certificate").ok())
            .is_some_and(|der| der == leaf_der)
    };
    let sigs_match = |list: &Value, sig_key: &str, sig_value: &str, cert_key: &str| {
        list.as_array().is_some_and(|sigs| {
            sigs.iter()
                .any(|s| s[sig_key].as_str() == Some(sig_value) && cert_is_leaf(&s[cert_key]))
        })
    };
    match body["kind"].as_str() {
        // intoto v0.0.2: the signature is stored base64-encoded again.
        Some("intoto") => {
            let content = &body["spec"]["content"];
            let double = base64::engine::general_purpose::STANDARD.encode(sig_b64);
            content["payloadHash"]["value"].as_str() == Some(payload_hash.as_str())
                && sigs_match(
                    &content["envelope"]["signatures"],
                    "sig",
                    &double,
                    "publicKey",
                )
        }
        Some("dsse") => {
            let spec = &body["spec"];
            spec["payloadHash"]["value"].as_str() == Some(payload_hash.as_str())
                && sigs_match(&spec["signatures"], "signature", sig_b64, "verifier")
        }
        _ => false,
    }
}

/// The Rekor log's public key as an [`EcPublicKey`] (P-256).
fn rekor_public_key() -> Result<EcPublicKey, String> {
    let der = rekor_spki();
    let spki = read_tlv(&der).ok_or("malformed Rekor SPKI")?;
    spki_public_key(&spki)
}

/// RFC 6962 leaf hash: `SHA256(0x00 || leaf_bytes)`.
fn merkle_leaf_hash(body: &[u8]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(1 + body.len());
    buf.push(0x00);
    buf.extend_from_slice(body);
    Sha256::digest(buf).into()
}

/// RFC 6962 interior-node hash: `SHA256(0x01 || left || right)`.
fn merkle_node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 65];
    buf[0] = 0x01;
    buf[1..33].copy_from_slice(left);
    buf[33..].copy_from_slice(right);
    Sha256::digest(buf).into()
}

/// Number of proof hashes at the bottom of the path (`decompInclProof.inner`).
fn inner_proof_size(index: u64, size: u64) -> u32 {
    let x = index ^ (size - 1);
    64 - x.leading_zeros()
}

fn chain_inner(mut seed: [u8; 32], hashes: &[[u8; 32]], index: u64) -> [u8; 32] {
    for (i, h) in hashes.iter().enumerate() {
        seed = if (index >> i) & 1 == 1 {
            merkle_node_hash(h, &seed)
        } else {
            merkle_node_hash(&seed, h)
        };
    }
    seed
}

fn chain_border_right(mut seed: [u8; 32], hashes: &[[u8; 32]]) -> [u8; 32] {
    for h in hashes {
        seed = merkle_node_hash(h, &seed);
    }
    seed
}

/// Verify the Merkle audit path climbs from `leaf_hash` to the checkpoint's
/// signed root hash (RFC 6962 inclusion proof).
fn verify_inclusion(
    proof: &serde_json::Map<String, Value>,
    leaf_hash: &[u8; 32],
    root_hash: &[u8],
) -> Result<(), String> {
    let index = json_u64(&proof["logIndex"], "inclusionProof logIndex")?;
    let tree_size = json_u64(&proof["treeSize"], "inclusionProof treeSize")?;
    if tree_size == 0 || index >= tree_size {
        return Err("inclusion proof index out of range".into());
    }
    let hashes: Vec<[u8; 32]> = proof["hashes"]
        .as_array()
        .ok_or("inclusionProof missing hashes")?
        .iter()
        .map(|h| {
            let s = h.as_str().ok_or("inclusion proof hash is not a string")?;
            let b = decode_b64(s, "inclusion proof hash")?;
            b.try_into()
                .map_err(|_| "inclusion proof hash is not 32 bytes".to_string())
        })
        .collect::<Result<_, _>>()?;
    let inner = inner_proof_size(index, tree_size) as usize;
    let border = (index >> inner as u64).count_ones() as usize;
    if hashes.len() != inner + border {
        return Err("inclusion proof hash count mismatch".into());
    }
    let seed = chain_inner(*leaf_hash, &hashes[..inner], index);
    let computed = chain_border_right(seed, &hashes[inner..]);
    if &computed[..] != root_hash {
        return Err("inclusion proof does not match the checkpoint root hash".into());
    }
    Ok(())
}

/// Expected subject forms for `{pkg}@{version}` in an in-toto statement.
/// Builds the npm registry's purl for a package name, matching real
/// attestation subjects exactly: a scoped name's leading `@` is percent-
/// encoded (`%40`) but the `/` before the package name is left bare —
/// e.g. `@babel/traverse` → `pkg:npm/%40babel/traverse`. Getting this
/// wrong makes every scoped package's (valid) provenance look tampered.
fn npm_purl_name(pkg_name: &str) -> String {
    match pkg_name.strip_prefix('@') {
        Some(rest) => format!("pkg:npm/%40{rest}"),
        None => format!("pkg:npm/{pkg_name}"),
    }
}

fn subject_matches(statement: &Value, pkg_name: &str, version: &str) -> bool {
    let purl = format!("{}@{version}", npm_purl_name(pkg_name));
    statement["subject"]
        .as_array()
        .map(|subjects| {
            subjects.iter().any(|s| {
                s["name"].as_str() == Some(&purl)
                    || s["name"].as_str() == Some(&format!("{pkg_name}@{version}"))
            })
        })
        .unwrap_or(false)
}

/// True when some subject's `digest.sha512` (hex, per in-toto) equals the
/// tarball's — binds the attestation to the bytes, not just the version.
fn subject_digest_matches(statement: &Value, tarball_sha512_hex: &str) -> bool {
    statement["subject"].as_array().is_some_and(|subjects| {
        subjects.iter().any(|s| {
            s["digest"]["sha512"]
                .as_str()
                .is_some_and(|d| d.eq_ignore_ascii_case(tarball_sha512_hex))
        })
    })
}

/// Outcome of [`verify_attestations`] on a response with no hard failure.
#[derive(Debug)]
pub enum VerifyOutcome {
    /// `n` attestations have a valid signature under the key in their own
    /// leaf certificate, the leaf chains to a pinned Fulcio root, and the
    /// Rekor entry is bound to the public log. The signer is verified
    /// (VULN-09).
    Verified(Vec<SignerIdentity>),
    /// The bundle uses a verification-material format this verifier doesn't
    /// support yet (e.g. a bare `publicKey` hint instead of an X.509 leaf
    /// certificate) — not evidence of tampering, just nothing we can check.
    /// Treated the same as "no provenance published" by the caller.
    Unsupported(String),
}

/// Verify every attestation in a registry provenance response.
///
/// Returns [`VerifyOutcome::Verified`] with the signer of each bundle, or [`VerifyOutcome::Unsupported`] when the bundle's
/// format can't be checked at all. Any malformed, mismatched, or tampered
/// attestation — i.e. anything actually indicating a problem rather than
/// just "unverifiable" — is a hard error (`Err`).
pub fn verify_attestations(
    response: &Value,
    pkg_name: &str,
    version: &str,
    tarball_sha512_hex: &str,
) -> Result<VerifyOutcome, String> {
    let attestations = response["attestations"]
        .as_array()
        .ok_or("provenance response missing 'attestations' array")?;
    if attestations.is_empty() {
        return Err("provenance response contains zero attestations".into());
    }
    let mut signers = Vec::new();
    for att in attestations {
        let predicate_type = att["predicateType"]
            .as_str()
            .ok_or("attestation missing predicateType")?;
        let bundle = &att["bundle"];
        let media_type = bundle["mediaType"]
            .as_str()
            .ok_or("bundle missing mediaType")?
            .to_string();
        if !media_type.starts_with("application/vnd.dev.sigstore.bundle") {
            return Err(format!("unsupported bundle mediaType: {media_type}"));
        }
        let envelope = &bundle["dsseEnvelope"];
        if !envelope.is_object() {
            return Err("bundle has no dsseEnvelope".into());
        }
        let payload_type = envelope["payloadType"]
            .as_str()
            .ok_or("dsseEnvelope missing payloadType")?;
        if payload_type != "application/vnd.in-toto+json" {
            return Err(format!("unsupported DSSE payloadType: {payload_type}"));
        }
        let payload_b64 = envelope["payload"]
            .as_str()
            .ok_or("dsseEnvelope missing payload")?;
        let payload = base64::engine::general_purpose::STANDARD
            .decode(payload_b64)
            .map_err(|e| format!("payload is not valid base64: {e}"))?;
        let statement: Value = serde_json::from_slice(&payload)
            .map_err(|e| format!("in-toto payload is not JSON: {e}"))?;
        if !subject_matches(&statement, pkg_name, version) {
            return Err(format!(
                "provenance subject does not match {pkg_name}@{version}"
            ));
        }
        if !subject_digest_matches(&statement, tarball_sha512_hex) {
            return Err(format!(
                "provenance subject digest does not match the downloaded tarball of \
                 {pkg_name}@{version}"
            ));
        }
        if statement["_type"].as_str() != Some("https://in-toto.io/Statement/v1")
            && statement["_type"].as_str() != Some("https://in-toto.io/Statement/v0.1")
        {
            return Err("payload is not an in-toto Statement".into());
        }
        if predicate_type != statement["predicateType"].as_str().unwrap_or("") {
            return Err("predicateType mismatch between attestation and statement".into());
        }

        // Public key: X.509 certificate chain (v0.1+ bundles) preferred.
        let vm = &bundle["verificationMaterial"];
        let certs = vm["x509CertificateChain"]["certificates"].as_array();
        let leaf_der = match certs
            .and_then(|c| c.first())
            .and_then(|c| c["rawBytes"].as_str())
        {
            Some(b64) => decode_b64(b64, "certificate rawBytes")?,
            None => {
                // Older/alternate bundles carry a bare public-key hint
                // instead of a Fulcio-issued cert — a legitimate Sigstore
                // shape (see `undici`'s real npm attestations), just one
                // this verifier can't check without a trusted keyring to
                // resolve the hint against. Not a tampering signal.
                return Ok(VerifyOutcome::Unsupported(
                    "bundle verificationMaterial carries no x509CertificateChain".into(),
                ));
            }
        };
        let key = extract_ec_public_key(&leaf_der)?;
        let chain_ders: Vec<Vec<u8>> = certs
            .map(|c| {
                c.iter()
                    .filter_map(|c| c["rawBytes"].as_str())
                    .map(|b| decode_b64(b, "certificate rawBytes"))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        let sigs = envelope["signatures"]
            .as_array()
            .ok_or("dsseEnvelope has no signatures array")?;
        if sigs.is_empty() {
            return Err("dsseEnvelope signatures array empty".into());
        }
        let mut verified_sig: Option<&str> = None;
        for sig in sigs {
            let sig_b64 = sig["sig"].as_str().ok_or("signature missing 'sig'")?;
            let sig_der = base64::engine::general_purpose::STANDARD
                .decode(sig_b64)
                .map_err(|e| format!("signature is not valid base64: {e}"))?;
            if verify_dsse_signature(&key, payload_type, &payload, &sig_der).is_ok() {
                verified_sig = Some(sig_b64);
                break;
            }
        }
        let Some(verified_sig) = verified_sig else {
            return Err(format!(
                "DSSE signature verification failed for {pkg_name}@{version} \
                 (predicate {predicate_type})"
            ));
        };
        // Trust the signer: chain to Fulcio and bind to Rekor (VULN-09).
        let tlog = vm["tlogEntries"]
            .as_array()
            .ok_or("bundle has no Rekor tlog entries")?;
        if tlog.is_empty() {
            return Err("bundle has no Rekor transparency log entries".into());
        }
        let mut as_of = None;
        for entry in tlog {
            verify_rekor(entry).map_err(|why| format!("invalid Rekor entry: {why}"))?;
            if as_of.is_none() && entry_binds(entry, &leaf_der, verified_sig, &payload) {
                as_of = Some(json_u64(&entry["integratedTime"], "integratedTime")? as i64);
            }
        }
        let as_of =
            as_of.ok_or("no Rekor entry in the bundle records this signature and certificate")?;
        verify_fulcio_chain(&leaf_der, &chain_ders, as_of)
            .map_err(|why| format!("provenance signer is not Fulcio-issued: {why}"))?;
        signers.push(signer_identity(&parse_cert(&leaf_der)?)?);
    }
    Ok(VerifyOutcome::Verified(signers))
}

/// Signer identities recorded per package the first time a verified
/// provenance is installed (trust on first use), in
/// `.3va/provenance-signers.json`; commit it like a lockfile. A later
/// version signed by someone else, or shipped without provenance, is
/// refused: a registry that controls both tarball and attestation can still
/// get a Fulcio certificate for its own identity, but not for the one
/// recorded here (VULN-09).
pub fn check_signer_pin(
    project_root: &std::path::Path,
    pkg_name: &str,
    signer: Option<&str>,
) -> Result<(), String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = project_root.join(".3va").join("provenance-signers.json");
    let mut pins: std::collections::BTreeMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();
    match (pins.get(pkg_name), signer) {
        (Some(recorded), Some(s)) if recorded == s => Ok(()),
        (Some(recorded), Some(s)) => Err(format!(
            "provenance is signed by `{s}`, but earlier versions were signed by `{recorded}` \
             (.3va/provenance-signers.json)"
        )),
        (Some(recorded), None) => Err(format!(
            "earlier versions carried provenance signed by `{recorded}`, this one has none \
             (.3va/provenance-signers.json)"
        )),
        (None, Some(s)) => {
            pins.insert(pkg_name.to_string(), s.to_string());
            let json = serde_json::to_string_pretty(&pins).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(path.parent().unwrap())
                .and_then(|_| std::fs::write(&path, json + "\n"))
                .map_err(|e| format!("cannot record provenance signer: {e}"))
        }
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/npm_provenance_sigstore_3.0.0.json"
    ));

    #[test]
    fn pae_matches_spec_vector() {
        // PAE = "DSSEv1" LEN(type) type LEN(payload) payload, single spaces.
        assert_eq!(dsse_pae("a", b"b"), b"DSSEv1 1 a 1 b".to_vec());
        assert_eq!(
            dsse_pae("application/vnd.in-toto+json", br#"{"x":1}"#),
            b"DSSEv1 28 application/vnd.in-toto+json 7 {\"x\":1}".to_vec()
        );
    }

    // sha512 of the real sigstore-3.0.0.tgz, as named by the fixture's subject.
    const FIXTURE_SHA512: &str = "3c73227e187710de25a0c7070b3ea5deffe5bb3813df36bef5ff2cb9b1a078c3636c98f31f8223fd8a17dc6beefa46a8b894489557531c70911000d87fe66d78";

    #[test]
    fn real_npm_provenance_fixture_verifies() {
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        match verify_attestations(&resp, "sigstore", "3.0.0", FIXTURE_SHA512)
            .expect("real npm provenance bundle must verify")
        {
            VerifyOutcome::Verified(signers) => {
                assert_eq!(
                    signers.len(),
                    1,
                    "fixture holds one SLSA provenance attestation"
                );
                assert_eq!(
                    signers[0].issuer,
                    "https://token.actions.githubusercontent.com"
                );
                assert_eq!(
                    signers[0].repository().as_deref(),
                    Some("https://github.com/sigstore/sigstore-js")
                );
            }
            VerifyOutcome::Unsupported(why) => {
                panic!("expected Verified, got Unsupported: {why}")
            }
        }
    }

    #[test]
    fn attestation_for_other_bytes_is_rejected() {
        // VULN-09: same name and version, different tarball.
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        let err = verify_attestations(&resp, "sigstore", "3.0.0", &"0".repeat(128))
            .expect_err("digest mismatch must be rejected");
        assert!(
            err.contains("digest does not match"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn scoped_package_purl_matches_real_npm_subject_encoding() {
        // Real npm attestation subjects percent-encode only the leading
        // `@` of a scope (`pkg:npm/%40babel/traverse@7.29.8`), not the `/`
        // — this used to be built unencoded and rejected every scoped
        // package's valid provenance as a subject mismatch.
        assert_eq!(
            npm_purl_name("@babel/traverse"),
            "pkg:npm/%40babel/traverse"
        );
        assert_eq!(npm_purl_name("undici"), "pkg:npm/undici");
    }

    #[test]
    fn tampered_signature_fails_hard() {
        let mut resp: Value = serde_json::from_str(FIXTURE).unwrap();
        let sig = resp["attestations"][0]["bundle"]["dsseEnvelope"]["signatures"][0]["sig"]
            .as_str()
            .unwrap();
        // Flip the tail of the base64 signature deterministically.
        let mut chars: Vec<char> = sig.chars().collect();
        let last_alnum = chars.iter_mut().rev().find(|c| **c != '=').unwrap();
        *last_alnum = if *last_alnum == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        resp["attestations"][0]["bundle"]["dsseEnvelope"]["signatures"][0]["sig"] =
            Value::String(tampered);
        let err = verify_attestations(&resp, "sigstore", "3.0.0", FIXTURE_SHA512)
            .expect_err("tampered attestation must be rejected");
        assert!(
            err.contains("signature verification failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn wrong_subject_fails_even_with_valid_signature() {
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        let err = verify_attestations(&resp, "evil-pkg", "3.0.0", FIXTURE_SHA512)
            .expect_err("subject mismatch must be rejected");
        assert!(
            err.contains("subject does not match evil-pkg@3.0.0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn extracts_ec_key_from_fixture_certificate() {
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        let b64 = resp["attestations"][0]["bundle"]["verificationMaterial"]["x509CertificateChain"]
            ["certificates"][0]["rawBytes"]
            .as_str()
            .unwrap();
        let der = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert!(matches!(
            extract_ec_public_key(&der),
            Ok(EcPublicKey::P256(_))
        ));
    }

    #[test]
    fn empty_attestation_list_is_a_hard_error() {
        let resp = serde_json::json!({ "attestations": [] });
        assert!(verify_attestations(&resp, "a", "1.0.0", FIXTURE_SHA512).is_err());
    }

    fn fixture_leaf_der() -> Vec<u8> {
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        let b64 = resp["attestations"][0]["bundle"]["verificationMaterial"]["x509CertificateChain"]
            ["certificates"][0]["rawBytes"]
            .as_str()
            .unwrap();
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap()
    }

    fn fixture_tlog_entry() -> Value {
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        resp["attestations"][0]["bundle"]["verificationMaterial"]["tlogEntries"][0].clone()
    }

    #[test]
    fn fixture_leaf_chains_to_pinned_fulcio_root() {
        let leaf = fixture_leaf_der();
        // The fixture entry was logged at integratedTime 1728922425 (Oct 14
        // 2024 16:13:45Z), the leaf's notBefore.
        verify_fulcio_chain(&leaf, &[], 1728922425)
            .expect("the real npm provenance certificate must chain to Fulcio");
    }

    #[test]
    fn chain_rejects_cert_not_valid_at_tlog_time() {
        let leaf = fixture_leaf_der();
        let err = verify_fulcio_chain(&leaf, &[], 1_800_000_000)
            .expect_err("leaf expired long before this as_of must be rejected");
        assert!(
            err.contains("not valid at tlog integration time"),
            "unexpected error: {err}"
        );
    }

    // A self-signed certificate, however internally consistent, must not be
    // accepted as a signer: VULN-09 is the property that "the bundle is
    // internally consistent" is NOT enough.
    #[test]
    fn self_signed_certificate_is_rejected_by_chain() {
        use p256::ecdsa::SigningKey;
        let secret = [7u8; 32];
        let signer = SigningKey::from_bytes(&secret.into()).unwrap();
        let cert = make_self_signed_cert(&signer);
        // Even verified "at" its own validity time, no pinned CA issues it.
        let err = verify_fulcio_chain(&cert, &[], 1_700_000_000)
            .expect_err("a self-signed cert must not chain to Fulcio");
        assert!(
            err.contains("not issued by a pinned Fulcio CA")
                || err.contains("chain signature invalid")
                || err.contains("not a code-signing leaf"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn fixture_rekor_entry_verifies() {
        let entry = fixture_tlog_entry();
        verify_rekor(&entry).expect("the real npm provenance tlog entry must verify");
    }

    #[test]
    fn tampered_rekor_set_is_rejected() {
        let mut entry = fixture_tlog_entry();
        let set = entry["inclusionPromise"]["signedEntryTimestamp"]
            .as_str()
            .unwrap()
            .to_string();
        let mut chars: Vec<char> = set.chars().collect();
        let last_alnum = chars.iter_mut().rev().find(|c| **c != '=').unwrap();
        *last_alnum = if *last_alnum == 'A' { 'B' } else { 'A' };
        entry["inclusionPromise"]["signedEntryTimestamp"] =
            Value::String(chars.into_iter().collect());
        let err = verify_rekor(&entry).expect_err("a tampered SET must be rejected");
        assert!(
            err.contains("SET") || err.contains("signature"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn foreign_rekor_log_is_rejected() {
        let mut entry = fixture_tlog_entry();
        // A log ID that is not SHA256 of the pinned Rekor key.
        entry["logId"]["keyId"] =
            Value::String("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into());
        let err = verify_rekor(&entry).expect_err("a foreign log ID must be rejected");
        assert!(
            err.contains("not from the trusted Sigstore Rekor log"),
            "unexpected error: {err}"
        );
    }

    // ── Minimal DER writer for self-signed test certificates ────────────────

    use p256::ecdsa::SigningKey;

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        let n = content.len();
        if n < 0x80 {
            out.push(n as u8);
        } else {
            let bytes = n.to_be_bytes();
            let lead = bytes.iter().position(|b| *b != 0).unwrap_or(7);
            out.push(0x80 | (8 - lead) as u8);
            out.extend_from_slice(&bytes[lead..]);
        }
        out.extend_from_slice(content);
        out
    }

    const OID_EC_PUB: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    const OID_P256_CURVE: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
    const OID_ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];

    fn oid(bytes: &[u8]) -> Vec<u8> {
        tlv(0x06, bytes)
    }

    fn make_self_signed_cert(signer: &SigningKey) -> Vec<u8> {
        use ecdsa::signature::Signer as _;
        let spki = {
            let alg = tlv(0x30, &[oid(OID_EC_PUB), oid(OID_P256_CURVE)].concat());
            let point = signer.verifying_key().to_encoded_point(false);
            let bitstring = tlv(0x03, &[&[0x00], point.as_bytes()].concat());
            tlv(0x30, &[alg, bitstring].concat())
        };
        let tbs = {
            let version = tlv(0xa0, &tlv(0x02, &[0x01]));
            let serial = tlv(0x02, &[0x01]);
            let sig_alg = tlv(0x30, &oid(OID_ECDSA_SHA256));
            let name = tlv(0x30, &[]);
            let validity = {
                let nb = tlv(0x17, b"231101000000Z");
                let na = tlv(0x17, b"331101000000Z");
                tlv(0x30, &[nb, na].concat())
            };
            let seq = [version, serial, sig_alg, name.clone(), validity, name, spki].concat();
            tlv(0x30, &seq)
        };
        let sig: p256::ecdsa::Signature = signer.sign(&tbs);
        let sig: Vec<u8> = sig.to_der().as_bytes().to_vec();
        let sig_alg = tlv(0x30, &oid(OID_ECDSA_SHA256));
        let bitstring = tlv(0x03, &[&[0x00], &sig[..]].concat());
        tlv(0x30, &[tbs, sig_alg, bitstring].concat())
    }

    #[test]
    fn rekor_entry_must_record_this_signature_certificate_and_payload() {
        // VULN-09: a valid entry copied from another bundle must not count.
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        let env = &resp["attestations"][0]["bundle"]["dsseEnvelope"];
        let sig = env["signatures"][0]["sig"].as_str().unwrap();
        let payload = decode_b64(env["payload"].as_str().unwrap(), "payload").unwrap();
        let entry = fixture_tlog_entry();
        let leaf = fixture_leaf_der();
        assert!(entry_binds(&entry, &leaf, sig, &payload));
        assert!(!entry_binds(&entry, &leaf, sig, b"another payload"));
        let other_sig = base64::engine::general_purpose::STANDARD.encode(b"other");
        assert!(!entry_binds(&entry, &leaf, &other_sig, &payload));
        let mut other_leaf = leaf.clone();
        let n = other_leaf.len();
        other_leaf[n - 1] ^= 1;
        assert!(!entry_binds(&entry, &other_leaf, sig, &payload));
    }

    #[test]
    fn a_leaf_cannot_act_as_a_ca() {
        // A Fulcio leaf is not a CA and carries the code-signing EKU; the
        // pinned intermediate is a CA.
        let leaf_der = fixture_leaf_der();
        let leaf = parse_cert(&leaf_der).unwrap();
        assert!(!leaf.is_ca());
        assert!(leaf.has_code_signing_eku());
        let inter = decode_b64(FULCIO_INTERMEDIATE_V1_DER_B64, "i").unwrap();
        assert!(parse_cert(&inter).unwrap().is_ca());
        // Offering the leaf itself as the issuer of another certificate
        // gets nowhere: it is skipped as a non-CA.
        assert!(
            verify_fulcio_chain(&leaf_der, std::slice::from_ref(&leaf_der), 1728922425).is_ok()
        );
    }

    #[test]
    fn repository_references_normalize_alike() {
        for r in [
            "git+https://github.com/sigstore/sigstore-js.git",
            "https://github.com/sigstore/sigstore-js",
            "git@github.com:sigstore/sigstore-js.git",
            "github:sigstore/sigstore-js",
            "sigstore/sigstore-js",
            "git://github.com/Sigstore/sigstore-js.git/",
        ] {
            assert_eq!(
                normalize_repository(r),
                "https://github.com/sigstore/sigstore-js",
                "{r}"
            );
        }
    }

    #[test]
    fn signer_pins_are_trust_on_first_use() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        check_signer_pin(root, "a", None).unwrap();
        check_signer_pin(root, "a", Some("gh https://github.com/o/a")).unwrap();
        check_signer_pin(root, "a", Some("gh https://github.com/o/a")).unwrap();
        assert!(check_signer_pin(root, "a", Some("gh https://github.com/evil/a")).is_err());
        assert!(
            check_signer_pin(root, "a", None).is_err(),
            "downgrade to no provenance"
        );
        check_signer_pin(root, "b", Some("gh https://github.com/o/b")).unwrap();
    }
}
