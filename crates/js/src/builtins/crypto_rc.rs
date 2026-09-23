//! Default (non-FIPS) native `crypto` ops on RustCrypto. The `fips` build
//! swaps this module for `crypto_fips.rs`, which has the same signatures.

use super::crypto::{norm_alg, to_pem};
use aes::cipher::{
    BlockDecryptMut, BlockEncryptMut, KeyIvInit, StreamCipher, block_padding::Pkcs7,
};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
use hmac::{Hmac, Mac};
use md5::Md5;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use rand::rngs::OsRng;
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes192CbcEnc = cbc::Encryptor<aes::Aes192>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes192CbcDec = cbc::Decryptor<aes::Aes192>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;
type Aes192Ctr = ctr::Ctr128BE<aes::Aes192>;
type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

pub(super) async fn do_generate_keypair(
    key_type: String,
    options_json: String,
) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || do_generate_keypair_sync_inner(&key_type, &options_json))
        .await?
}

pub(super) fn do_generate_keypair_sync_inner(
    key_type: &str,
    options_json: &str,
) -> anyhow::Result<String> {
    let opts: serde_json::Value =
        serde_json::from_str(options_json).unwrap_or(serde_json::Value::Null);

    match key_type.to_lowercase().as_str() {
        "rsa" | "rsa-pss" => {
            use rsa::{
                RsaPrivateKey,
                pkcs8::{EncodePrivateKey, EncodePublicKey},
            };
            let bits = opts
                .get("modulusLength")
                .and_then(|v| v.as_u64())
                .unwrap_or(2048) as usize;
            // RSA keygen time grows steeply with modulus size — an
            // attacker-controlled (or just careless) script requesting an
            // absurd modulusLength would tie up a blocking-pool thread for
            // an unbounded amount of time/memory. 512 matches the practical
            // floor other runtimes accept; 16384 is far beyond any real
            // use case but still generous.
            if !(512..=16384).contains(&bits) {
                anyhow::bail!("RSA modulusLength must be between 512 and 16384 bits, got {bits}");
            }
            let mut rng = OsRng;
            let private_key = RsaPrivateKey::new(&mut rng, bits)
                .map_err(|e| anyhow::anyhow!("RSA keygen failed: {e}"))?;
            let public_key = private_key.to_public_key();
            let priv_der = private_key
                .to_pkcs8_der()
                .map_err(|e| anyhow::anyhow!("RSA private key encode failed: {e}"))?;
            let pub_der = public_key
                .to_public_key_der()
                .map_err(|e| anyhow::anyhow!("RSA public key encode failed: {e}"))?;
            let priv_pem = to_pem("PRIVATE KEY", priv_der.as_bytes());
            let pub_pem = to_pem("PUBLIC KEY", pub_der.as_bytes());
            Ok(
                serde_json::json!({ "privateKeyPem": priv_pem, "publicKeyPem": pub_pem })
                    .to_string(),
            )
        }
        "ec" => {
            use p256::pkcs8::{EncodePrivateKey, EncodePublicKey};
            let curve = opts
                .get("namedCurve")
                .and_then(|v| v.as_str())
                .unwrap_or("P-256")
                .to_string();
            let mut rng = OsRng;
            match curve.as_str() {
                "P-256" | "prime256v1" | "secp256r1" => {
                    let sk = p256::SecretKey::random(&mut rng);
                    let pk = sk.public_key();
                    let priv_der = sk
                        .to_pkcs8_der()
                        .map_err(|e| anyhow::anyhow!("EC P-256 private key encode failed: {e}"))?;
                    let pub_der = pk
                        .to_public_key_der()
                        .map_err(|e| anyhow::anyhow!("EC P-256 public key encode failed: {e}"))?;
                    let priv_pem = to_pem("PRIVATE KEY", priv_der.as_bytes());
                    let pub_pem = to_pem("PUBLIC KEY", pub_der.as_bytes());
                    Ok(
                        serde_json::json!({ "privateKeyPem": priv_pem, "publicKeyPem": pub_pem })
                            .to_string(),
                    )
                }
                "P-384" | "secp384r1" => {
                    use p384::pkcs8::{EncodePrivateKey, EncodePublicKey};
                    let sk = p384::SecretKey::random(&mut rng);
                    let pk = sk.public_key();
                    let priv_der = sk
                        .to_pkcs8_der()
                        .map_err(|e| anyhow::anyhow!("EC P-384 private key encode failed: {e}"))?;
                    let pub_der = pk
                        .to_public_key_der()
                        .map_err(|e| anyhow::anyhow!("EC P-384 public key encode failed: {e}"))?;
                    let priv_pem = to_pem("PRIVATE KEY", priv_der.as_bytes());
                    let pub_pem = to_pem("PUBLIC KEY", pub_der.as_bytes());
                    Ok(
                        serde_json::json!({ "privateKeyPem": priv_pem, "publicKeyPem": pub_pem })
                            .to_string(),
                    )
                }
                other => Err(anyhow::anyhow!("unsupported EC curve: {other}")),
            }
        }
        "ed25519" => Err(anyhow::anyhow!(
            "ed25519 generateKeyPair: use crypto.subtle.generateKey with {{name:'Ed25519'}} instead"
        )),
        other => Err(anyhow::anyhow!("unsupported key type: {other}")),
    }
}

