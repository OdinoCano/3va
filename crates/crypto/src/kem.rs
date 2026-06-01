//! ML-KEM-768 Key Encapsulation Mechanism (FIPS 203).
//!
//! ML-KEM (formerly Kyber) is the NIST-standardised post-quantum KEM.  The
//! 768 parameter set targets NIST security level 3 (roughly 178-bit classical /
//! 107-bit quantum).
//!
//! ## Sizes (ML-KEM-768)
//! | Artifact               | Size     |
//! |------------------------|----------|
//! | Encapsulation key      | 1 184 B  |
//! | Decapsulation key seed |    64 B  |
//! | Ciphertext             | 1 088 B  |
//! | Shared secret          |    32 B  |
//!
//! The decapsulation key is stored as its 64-byte seed; the full 2 400-byte
//! expanded form is derived on load.
//!
//! ## Wire format
//! All byte arrays are hex-encoded for easy transport.

use ml_kem::{
    Ciphertext, Decapsulate, DecapsulationKey, Encapsulate, EncapsulationKey, Kem, KeyExport,
    MlKem768, array::Array, kem::Key,
};

use crate::CryptoError;

/// An ML-KEM-768 key pair (decapsulation key + encapsulation key).
pub struct MlKemKeypair {
    pub dk: DecapsulationKey<MlKem768>,
    pub ek: EncapsulationKey<MlKem768>,
}

/// A 32-byte ML-KEM shared secret.
pub struct MlKemSharedSecret(pub [u8; 32]);

/// An ML-KEM-768 ciphertext (1 088 bytes).
pub struct MlKemCiphertext(pub Ciphertext<MlKem768>);

// ── Key generation ────────────────────────────────────────────────────────────

impl MlKemKeypair {
    /// Generate a new key pair using the system CSPRNG.
    pub fn generate() -> Self {
        let (dk, ek) = MlKem768::generate_keypair();
        MlKemKeypair { dk, ek }
    }

    // ── Serialisation ─────────────────────────────────────────────────────────

    /// Return the raw encapsulation key bytes (1 184 B for ML-KEM-768).
    pub fn encapsulation_key_bytes(&self) -> Vec<u8> {
        self.ek.to_bytes().as_slice().to_vec()
    }

    /// Hex-encode the encapsulation (public) key.
    pub fn encapsulation_key_hex(&self) -> String {
        hex::encode(self.ek.to_bytes().as_slice())
    }

    /// Hex-encode the decapsulation (private) key seed (64 bytes).
    pub fn decapsulation_key_hex(&self) -> String {
        hex::encode(self.dk.to_bytes().as_slice())
    }
}

// ── Encapsulation ─────────────────────────────────────────────────────────────

impl MlKemCiphertext {
    /// Hex-encode the ciphertext.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0.as_slice())
    }

    /// Decode a ciphertext from raw bytes (1 088 B for ML-KEM-768).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let arr: Ciphertext<MlKem768> = Array::try_from(bytes).map_err(|_| {
            CryptoError::InvalidKey(format!(
                "ciphertext: expected 1088 bytes, got {}",
                bytes.len()
            ))
        })?;
        Ok(MlKemCiphertext(arr))
    }

    /// Decode a ciphertext from hex.
    pub fn from_hex(s: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(s).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
        Self::from_bytes(&bytes)
    }
}

/// Encapsulate a fresh shared secret to `ek`.
///
/// Returns `(ciphertext, shared_secret)`.  Send the ciphertext to the holder
/// of the matching decapsulation key; both sides then hold the same 32-byte
/// secret.
pub fn encapsulate(ek: &EncapsulationKey<MlKem768>) -> (MlKemCiphertext, MlKemSharedSecret) {
    let (ct, ss) = ek.encapsulate();
    let mut secret = [0u8; 32];
    secret.copy_from_slice(ss.as_slice());
    (MlKemCiphertext(ct), MlKemSharedSecret(secret))
}

/// Decapsulate `ct` using `dk` to recover the shared secret.
pub fn decapsulate(dk: &DecapsulationKey<MlKem768>, ct: &MlKemCiphertext) -> MlKemSharedSecret {
    let ss = dk.decapsulate(&ct.0);
    let mut secret = [0u8; 32];
    secret.copy_from_slice(ss.as_slice());
    MlKemSharedSecret(secret)
}

// ── Deserialisation helpers for stand-alone keys ──────────────────────────────

/// Decode an encapsulation key from hex.
pub fn encapsulation_key_from_hex(s: &str) -> Result<EncapsulationKey<MlKem768>, CryptoError> {
    let bytes = hex::decode(s).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    let arr: Key<EncapsulationKey<MlKem768>> = Array::try_from(bytes.as_slice()).map_err(|_| {
        CryptoError::InvalidKey(format!(
            "encapsulation key: expected 1184 bytes, got {}",
            bytes.len()
        ))
    })?;
    EncapsulationKey::<MlKem768>::new(&arr)
        .map_err(|_| CryptoError::InvalidKey("invalid encapsulation key encoding".into()))
}

