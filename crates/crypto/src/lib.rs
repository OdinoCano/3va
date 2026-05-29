//! Post-quantum cryptography — ML-KEM-768, ML-DSA-65, Lamport one-time signatures, and HKDF.
//!
//! # Examples
//!
//! ## ML-KEM-768 key encapsulation
//!
//! ```
//! use vvva_crypto::kem::MlKemKeypair;
//! use vvva_crypto::{encapsulate, decapsulate};
//!
//! let kp = MlKemKeypair::generate();
//! let (ct, ss_enc) = encapsulate(&kp.ek);
//! let ss_dec = decapsulate(&kp.dk, &ct);
//! assert_eq!(ss_enc.0, ss_dec.0, "shared secrets must match");
//! ```
//!
//! ## ML-DSA-65 signatures
//!
//! ```
//! use vvva_crypto::{generate_keypair_hex, signing_key_from_hex, verifying_key_from_hex, sign, verify};
//!
//! let (sk_hex, vk_hex) = generate_keypair_hex();
//! let sk = signing_key_from_hex(&sk_hex).unwrap();
//! let vk = verifying_key_from_hex(&vk_hex).unwrap();
//!
//! let msg = b"hello 3va";
//! let sig = sign(&sk, msg);
//! assert!(verify(&vk, msg, &sig).is_ok());
//! assert!(verify(&vk, b"wrong", &sig).is_err());
//! ```

pub mod dsa;
pub mod hkdf;
pub mod kem;
pub mod lamport;

pub use dsa::{
    generate_keypair_hex, generate_signing_key, sign, signing_key_from_hex, signing_key_to_hex,
    verify, verifying_key_from_hex, verifying_key_to_hex,
};
pub use hkdf::hkdf_expand;
pub use kem::{
    MlKemCiphertext, MlKemKeypair, MlKemSharedSecret, decapsulate, decapsulation_key_from_hex,
    encapsulate, encapsulation_key_from_hex,
};
pub use lamport::{LamportKeypair, LamportPublicKey, LamportSignature};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Algorithm {
    LamportSha256,
    MlKem768,
    MlDsa65,
}

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("algorithm not available: {0}")]
    NotAvailable(&'static str),

    #[error("invalid key material: {0}")]
    InvalidKey(String),

    #[error("signature verification failed")]
    VerificationFailed,

    #[error("serialisation error: {0}")]
    Serialisation(String),
}