type HmacSha1 = Hmac<Sha1>;
type HmacSha224 = Hmac<Sha224>;
type HmacSha256 = Hmac<Sha256>;
type HmacSha384 = Hmac<Sha384>;
type HmacSha512 = Hmac<Sha512>;

pub(super) fn do_hash(algorithm: String, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    match norm_alg(&algorithm).as_str() {
        "md5" | "md-5" => Ok(Md5::digest(&data).to_vec()),
        "sha1" => Ok(Sha1::digest(&data).to_vec()),
        "sha224" => Ok(Sha224::digest(&data).to_vec()),
        "sha256" => Ok(Sha256::digest(&data).to_vec()),
        "sha384" => Ok(Sha384::digest(&data).to_vec()),
        "sha512" => Ok(Sha512::digest(&data).to_vec()),
        other => Err(anyhow::anyhow!("unsupported hash algorithm: {other}")),
    }
}

pub(super) fn do_rsa_sign(digest_alg: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use rsa::pkcs1v15::SigningKey;
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::signature::{SignatureEncoding, Signer};

    let priv_key = rsa::RsaPrivateKey::from_pkcs8_pem(pem)
        .map_err(|e| anyhow::anyhow!("RSA private key parse error: {e}"))?;
    let sig_bytes: Vec<u8> = match norm_alg(digest_alg).as_str() {
        "sha1" => SigningKey::<Sha1>::new(priv_key).sign(data).to_vec(),
        "sha224" => SigningKey::<Sha224>::new(priv_key).sign(data).to_vec(),
        "sha384" => SigningKey::<Sha384>::new(priv_key).sign(data).to_vec(),
        "sha512" => SigningKey::<Sha512>::new(priv_key).sign(data).to_vec(),
        _ => SigningKey::<Sha256>::new(priv_key).sign(data).to_vec(),
    };
    Ok(sig_bytes)
}

pub(super) fn do_rsa_verify(
    digest_alg: &str,
    pem: &str,
    data: &[u8],
    sig_bytes: &[u8],
) -> anyhow::Result<bool> {
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::pkcs8::DecodePublicKey;
    use rsa::signature::Verifier;

    let pub_key = rsa::RsaPublicKey::from_public_key_pem(pem)
        .map_err(|e| anyhow::anyhow!("RSA public key parse error: {e}"))?;
    let sig = Signature::try_from(sig_bytes)
        .map_err(|_| anyhow::anyhow!("RSA signature parse error: invalid bytes"))?;
    let ok = match norm_alg(digest_alg).as_str() {
        "sha1" => VerifyingKey::<Sha1>::new(pub_key).verify(data, &sig),
        "sha224" => VerifyingKey::<Sha224>::new(pub_key).verify(data, &sig),
        "sha384" => VerifyingKey::<Sha384>::new(pub_key).verify(data, &sig),
        "sha512" => VerifyingKey::<Sha512>::new(pub_key).verify(data, &sig),
        _ => VerifyingKey::<Sha256>::new(pub_key).verify(data, &sig),
    };
    Ok(ok.is_ok())
}

