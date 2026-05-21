//! Lamport One-Time Signature Scheme using SHA-256.
//!
//! Security model: quantum-resistant, reducing to the preimage resistance of
//! SHA-256.  A quantum computer running Grover's algorithm reduces the effective
//! security from 256 bits to 128 bits — still considered adequate (NIST level 1).
//!
//! **Critical limitation**: each `LamportKeypair` must sign **at most one**
//! message.  Signing two messages with the same key leaks half the secret key
//! and allows forgery.  For multi-message use, generate a new key pair per
//! message or build a Merkle tree (XMSS / LMS) on top.
//!
//! ## Sizes
//! | Artifact     | Size   |
//! |--------------|--------|
//! | Secret key   | 16 KiB |
//! | Public key   | 16 KiB |
//! | Signature    |  8 KiB |
//!
//! ## Wire format
//! All artifacts are hex-encoded for easy transport.

use rand::RngCore;
use rand_chacha::ChaCha20Rng;
use rand::SeedableRng;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::CryptoError;

/// Number of message bits signed per key pair (= SHA-256 output in bits).
const N: usize = 256;

/// Length of each hash-chain value in bytes (= SHA-256 output in bytes).
const HASH_LEN: usize = 32;

/// Total number of secret/public key values: 2 per message bit (one for 0, one for 1).
const SK_COUNT: usize = 2 * N; // 512

/// Secret key: 512 random 32-byte values, zeroised on drop.
pub struct LamportKeypair {
    sk: Box<[[u8; HASH_LEN]; SK_COUNT]>,
    pub public_key: LamportPublicKey,
    used: bool,
}

/// Public key: SHA-256 of each secret key value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LamportPublicKey {
    data: Box<[[u8; HASH_LEN]; SK_COUNT]>,
}

/// A Lamport signature: the revealed half of the secret key.
#[derive(Debug, Clone)]
pub struct LamportSignature {
    /// 256 revealed secret-key values (one per bit of the message hash).
    revealed: Box<[[u8; HASH_LEN]; N]>,
}

// ── Key generation ────────────────────────────────────────────────────────────

impl LamportKeypair {
    /// Generate a new key pair using a cryptographically secure RNG.
    pub fn generate() -> Self {
        let mut rng = ChaCha20Rng::from_entropy();
        Self::generate_with_rng(&mut rng)
    }

    /// Generate with a caller-supplied RNG (useful for deterministic tests).
    pub fn generate_with_rng(rng: &mut impl RngCore) -> Self {
        // Allocate on the heap to avoid stack overflow (16 KiB would be tight).
        let mut sk = Box::new([[0u8; HASH_LEN]; SK_COUNT]);
        for slot in sk.iter_mut() {
            rng.fill_bytes(slot);
        }

        let mut pk_data = Box::new([[0u8; HASH_LEN]; SK_COUNT]);
        for (i, sk_val) in sk.iter().enumerate() {
            pk_data[i] = Sha256::digest(sk_val).into();
        }

        let public_key = LamportPublicKey { data: pk_data };
        LamportKeypair { sk, public_key, used: false }
    }

    // ── Signing ───────────────────────────────────────────────────────────────

    /// Sign `message` and return a signature.
    ///
    /// Returns `Err(CryptoError::InvalidKey)` if this key pair has already been
    /// used, enforcing the one-time property.
    pub fn sign(&mut self, message: &[u8]) -> Result<LamportSignature, CryptoError> {
        if self.used {
            return Err(CryptoError::InvalidKey(
                "Lamport key pair already used. Generate a new key pair for each message."
                    .to_string(),
            ));
        }
        self.used = true;

        let msg_hash: [u8; HASH_LEN] = Sha256::digest(message).into();
        let mut revealed = Box::new([[0u8; HASH_LEN]; N]);

        for (bit_idx, slot) in revealed.iter_mut().enumerate() {
            let byte = msg_hash[bit_idx / 8];
            let bit = (byte >> (7 - (bit_idx % 8))) & 1;
            // For bit == 0, reveal sk[2*i]; for bit == 1, reveal sk[2*i + 1].
            *slot = self.sk[2 * bit_idx + bit as usize];
        }

        Ok(LamportSignature { revealed })
    }

    // ── Serialisation ─────────────────────────────────────────────────────────

    /// Hex-encode the public key for transport.
    pub fn public_key_hex(&self) -> String {
        let bytes: Vec<u8> = self.public_key.data.iter().flatten().copied().collect();
        hex::encode(bytes)
    }
}

impl Drop for LamportKeypair {
    fn drop(&mut self) {
        // Zeroize the secret key material on drop.
        for slot in self.sk.iter_mut() {
            slot.zeroize();
        }
    }
}

// ── Verification ─────────────────────────────────────────────────────────────

impl LamportPublicKey {
    /// Verify `signature` over `message` against this public key.
    pub fn verify(&self, message: &[u8], sig: &LamportSignature) -> Result<(), CryptoError> {
        let msg_hash: [u8; HASH_LEN] = Sha256::digest(message).into();

        for (bit_idx, revealed_val) in sig.revealed.iter().enumerate() {
            let byte = msg_hash[bit_idx / 8];
            let bit = (byte >> (7 - (bit_idx % 8))) & 1;
            let expected_pk = self.data[2 * bit_idx + bit as usize];
            let got: [u8; HASH_LEN] = Sha256::digest(revealed_val).into();
            if got != expected_pk {
                return Err(CryptoError::VerificationFailed);
            }
        }
        Ok(())
    }

