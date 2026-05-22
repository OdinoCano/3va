//! HKDF-SHA256 key derivation.
//!
//! HKDF (RFC 5869) is a hash-based key derivation function.  Because it relies
//! only on HMAC-SHA256, it is **post-quantum safe** at 128-bit security against
//! Grover's algorithm — the same security level as SHA-256 itself.
//!
//! This is the building block for deriving symmetric keys that remain secure
//! after a large-scale quantum computer is available.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// HKDF-Extract: combine `ikm` (input key material) and `salt` to produce a
/// pseudorandom key (PRK) suitable for HKDF-Expand.
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    let effective_salt = if salt.is_empty() {
        &[0u8; 32][..]
    } else {
        salt
    };
    let mut mac = HmacSha256::new_from_slice(effective_salt).expect("HMAC accepts any key length");
    mac.update(ikm);
    mac.finalize().into_bytes().into()
}

/// HKDF-Expand: derive `length` bytes of keying material from a PRK.
///
/// `info` is a context/application-specific binding string.
/// `length` must be ≤ 255 × 32 = 8160 bytes.
///
/// # Panics
/// Panics if `length` > 8160 (HKDF specification limit).
pub fn hkdf_expand(prk: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    assert!(
        length <= 255 * 32,
        "HKDF-Expand: requested length {length} exceeds maximum 8160 bytes"
    );

    let n = length.div_ceil(32); // number of blocks needed
    let mut okm = Vec::with_capacity(n * 32);
    let mut t_prev: Vec<u8> = Vec::new();

    for counter in 1u8..=(n as u8) {
        let mut mac = HmacSha256::new_from_slice(prk).expect("HMAC accepts any key length");
        mac.update(&t_prev);
        mac.update(info);
        mac.update(&[counter]);
        t_prev = mac.finalize().into_bytes().to_vec();
        okm.extend_from_slice(&t_prev);
    }

    okm.truncate(length);
    okm
}

/// Convenience wrapper: extract-then-expand in one call.
pub fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    let prk = hkdf_extract(salt, ikm);
    hkdf_expand(&prk, info, length)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 5869 test vector #1 (SHA-256).
    #[test]
    fn rfc5869_test_vector_1() {
        let ikm = hex::decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b").unwrap();
        let salt = hex::decode("000102030405060708090a0b0c").unwrap();
        let info = hex::decode("f0f1f2f3f4f5f6f7f8f9").unwrap();

        let prk = hkdf_extract(&salt, &ikm);
        let expected_prk = "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5";
        assert_eq!(hex::encode(prk), expected_prk);

        let okm = hkdf_expand(&prk, &info, 42);
        let expected_okm =
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865";
        assert_eq!(hex::encode(&okm), expected_okm);
    }

    /// RFC 5869 test vector #3 (empty salt / info).
    #[test]
    fn rfc5869_test_vector_3() {
        let ikm = hex::decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b").unwrap();
        let prk = hkdf_extract(&[], &ikm);
        let expected_prk = "19ef24a32c717b167f33a91d6f648bdf96596776afdb6377ac434c1c293ccb04";
        assert_eq!(hex::encode(prk), expected_prk);

        let okm = hkdf_expand(&prk, &[], 42);
        let expected_okm =
            "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8";
        assert_eq!(hex::encode(&okm), expected_okm);
    }

    #[test]
    fn hkdf_convenience_produces_non_zero_output() {
        let key = hkdf(b"salt", b"input key material", b"context", 32);
        assert_eq!(key.len(), 32);
        assert_ne!(key, vec![0u8; 32]);
    }

    #[test]
    fn different_info_produces_different_keys() {
        let prk = hkdf_extract(b"salt", b"ikm");
        let k1 = hkdf_expand(&prk, b"context A", 32);
        let k2 = hkdf_expand(&prk, b"context B", 32);
        assert_ne!(k1, k2);
    }

    #[test]
    fn length_is_respected() {
        let prk = hkdf_extract(b"s", b"k");
        for len in [1, 16, 32, 64, 100, 200] {
            assert_eq!(hkdf_expand(&prk, b"info", len).len(), len);
        }
    }

    #[test]
    fn deterministic_output_same_inputs() {
        let a = hkdf(b"s", b"k", b"i", 32);
        let b = hkdf(b"s", b"k", b"i", 32);
        assert_eq!(a, b);
    }
}