pub(super) fn do_rsa_pss_sign(digest_alg: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::pss::SigningKey;
    use rsa::signature::{RandomizedSigner, SignatureEncoding};

    let priv_key = rsa::RsaPrivateKey::from_pkcs8_pem(pem)
        .map_err(|e| anyhow::anyhow!("RSA private key parse error: {e}"))?;
    let mut rng = OsRng;
    let sig_bytes: Vec<u8> = match norm_alg(digest_alg).as_str() {
        "sha1" => SigningKey::<Sha1>::new(priv_key)
            .sign_with_rng(&mut rng, data)
            .to_vec(),
        "sha224" => SigningKey::<Sha224>::new(priv_key)
            .sign_with_rng(&mut rng, data)
            .to_vec(),
        "sha384" => SigningKey::<Sha384>::new(priv_key)
            .sign_with_rng(&mut rng, data)
            .to_vec(),
        "sha512" => SigningKey::<Sha512>::new(priv_key)
            .sign_with_rng(&mut rng, data)
            .to_vec(),
        _ => SigningKey::<Sha256>::new(priv_key)
            .sign_with_rng(&mut rng, data)
            .to_vec(),
    };
    Ok(sig_bytes)
}

pub(super) fn do_rsa_pss_verify(
    digest_alg: &str,
    pem: &str,
    data: &[u8],
    sig_bytes: &[u8],
) -> anyhow::Result<bool> {
    use rsa::pkcs8::DecodePublicKey;
    use rsa::pss::{Signature, VerifyingKey};
    use rsa::signature::Verifier;

    let pub_key = rsa::RsaPublicKey::from_public_key_pem(pem)
        .map_err(|e| anyhow::anyhow!("RSA public key parse error: {e}"))?;
    let sig = Signature::try_from(sig_bytes)
        .map_err(|_| anyhow::anyhow!("RSA signature parse error: invalid bytes"))?;
    let ok = match norm_alg(digest_alg).as_str() {
        "sha1" => VerifyingKey::<Sha1>::new(pub_key).verify(data, &sig),
        "sha224" => VerifyingKey::<Sha224>::new(pub_key).verify(data, &sig),
        "sha384" => VerifyingKey::<Sha384>::new(pub_key).verify(data, &sig),
        "sha512" => VerifyingKey::<Sha512>::new(pub_key).verify(data, &sig),
        _ => VerifyingKey::<Sha256>::new(pub_key).verify(data, &sig),
    };
    Ok(ok.is_ok())
}

pub(super) fn do_ec_sign(named_curve: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    match norm_alg(named_curve).as_str() {
        "p256" | "prime256v1" | "secp256r1" => {
            use p256::ecdsa::signature::Signer;
            use p256::ecdsa::{Signature, SigningKey};
            use p256::pkcs8::DecodePrivateKey;
            let key = SigningKey::from_pkcs8_pem(pem)
                .map_err(|e| anyhow::anyhow!("P-256 private key: {e}"))?;
            let sig: Signature = key.sign(data);
            Ok(sig.to_der().as_ref().to_vec())
        }
        "p384" | "secp384r1" => {
            use p384::ecdsa::signature::Signer;
            use p384::ecdsa::{Signature, SigningKey};
            use p384::pkcs8::DecodePrivateKey;
            let key = SigningKey::from_pkcs8_pem(pem)
                .map_err(|e| anyhow::anyhow!("P-384 private key: {e}"))?;
            let sig: Signature = key.sign(data);
            Ok(sig.to_der().as_ref().to_vec())
        }
        other => Err(anyhow::anyhow!("unsupported EC curve for signing: {other}")),
    }
}

/// Same as `do_ec_sign` but returns the WebCrypto/raw fixed-length r||s
/// encoding instead of DER — `subtle.sign` requires this exact byte length
/// (64 for P-256, 96 for P-384), unlike `crypto.createSign` which uses DER.
pub(super) fn do_ec_sign_raw(named_curve: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    match norm_alg(named_curve).as_str() {
        "p256" | "prime256v1" | "secp256r1" => {
            use p256::ecdsa::signature::Signer;
            use p256::ecdsa::{Signature, SigningKey};
            use p256::pkcs8::DecodePrivateKey;
            let key = SigningKey::from_pkcs8_pem(pem)
                .map_err(|e| anyhow::anyhow!("P-256 private key: {e}"))?;
            let sig: Signature = key.sign(data);
            Ok(sig.to_bytes().to_vec())
        }
        "p384" | "secp384r1" => {
            use p384::ecdsa::signature::Signer;
            use p384::ecdsa::{Signature, SigningKey};
            use p384::pkcs8::DecodePrivateKey;
            let key = SigningKey::from_pkcs8_pem(pem)
                .map_err(|e| anyhow::anyhow!("P-384 private key: {e}"))?;
            let sig: Signature = key.sign(data);
            Ok(sig.to_bytes().to_vec())
        }
        other => Err(anyhow::anyhow!("unsupported EC curve for signing: {other}")),
    }
}

