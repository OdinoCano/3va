pub mod dsa;
pub mod hkdf;
pub mod kem;
pub mod lamport;

pub use dsa::{
    generate_signing_key, sign, signing_key_from_hex, signing_key_to_hex, verify,
    verifying_key_from_hex, verifying_key_to_hex,
};
pub use hkdf::hkdf_expand;
pub use kem::{
    MlKemCiphertext, MlKemKeypair, MlKemSharedSecret, decapsulate, encapsulate,
    encapsulation_key_from_hex, decapsulation_key_from_hex,
};
pub use lamport::{LamportKeypair, LamportPublicKey, LamportSignature};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Algorithm {
    LamportSha256,
    MlKem768,
    MlDsa65,
    SlhDsaSha2128s,
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
