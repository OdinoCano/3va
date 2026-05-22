//! Post-quantum-safe cryptography for 3va.
//!
//! ## What IS implemented (and genuinely quantum-resistant)
//!
//! | API                  | Algorithm          | PQ-safe? | Notes                          |
//! |----------------------|--------------------|----------|--------------------------------|
//! | `LamportKeypair`     | Lamport OTS        | ✓        | Hash-based (SHA-256), one-time |
//! | `hkdf_expand`        | HKDF-SHA256        | ✓        | Key derivation                 |
//!
//! ## What is NOT yet implemented
//!
//! The NIST-standardized post-quantum algorithms (ML-KEM / FIPS 203 and
//! ML-DSA / FIPS 204) require the `ml-kem` and `ml-dsa` crates from the
//! RustCrypto project.  Those crates are **not bundled** with this workspace.
//! Calling [`Algorithm::MlKem768`] or [`Algorithm::MlDsa65`] returns
//! [`CryptoError::NotAvailable`] with a remediation message.
//!
//! Add them to your `Cargo.toml` when they become available in this workspace:
//! ```toml
//! ml-kem = "0.3"   # FIPS 203 – key encapsulation
//! ml-dsa = "0.1"   # FIPS 204 – digital signatures
//! ```

pub mod hkdf;
pub mod lamport;

pub use hkdf::hkdf_expand;
pub use lamport::{LamportKeypair, LamportPublicKey, LamportSignature};

use thiserror::Error;

/// Cryptographic algorithms exposed by this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Algorithm {
    /// Lamport one-time signature scheme (SHA-256).
    /// Genuinely quantum-resistant; security reduces to SHA-256 preimage hardness.
    /// **Limitation**: each key pair must sign at most one message.
    LamportSha256,

    /// ML-KEM-768 key encapsulation (FIPS 203 / Kyber).
    /// **Status**: not available — requires the `ml-kem` crate.
    MlKem768,

    /// ML-DSA-65 digital signature (FIPS 204 / Dilithium).
    /// **Status**: not available — requires the `ml-dsa` crate.
    MlDsa65,

    /// SLH-DSA / SPHINCS⁺ (FIPS 205) hash-based signatures.
    /// **Status**: not available — requires the `slh-dsa` crate.
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

/// Canonical error message shown when a NIST PQC algorithm is requested but
/// the required crate has not been compiled in.
pub const NOT_AVAILABLE_MSG: &str = "ML-KEM and ML-DSA require the `ml-kem`/`ml-dsa` RustCrypto crates. \
     Add them to your Cargo.toml: ml-kem = \"0.3\", ml-dsa = \"0.1\"";