pub(super) fn do_ec_verify(
    named_curve: &str,
    pem: &str,
    data: &[u8],
    sig_bytes: &[u8],
) -> anyhow::Result<bool> {
    match norm_alg(named_curve).as_str() {
        "p256" | "prime256v1" | "secp256r1" => {
            use p256::ecdsa::signature::Verifier;
            use p256::ecdsa::{Signature, VerifyingKey};
            use p256::pkcs8::DecodePublicKey;
            let pub_key = p256::PublicKey::from_public_key_pem(pem)
                .map_err(|e| anyhow::anyhow!("P-256 public key: {e}"))?;
            let vk = VerifyingKey::from(&pub_key);
            let sig = Signature::from_der(sig_bytes)
                .or_else(|_| Signature::try_from(sig_bytes))
                .map_err(|e| anyhow::anyhow!("P-256 signature parse: {e}"))?;
            Ok(vk.verify(data, &sig).is_ok())
        }
        "p384" | "secp384r1" => {
            use p384::ecdsa::signature::Verifier;
            use p384::ecdsa::{Signature, VerifyingKey};
            use p384::pkcs8::DecodePublicKey;
            let pub_key = p384::PublicKey::from_public_key_pem(pem)
                .map_err(|e| anyhow::anyhow!("P-384 public key: {e}"))?;
            let vk = VerifyingKey::from(&pub_key);
            let sig = Signature::from_der(sig_bytes)
                .or_else(|_| Signature::try_from(sig_bytes))
                .map_err(|e| anyhow::anyhow!("P-384 signature parse: {e}"))?;
            Ok(vk.verify(data, &sig).is_ok())
        }
        other => Err(anyhow::anyhow!("unsupported EC curve for verify: {other}")),
    }
}

pub(super) fn do_hmac(algorithm: String, key: Vec<u8>, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    macro_rules! run_hmac {
        ($T:ty) => {{
            let mut mac = <$T as hmac::Mac>::new_from_slice(&key)
                .map_err(|e| anyhow::anyhow!("invalid HMAC key: {e}"))?;
            mac.update(&data);
            Ok(mac.finalize().into_bytes().to_vec())
        }};
    }
    match norm_alg(&algorithm).as_str() {
        "sha1" => run_hmac!(HmacSha1),
        "sha224" => run_hmac!(HmacSha224),
        "sha256" => run_hmac!(HmacSha256),
        "sha384" => run_hmac!(HmacSha384),
        "sha512" => run_hmac!(HmacSha512),
        other => Err(anyhow::anyhow!("unsupported HMAC algorithm: {other}")),
    }
}

pub(super) fn do_random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    OsRng.fill_bytes(&mut buf);
    buf
}

pub(super) async fn do_pbkdf2(
    password: Vec<u8>,
    salt: Vec<u8>,
    iterations: u32,
    keylen: usize,
    digest: String,
) -> anyhow::Result<Vec<u8>> {
    let keylen = keylen.min(64 * 1024);
    tokio::task::spawn_blocking(move || {
        let mut out = vec![0u8; keylen];
        match norm_alg(&digest).as_str() {
            "sha1" => pbkdf2_hmac::<Sha1>(&password, &salt, iterations, &mut out),
            "sha224" => pbkdf2_hmac::<Sha224>(&password, &salt, iterations, &mut out),
            "sha256" => pbkdf2_hmac::<Sha256>(&password, &salt, iterations, &mut out),
            "sha384" => pbkdf2_hmac::<Sha384>(&password, &salt, iterations, &mut out),
            "sha512" => pbkdf2_hmac::<Sha512>(&password, &salt, iterations, &mut out),
            other => return Err(anyhow::anyhow!("unsupported PBKDF2 digest: {other}")),
        }
        Ok(out)
    })
    .await?
}

