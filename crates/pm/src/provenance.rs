//! npm provenance / Sigstore attestation verification.
//!
//! Downloads a package's attestations from the registry provenance endpoint
//! (`/-/npm/v1/attestations/{pkg}@{version}`) and verifies the Sigstore
//! bundle: the DSSE envelope signature is checked against the ECDSA public
//! key embedded in the bundle's X.509 certificate, and the in-toto statement
//! must name exactly `{pkg}@{version}` as its subject.
//!
//! Scope notes (kept honest):
//! - A missing attestation is "no provenance", not an error — unless the
//!   caller requires provenance (`--require-provenance` /
//!   `_3VA_REQUIRE_PROVENANCE=1`).
//! - An attestation present but invalid (tampered signature, wrong subject,
//!   malformed envelope) IS a hard error.
//! - Not yet verified: chaining the leaf certificate to the Fulcio root and
//!   Rekor log inclusion proofs (tlog entries are checked for presence only).

use anyhow::Context;
use base64::Engine;
use serde_json::Value;

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
        let spki_parts = children(elem.content);
        if spki_parts.len() != 2 {
            continue;
        }
        let alg = &spki_parts[0];
        let bitstring = &spki_parts[1];
        if alg.tag != 0x30 || bitstring.tag != 0x03 || bitstring.content.first() != Some(&0x00) {
            continue;
        }
        let alg_parts = children(alg.content);
        let (Some(oid_tlv), curve_oid) = (alg_parts.first(), alg_parts.get(1)) else {
            continue;
        };
        if oid_tlv.content != OID_EC_PUBLIC_KEY {
            continue;
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
        return Err("unsupported EC curve in certificate".into());
    }
    Err("no EC SubjectPublicKeyInfo found in certificate".into())
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

/// Outcome of [`verify_attestations`] on a response with no hard failure.
#[derive(Debug)]
pub enum VerifyOutcome {
    /// `n` attestations were cryptographically checked and matched.
    Verified(usize),
    /// The bundle uses a verification-material format this verifier doesn't
    /// support yet (e.g. a bare `publicKey` hint instead of an X.509 leaf
    /// certificate) — not evidence of tampering, just nothing we can check.
    /// Treated the same as "no provenance published" by the caller.
    Unsupported(String),
}

/// Verify every attestation in a registry provenance response.
///
/// Returns [`VerifyOutcome::Verified`] with the number of cryptographically
/// checked bundles, or [`VerifyOutcome::Unsupported`] when the bundle's
/// format can't be checked at all. Any malformed, mismatched, or tampered
/// attestation — i.e. anything actually indicating a problem rather than
/// just "unverifiable" — is a hard error (`Err`).
pub fn verify_attestations(
    response: &Value,
    pkg_name: &str,
    version: &str,
) -> Result<VerifyOutcome, String> {
    let attestations = response["attestations"]
        .as_array()
        .ok_or("provenance response missing 'attestations' array")?;
    if attestations.is_empty() {
        return Err("provenance response contains zero attestations".into());
    }
    let mut verified = 0;
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
        let cert_b64 = vm["x509CertificateChain"]["certificates"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["rawBytes"].as_str());
        let key = match cert_b64 {
            Some(b64) => {
                let der = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| format!("certificate rawBytes invalid base64: {e}"))?;
                extract_ec_public_key(&der)?
            }
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

        let sigs = envelope["signatures"]
            .as_array()
            .ok_or("dsseEnvelope has no signatures array")?;
        if sigs.is_empty() {
            return Err("dsseEnvelope signatures array empty".into());
        }
        let mut any_ok = false;
        for sig in sigs {
            let sig_b64 = sig["sig"].as_str().ok_or("signature missing 'sig'")?;
            let sig_der = base64::engine::general_purpose::STANDARD
                .decode(sig_b64)
                .map_err(|e| format!("signature is not valid base64: {e}"))?;
            if verify_dsse_signature(&key, payload_type, &payload, &sig_der).is_ok() {
                any_ok = true;
                break;
            }
        }
        if !any_ok {
            return Err(format!(
                "DSSE signature verification failed for {pkg_name}@{version} \
                 (predicate {predicate_type})"
            ));
        }
        // Transparency-log presence check (full inclusion proof is future work).
        if vm["tlogEntries"].as_array().is_none_or(Vec::is_empty) {
            return Err("bundle has no Rekor transparency log entries".into());
        }
        verified += 1;
    }
    Ok(VerifyOutcome::Verified(verified))
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

    #[test]
    fn real_npm_provenance_fixture_verifies() {
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        match verify_attestations(&resp, "sigstore", "3.0.0")
            .expect("real npm provenance bundle must verify")
        {
            VerifyOutcome::Verified(n) => {
                assert_eq!(n, 1, "fixture holds one SLSA provenance attestation")
            }
            VerifyOutcome::Unsupported(why) => panic!("expected Verified, got Unsupported: {why}"),
        }
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
        let err = verify_attestations(&resp, "sigstore", "3.0.0")
            .expect_err("tampered attestation must be rejected");
        assert!(
            err.contains("signature verification failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn wrong_subject_fails_even_with_valid_signature() {
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();
        let err = verify_attestations(&resp, "evil-pkg", "3.0.0")
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
        assert!(verify_attestations(&resp, "a", "1.0.0").is_err());
    }
}
