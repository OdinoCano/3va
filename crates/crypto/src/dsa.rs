//! ML-DSA-65 Digital Signature Algorithm (FIPS 204).
//!
//! ML-DSA (formerly Dilithium) is the NIST-standardised post-quantum signature
//! scheme.  The 65 parameter set targets NIST security level 3 (roughly
//! 178-bit classical / 107-bit quantum).
//!
//! Unlike Lamport OTS, ML-DSA keys can sign an unlimited number of messages.
//!
//! ## Sizes (ML-DSA-65)
//! | Artifact      | Size    |
//! |---------------|---------|
//! | Signing key   |    32 B (seed; expands to 4 032 B internally) |
//! | Verifying key | 1 952 B |
//! | Signature     | 3 309 B |
//!
//! ## Wire format
//! All byte arrays are hex-encoded for easy transport.

use ml_dsa::{
    EncodedVerifyingKey, Generate, KeyExport, MlDsa65, Signature, SignatureEncoding, SigningKey,
    Signer, Verifier, VerifyingKey, common::Key,
};

use crate::CryptoError;

// ── Key generation ────────────────────────────────────────────────────────────

/// Generate a new ML-DSA-65 signing key using the system CSPRNG.
pub fn generate_signing_key() -> SigningKey<MlDsa65> {
    SigningKey::<MlDsa65>::generate()
}

// ── Signing ───────────────────────────────────────────────────────────────────

/// Sign `message` with `sk` and return the raw signature bytes.
pub fn sign(sk: &SigningKey<MlDsa65>, message: &[u8]) -> Vec<u8> {
    let sig: Signature<MlDsa65> = sk.sign(message);
    sig.to_bytes().to_vec()
}

// ── Verification ──────────────────────────────────────────────────────────────

/// Verify a signature over `message` against `vk`.
///
/// Returns `Ok(())` on success, `Err(CryptoError::VerificationFailed)` otherwise.
pub fn verify(
    vk: &VerifyingKey<MlDsa65>,
    message: &[u8],
    sig_bytes: &[u8],
) -> Result<(), CryptoError> {
    let sig = Signature::<MlDsa65>::try_from(sig_bytes)
        .map_err(|_| CryptoError::InvalidKey("invalid signature encoding".into()))?;
    vk.verify(message, &sig)
        .map_err(|_| CryptoError::VerificationFailed)
}

// ── Serialisation helpers ─────────────────────────────────────────────────────

/// Hex-encode a signing key (32-byte seed).
pub fn signing_key_to_hex(sk: &SigningKey<MlDsa65>) -> String {
    hex::encode(sk.to_bytes().as_slice())
}

/// Decode a signing key from its 32-byte seed hex.
pub fn signing_key_from_hex(s: &str) -> Result<SigningKey<MlDsa65>, CryptoError> {
    let bytes = hex::decode(s).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    let arr: Key<SigningKey<MlDsa65>> = bytes
        .as_slice()
        .try_into()
        .map_err(|_| {
            CryptoError::InvalidKey(format!(
                "signing key: expected 32 bytes, got {}",
                bytes.len()
            ))
        })?;
    Ok(SigningKey::<MlDsa65>::from_seed(&arr))
}

/// Hex-encode a verifying key (1 952 bytes).
pub fn verifying_key_to_hex(vk: &VerifyingKey<MlDsa65>) -> String {
    hex::encode(vk.to_bytes().as_slice())
}

/// Decode a verifying key from hex.
pub fn verifying_key_from_hex(s: &str) -> Result<VerifyingKey<MlDsa65>, CryptoError> {
    let bytes = hex::decode(s).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    let arr: EncodedVerifyingKey<MlDsa65> = bytes
        .as_slice()
        .try_into()
        .map_err(|_| {
            CryptoError::InvalidKey(format!(
                "verifying key: expected 1952 bytes, got {}",
                bytes.len()
            ))
        })?;
    Ok(VerifyingKey::<MlDsa65>::decode(&arr))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ml_dsa::Keypair;

    #[test]
    fn sign_and_verify_round_trip() {
        let sk = generate_signing_key();
        let vk = sk.verifying_key().clone();
        let msg = b"hello post-quantum world";
        let sig = sign(&sk, msg);
        verify(&vk, msg, &sig).expect("valid signature must verify");
    }

    #[test]
    fn wrong_message_fails_verification() {
        let sk = generate_signing_key();
        let vk = sk.verifying_key().clone();
        let sig = sign(&sk, b"correct message");
        assert!(verify(&vk, b"wrong message", &sig).is_err());
    }

    #[test]
    fn tampered_signature_fails_verification() {
        let sk = generate_signing_key();
        let vk = sk.verifying_key().clone();
        let mut sig = sign(&sk, b"data");
        sig[0] ^= 0xff;
        assert!(verify(&vk, b"data", &sig).is_err());
    }

    #[test]
    fn signing_key_hex_round_trip() {
        let sk = generate_signing_key();
        let hex = signing_key_to_hex(&sk);
        let sk2 = signing_key_from_hex(&hex).expect("decode must succeed");
        let vk = sk.verifying_key().clone();
        let sig = sign(&sk2, b"round-trip test");
        verify(&vk, b"round-trip test", &sig).expect("decoded key must produce valid signatures");
    }

    #[test]
    fn verifying_key_hex_round_trip() {
        let sk = generate_signing_key();
        let vk = sk.verifying_key().clone();
        let hex = verifying_key_to_hex(&vk);
        let vk2 = verifying_key_from_hex(&hex).expect("decode must succeed");
        let sig = sign(&sk, b"vk round-trip");
        verify(&vk2, b"vk round-trip", &sig).expect("decoded vk must verify");
    }

    #[test]
    fn key_sizes_are_correct() {
        let sk = generate_signing_key();
        let vk = sk.verifying_key().clone();
        let sig = sign(&sk, b"size check");
        // Seed-based signing key is 32 bytes.
        assert_eq!(hex::decode(signing_key_to_hex(&sk)).unwrap().len(), 32);
        assert_eq!(hex::decode(verifying_key_to_hex(&vk)).unwrap().len(), 1952);
        assert_eq!(sig.len(), 3309);
    }

    #[test]
    fn different_keys_cannot_forge_signatures() {
        let sk1 = generate_signing_key();
        let sk2 = generate_signing_key();
        let vk1 = sk1.verifying_key().clone();
        let sig = sign(&sk2, b"message");
        assert!(verify(&vk1, b"message", &sig).is_err());
    }

    #[test]
    fn multiple_signatures_with_same_key() {
        // Unlike Lamport OTS, ML-DSA supports signing many messages.
        let sk = generate_signing_key();
        let vk = sk.verifying_key().clone();
        for i in 0..5u8 {
            let msg = [i; 32];
            let sig = sign(&sk, &msg);
            verify(&vk, &msg, &sig).expect("each signature must verify");
        }
    }
}