pub(super) fn do_pbkdf2_sync(
    password: Vec<u8>,
    salt: Vec<u8>,
    iterations: u32,
    keylen: usize,
    digest: String,
) -> anyhow::Result<Vec<u8>> {
    let keylen = keylen.min(64 * 1024);
    let mut out = vec![0u8; keylen];
    match norm_alg(&digest).as_str() {
        "sha1" => pbkdf2_hmac::<Sha1>(&password, &salt, iterations, &mut out),
        "sha224" => pbkdf2_hmac::<Sha224>(&password, &salt, iterations, &mut out),
        "sha256" => pbkdf2_hmac::<Sha256>(&password, &salt, iterations, &mut out),
        "sha384" => pbkdf2_hmac::<Sha384>(&password, &salt, iterations, &mut out),
        "sha512" => pbkdf2_hmac::<Sha512>(&password, &salt, iterations, &mut out),
        other => return Err(anyhow::anyhow!("unsupported PBKDF2 digest: {other}")),
    }
    Ok(out)
}

pub(super) fn do_cipher_one_shot(
    alg: &str,
    key: &[u8],
    iv: &[u8],
    data: &[u8],
    encrypt: bool,
) -> anyhow::Result<Vec<u8>> {
    let alg_lower = alg.to_lowercase();
    match alg_lower.as_str() {
        "aes-128-cbc" | "aes128" if encrypt => {
            let enc = Aes128CbcEnc::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-128-cbc key/iv error: {e}"))?;
            Ok(enc.encrypt_padded_vec_mut::<Pkcs7>(data))
        }
        "aes-192-cbc" if encrypt => {
            let enc = Aes192CbcEnc::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-192-cbc key/iv error: {e}"))?;
            Ok(enc.encrypt_padded_vec_mut::<Pkcs7>(data))
        }
        "aes-256-cbc" | "aes256" if encrypt => {
            let enc = Aes256CbcEnc::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-256-cbc key/iv error: {e}"))?;
            Ok(enc.encrypt_padded_vec_mut::<Pkcs7>(data))
        }
        "aes-128-cbc" | "aes128" => {
            let dec = Aes128CbcDec::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-128-cbc key/iv error: {e}"))?;
            dec.decrypt_padded_vec_mut::<Pkcs7>(data)
                .map_err(|e| anyhow::anyhow!("aes-128-cbc decrypt error: {e}"))
        }
        "aes-192-cbc" => {
            let dec = Aes192CbcDec::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-192-cbc key/iv error: {e}"))?;
            dec.decrypt_padded_vec_mut::<Pkcs7>(data)
                .map_err(|e| anyhow::anyhow!("aes-192-cbc decrypt error: {e}"))
        }
        "aes-256-cbc" | "aes256" => {
            let dec = Aes256CbcDec::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-256-cbc key/iv error: {e}"))?;
            dec.decrypt_padded_vec_mut::<Pkcs7>(data)
                .map_err(|e| anyhow::anyhow!("aes-256-cbc decrypt error: {e}"))
        }
        "aes-128-ctr" => {
            let mut cipher = Aes128Ctr::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-128-ctr key/iv error: {e}"))?;
            let mut out = data.to_vec();
            cipher.apply_keystream(&mut out);
            Ok(out)
        }
        "aes-192-ctr" => {
            let mut cipher = Aes192Ctr::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-192-ctr key/iv error: {e}"))?;
            let mut out = data.to_vec();
            cipher.apply_keystream(&mut out);
            Ok(out)
        }
        "aes-256-ctr" => {
            let mut cipher = Aes256Ctr::new_from_slices(key, iv)
                .map_err(|e| anyhow::anyhow!("aes-256-ctr key/iv error: {e}"))?;
            let mut out = data.to_vec();
            cipher.apply_keystream(&mut out);
            Ok(out)
        }
        other => Err(anyhow::anyhow!("unsupported cipher algorithm: {other}")),
    }
}

pub(super) async fn do_scrypt(
    password: Vec<u8>,
    salt: Vec<u8>,
    n: u64,
    r: u32,
    p: u32,
    keylen: usize,
) -> anyhow::Result<Vec<u8>> {
    let keylen = keylen.min(64 * 1024);
    tokio::task::spawn_blocking(move || {
        if n == 0 || (n & (n - 1)) != 0 {
            return Err(anyhow::anyhow!(
                "scrypt N must be a power of 2 greater than 1"
            ));
        }
        let log_n = n.ilog2() as u8;
        let params = scrypt::Params::new(log_n, r, p, keylen)
            .map_err(|e| anyhow::anyhow!("invalid scrypt params: {e}"))?;
        let mut out = vec![0u8; keylen];
        scrypt::scrypt(&password, &salt, &params, &mut out)
            .map_err(|e| anyhow::anyhow!("scrypt error: {e}"))?;
        Ok(out)
    })
    .await?
}

