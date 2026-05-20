use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HashAlgorithm {
    SHA256,
    SHA512,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureInfo {
    pub algorithm: HashAlgorithm,
    pub hash: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationStatus {
    Verified,
    Unverified,
    Mismatch,
    Missing,
    Failed(String),
}

pub struct SignatureVerifier {
    algorithm: HashAlgorithm,
}

impl SignatureVerifier {
    pub fn new(algorithm: HashAlgorithm) -> Self {
        Self { algorithm }
    }

    pub fn sha256() -> Self {
        Self {
            algorithm: HashAlgorithm::SHA256,
        }
    }

    pub fn sha512() -> Self {
        Self {
            algorithm: HashAlgorithm::SHA512,
        }
    }

    pub fn compute_hash(&self, path: &Path) -> Result<String, std::io::Error> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        match self.algorithm {
            HashAlgorithm::SHA256 => {
                let mut hasher = Sha256::new();
                let mut buffer = [0u8; 8192];
                loop {
                    let bytes_read = reader.read(&mut buffer)?;
                    if bytes_read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..bytes_read]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            }
            HashAlgorithm::SHA512 => {
                let mut hasher = Sha512::new();
                let mut buffer = [0u8; 8192];
                loop {
                    let bytes_read = reader.read(&mut buffer)?;
                    if bytes_read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..bytes_read]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            }
        }
    }

    pub fn verify(&self, path: &Path, expected_hash: &str) -> VerificationStatus {
        match self.compute_hash(path) {
            Ok(computed) => {
                if computed.to_lowercase() == expected_hash.to_lowercase() {
                    VerificationStatus::Verified
                } else {
                    VerificationStatus::Mismatch
                }
            }
            Err(e) => VerificationStatus::Failed(e.to_string()),
        }
    }

    /// Verify a tarball's bytes against an SRI integrity string from a registry
    /// (`sha512-<base64>` or `sha256-<base64>`).  Returns `Missing` when no
    /// integrity string is provided (caller should warn but not fail hard).
    pub fn verify_tarball(&self, data: &[u8], integrity: &str) -> VerificationStatus {
        let b64 = base64::engine::general_purpose::STANDARD;
        if let Some(expected) = integrity.strip_prefix("sha512-") {
            let mut h = Sha512::new();
            h.update(data);
            let computed = b64.encode(h.finalize());
            return if computed == expected {
                VerificationStatus::Verified
            } else {
                VerificationStatus::Mismatch
            };
        }
        if let Some(expected) = integrity.strip_prefix("sha256-") {
            let mut h = Sha256::new();
            h.update(data);
            let computed = b64.encode(h.finalize());
            return if computed == expected {
                VerificationStatus::Verified
            } else {
                VerificationStatus::Mismatch
            };
        }
        // Unknown algorithm — cannot verify
        VerificationStatus::Missing
    }

    /// Verify a tarball against the optional integrity string supplied by the
    /// registry metadata.  This is the main entry point used by the install flow.
    pub fn verify_from_registry(
        &self,
        tarball_data: &[u8],
        integrity: Option<&str>,
    ) -> VerificationStatus {
        match integrity {
            Some(hash) => self.verify_tarball(tarball_data, hash),
            None => VerificationStatus::Missing,
        }
    }

    pub fn compute_file_hashes(dir: &Path) -> Vec<(String, String)> {
        let mut hashes = Vec::new();

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let verifier = Self::sha256();
                    if let Ok(hash) = verifier.compute_hash(&path)
                        && let Some(name) = path.file_name().and_then(|n| n.to_str())
                    {
                        hashes.push((name.to_string(), hash));
                    }
                }
            }
        }

        hashes
    }
}

impl Default for SignatureVerifier {
    fn default() -> Self {
        Self::sha256()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_hash_computation() {
        let verifier = SignatureVerifier::sha256();
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file).unwrap();
        f.write_all(b"hello world").unwrap();

        let hash = verifier.compute_hash(&file).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_hash_verification() {
        let verifier = SignatureVerifier::sha256();
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file).unwrap();
        f.write_all(b"hello world").unwrap();

        let result = verifier.verify(
            &file,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
        );
        assert!(matches!(result, VerificationStatus::Verified));

        let bad_result = verifier.verify(&file, "wronghash");
        assert!(matches!(bad_result, VerificationStatus::Mismatch));
    }
}