    /// Decode a public key from hex.
    pub fn from_hex(s: &str) -> Result<Self, CryptoError> {
        let bytes =
            hex::decode(s).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
        let expected = SK_COUNT * HASH_LEN;
        if bytes.len() != expected {
            return Err(CryptoError::InvalidKey(format!(
                "expected {expected} bytes, got {}",
                bytes.len()
            )));
        }
        let mut data = Box::new([[0u8; HASH_LEN]; SK_COUNT]);
        for (i, chunk) in bytes.chunks(HASH_LEN).enumerate() {
            data[i].copy_from_slice(chunk);
        }
        Ok(LamportPublicKey { data })
    }
}

// ── Signature serialisation ───────────────────────────────────────────────────

impl LamportSignature {
    /// Hex-encode the signature for transport.
    pub fn to_hex(&self) -> String {
        let bytes: Vec<u8> = self.revealed.iter().flatten().copied().collect();
        hex::encode(bytes)
    }

    /// Decode a signature from hex.
    pub fn from_hex(s: &str) -> Result<Self, CryptoError> {
        let bytes =
            hex::decode(s).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
        let expected = N * HASH_LEN;
        if bytes.len() != expected {
            return Err(CryptoError::InvalidKey(format!(
                "expected {expected} bytes, got {}",
                bytes.len()
            )));
        }
        let mut revealed = Box::new([[0u8; HASH_LEN]; N]);
        for (i, chunk) in bytes.chunks(HASH_LEN).enumerate() {
            revealed[i].copy_from_slice(chunk);
        }
        Ok(LamportSignature { revealed })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha20Rng;

    fn det_rng() -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(0xdeadbeef)
    }

    #[test]
    fn keygen_produces_valid_public_key() {
        let kp = LamportKeypair::generate_with_rng(&mut det_rng());
        // Public key must be SK_COUNT * HASH_LEN = 16384 bytes.
        assert_eq!(kp.public_key.data.len(), SK_COUNT);
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let mut kp = LamportKeypair::generate_with_rng(&mut det_rng());
        let pk = kp.public_key.clone();
        let msg = b"hello post-quantum world";
        let sig = kp.sign(msg).expect("first sign must succeed");
        pk.verify(msg, &sig).expect("valid signature must verify");
    }

    #[test]
    fn wrong_message_fails_verification() {
        let mut kp = LamportKeypair::generate_with_rng(&mut det_rng());
        let pk = kp.public_key.clone();
        let sig = kp.sign(b"correct message").unwrap();
        let result = pk.verify(b"wrong message", &sig);
        assert!(result.is_err(), "wrong message must fail verification");
    }

    #[test]
    fn tampered_signature_fails_verification() {
        let mut kp = LamportKeypair::generate_with_rng(&mut det_rng());
        let pk = kp.public_key.clone();
        let mut sig = kp.sign(b"data").unwrap();
        sig.revealed[0][0] ^= 0xff; // flip some bits
        assert!(pk.verify(b"data", &sig).is_err());
    }

    #[test]
    fn signing_twice_with_same_keypair_is_rejected() {
        let mut kp = LamportKeypair::generate_with_rng(&mut det_rng());
        let _ = kp.sign(b"first").unwrap();
        let result = kp.sign(b"second");
        assert!(
            matches!(result, Err(CryptoError::InvalidKey(_))),
            "one-time constraint must be enforced"
        );
    }

    #[test]
    fn public_key_hex_round_trip() {
        let kp = LamportKeypair::generate_with_rng(&mut det_rng());
        let hex = kp.public_key_hex();
        let pk2 = LamportPublicKey::from_hex(&hex).expect("decode must succeed");
        assert_eq!(kp.public_key, pk2);
    }

    #[test]
    fn signature_hex_round_trip() {
        let mut kp = LamportKeypair::generate_with_rng(&mut det_rng());
        let sig = kp.sign(b"round-trip test").unwrap();
        let hex = sig.to_hex();
        let sig2 = LamportSignature::from_hex(&hex).expect("decode must succeed");
        // Verify the decoded signature works.
        kp.public_key.verify(b"round-trip test", &sig2).expect("decoded sig must verify");
    }

    #[test]
    fn invalid_hex_pk_returns_error() {
        assert!(LamportPublicKey::from_hex("notvalidhex").is_err());
        assert!(LamportPublicKey::from_hex("deadbeef").is_err()); // too short
    }

    #[test]
    fn different_messages_produce_different_signatures() {
        let mut kp1 = LamportKeypair::generate_with_rng(&mut ChaCha20Rng::seed_from_u64(1));
        let mut kp2 = LamportKeypair::generate_with_rng(&mut ChaCha20Rng::seed_from_u64(1));
        let s1 = kp1.sign(b"msg A").unwrap();
        let s2 = kp2.sign(b"msg B").unwrap();
        assert_ne!(s1.to_hex(), s2.to_hex());
    }

    /// Document the algorithm's known limitation.
    #[test]
    fn lamport_is_one_time_and_not_ml_dsa() {
        // Lamport OTS is quantum-resistant but NOT the NIST ML-DSA standard.
        // This test serves as a compile-time reminder.
        let mut kp = LamportKeypair::generate_with_rng(&mut det_rng());
        let sig = kp.sign(b"test").unwrap();
        assert_eq!(sig.to_hex().len(), N * HASH_LEN * 2); // hex = 2 chars/byte
    }
}