pub(super) fn do_aes_gcm_encrypt(
    key_len: usize,
    key: Vec<u8>,
    iv: Vec<u8>,
    plaintext: Vec<u8>,
    aad: Vec<u8>,
) -> anyhow::Result<Vec<u8>> {
    if key.len() != key_len {
        anyhow::bail!("AES-GCM key must be {} bytes, got {}", key_len, key.len());
    }
    if iv.len() != 12 {
        anyhow::bail!("AES-GCM IV must be 12 bytes, got {}", iv.len());
    }
    let nonce = Nonce::from_slice(&iv);
    let payload = Payload {
        msg: &plaintext,
        aad: &aad,
    };
    match key_len {
        16 => {
            let cipher = Aes128Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
            cipher
                .encrypt(nonce, payload)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        32 => {
            let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
            cipher
                .encrypt(nonce, payload)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        n => anyhow::bail!("unsupported AES key length: {}", n),
    }
}

pub(super) fn do_aes_gcm_decrypt(
    key_len: usize,
    key: Vec<u8>,
    iv: Vec<u8>,
    ciphertext_and_tag: Vec<u8>,
    aad: Vec<u8>,
) -> anyhow::Result<Vec<u8>> {
    if key.len() != key_len {
        anyhow::bail!("AES-GCM key must be {} bytes, got {}", key_len, key.len());
    }
    if iv.len() != 12 {
        anyhow::bail!("AES-GCM IV must be 12 bytes, got {}", iv.len());
    }
    let nonce = Nonce::from_slice(&iv);
    let payload = Payload {
        msg: &ciphertext_and_tag,
        aad: &aad,
    };
    match key_len {
        16 => {
            let cipher = Aes128Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
            cipher
                .decrypt(nonce, payload)
                .map_err(|_| anyhow::anyhow!("decryption failed"))
        }
        32 => {
            let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
            cipher
                .decrypt(nonce, payload)
                .map_err(|_| anyhow::anyhow!("decryption failed"))
        }
        n => anyhow::bail!("unsupported AES key length: {}", n),
    }
}

pub(super) fn do_scrypt_sync(
    password: &[u8],
    salt: &[u8],
    n: u64,
    r: u32,
    p: u32,
    keylen: usize,
) -> Result<Vec<u8>, String> {
    if n == 0 || (n & (n - 1)) != 0 {
        return Err("N must be a power of 2 greater than 1".into());
    }
    let params = scrypt::Params::new(n.ilog2() as u8, r, p, keylen)
        .map_err(|e| format!("invalid scrypt params: {e}"))?;
    let mut out = vec![0u8; keylen];
    scrypt::scrypt(password, salt, &params, &mut out).map_err(|e| format!("scrypt error: {e}"))?;
    Ok(out)
}

pub(super) fn do_ecdh_compute(
    curve: &str,
    priv_pem: &str,
    other_pub: &[u8],
) -> Result<Vec<u8>, String> {
    let curve_lower = curve.to_lowercase();
    let is_p384 = curve_lower.contains("384") || curve_lower.contains("secp384");
    if is_p384 {
        use p384::pkcs8::DecodePrivateKey;
        let sk = p384::SecretKey::from_pkcs8_pem(priv_pem)
            .map_err(|e| format!("ECDH: invalid private key: {e}"))?;
        let other_pk = p384::PublicKey::from_sec1_bytes(other_pub)
            .map_err(|e| format!("ECDH: invalid public key: {e}"))?;
        let shared = p384::ecdh::diffie_hellman(sk.to_nonzero_scalar(), other_pk.as_affine());
        Ok(shared.raw_secret_bytes().to_vec())
    } else {
        use p256::pkcs8::DecodePrivateKey;
        let sk = p256::SecretKey::from_pkcs8_pem(priv_pem)
            .map_err(|e| format!("ECDH: invalid private key: {e}"))?;
        let other_pk = p256::PublicKey::from_sec1_bytes(other_pub)
            .map_err(|e| format!("ECDH: invalid public key: {e}"))?;
        let shared = p256::ecdh::diffie_hellman(sk.to_nonzero_scalar(), other_pk.as_affine());
        Ok(shared.raw_secret_bytes().to_vec())
    }
}