/// Decode a decapsulation key from its 64-byte seed hex.
pub fn decapsulation_key_from_hex(s: &str) -> Result<DecapsulationKey<MlKem768>, CryptoError> {
    let bytes = hex::decode(s).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    let seed: Key<DecapsulationKey<MlKem768>> =
        Array::try_from(bytes.as_slice()).map_err(|_| {
            CryptoError::InvalidKey(format!(
                "decapsulation key: expected 64 bytes, got {}",
                bytes.len()
            ))
        })?;
    Ok(DecapsulationKey::<MlKem768>::from_seed(seed))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keygen_encap_decap_round_trip() {
        let kp = MlKemKeypair::generate();
        let (ct, ss_send) = encapsulate(&kp.ek);
        let ss_recv = decapsulate(&kp.dk, &ct);
        assert_eq!(ss_send.0, ss_recv.0, "shared secrets must match");
    }

    #[test]
    fn different_keypairs_produce_different_secrets() {
        let kp1 = MlKemKeypair::generate();
        let kp2 = MlKemKeypair::generate();
        let (ct, ss1) = encapsulate(&kp1.ek);
        // Decapsulating with the wrong key gives a different implicit-reject secret.
        let ss_wrong = decapsulate(&kp2.dk, &ct);
        assert_ne!(ss1.0, ss_wrong.0);
    }

    #[test]
    fn ciphertext_hex_round_trip() {
        let kp = MlKemKeypair::generate();
        let (ct, ss_send) = encapsulate(&kp.ek);
        let hex = ct.to_hex();
        let ct2 = MlKemCiphertext::from_hex(&hex).expect("decode must succeed");
        let ss_recv = decapsulate(&kp.dk, &ct2);
        assert_eq!(ss_send.0, ss_recv.0);
    }

    #[test]
    fn encapsulation_key_hex_round_trip() {
        let kp = MlKemKeypair::generate();
        let hex = kp.encapsulation_key_hex();
        let ek2 = encapsulation_key_from_hex(&hex).expect("decode must succeed");
        let (ct, ss_send) = encapsulate(&ek2);
        let ss_recv = decapsulate(&kp.dk, &ct);
        assert_eq!(ss_send.0, ss_recv.0);
    }

    #[test]
    fn decapsulation_key_hex_round_trip() {
        let kp = MlKemKeypair::generate();
        let dk_hex = kp.decapsulation_key_hex();
        let (ct, ss_send) = encapsulate(&kp.ek);
        let dk2 = decapsulation_key_from_hex(&dk_hex).expect("decode must succeed");
        let ss_recv = decapsulate(&dk2, &ct);
        assert_eq!(ss_send.0, ss_recv.0);
    }

    #[test]
    fn key_sizes_are_correct() {
        let kp = MlKemKeypair::generate();
        assert_eq!(hex::decode(kp.encapsulation_key_hex()).unwrap().len(), 1184);
        // to_bytes() returns the 64-byte seed
        assert_eq!(hex::decode(kp.decapsulation_key_hex()).unwrap().len(), 64);
    }

    #[test]
    fn ciphertext_size_is_correct() {
        let kp = MlKemKeypair::generate();
        let (ct, _) = encapsulate(&kp.ek);
        assert_eq!(hex::decode(ct.to_hex()).unwrap().len(), 1088);
    }

    #[test]
    fn shared_secret_is_32_bytes() {
        let kp = MlKemKeypair::generate();
        let (_, ss) = encapsulate(&kp.ek);
        assert_eq!(ss.0.len(), 32);
    }

    #[test]
    fn invalid_hex_ciphertext_returns_error() {
        assert!(MlKemCiphertext::from_hex("notvalidhex").is_err());
        assert!(MlKemCiphertext::from_hex("deadbeef").is_err()); // too short
    }

    #[test]
    fn from_bytes_round_trip() {
        let kp = MlKemKeypair::generate();
        let (ct, ss_send) = encapsulate(&kp.ek);
        // from_bytes must produce the same shared secret as decapsulating the
        // original ciphertext — no hex round-trip needed.
        let raw = ct.0.as_slice().to_vec();
        let ct2 = MlKemCiphertext::from_bytes(&raw).expect("from_bytes must succeed on 1088 B");
        let ss_recv = decapsulate(&kp.dk, &ct2);
        assert_eq!(
            ss_send.0, ss_recv.0,
            "shared secret must match via from_bytes"
        );
    }

    #[test]
    fn from_bytes_wrong_length_returns_error() {
        assert!(MlKemCiphertext::from_bytes(&[0u8; 42]).is_err());
        assert!(MlKemCiphertext::from_bytes(&[]).is_err());
    }

    #[test]
    fn from_bytes_and_from_hex_are_equivalent() {
        let kp = MlKemKeypair::generate();
        let (ct, _) = encapsulate(&kp.ek);
        let raw = ct.0.as_slice().to_vec();
        let via_bytes = MlKemCiphertext::from_bytes(&raw).unwrap();
        let via_hex = MlKemCiphertext::from_hex(&hex::encode(&raw)).unwrap();
        // Both paths must produce the same shared secret.
        let ss1 = decapsulate(&kp.dk, &via_bytes);
        let ss2 = decapsulate(&kp.dk, &via_hex);
        assert_eq!(ss1.0, ss2.0);
    }
}
