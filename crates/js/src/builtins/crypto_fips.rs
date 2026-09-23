//! `fips` build of the native `crypto` ops: same signatures as the RustCrypto
//! versions in `crypto.rs`, backed by the AWS-LC FIPS 140-3 module. Algorithms
//! the module does not offer as approved services (MD5, scrypt, SHA-1/SHA-224
//! signing, RSA < 2048) fail with `ERR_CRYPTO_FIPS_FORCED`, like Node's
//! `--force-fips`.

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::{self, KeyPair};
use aws_lc_rs::{aead, agreement, cipher, digest, hmac, pbkdf2, rand, rsa};
use std::num::NonZeroU32;

use super::crypto::{norm_alg, to_pem};

fn forced(what: &str) -> anyhow::Error {
    anyhow::anyhow!("ERR_CRYPTO_FIPS_FORCED: {what} is not available in the FIPS build of 3va")
}

/// Body of a PEM block → DER. Labels are not checked; aws-lc rejects wrong key types.
fn pem_to_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let b64: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("-----"))
        .collect();
    STANDARD
        .decode(b64)
        .map_err(|e| anyhow::anyhow!("invalid PEM: {e}"))
}

fn keypair_json(priv_der: &[u8], pub_der: &[u8]) -> String {
    serde_json::json!({
        "privateKeyPem": to_pem("PRIVATE KEY", priv_der),
        "publicKeyPem": to_pem("PUBLIC KEY", pub_der),
    })
    .to_string()
}

fn unspecified(what: &str) -> impl Fn(aws_lc_rs::error::Unspecified) -> anyhow::Error + '_ {
    move |_| anyhow::anyhow!("{what} failed")
}

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
            let bits = opts
                .get("modulusLength")
                .and_then(|v| v.as_u64())
                .unwrap_or(2048);
            let size = match bits {
                2048 => rsa::KeySize::Rsa2048,
                3072 => rsa::KeySize::Rsa3072,
                4096 => rsa::KeySize::Rsa4096,
                8192 => rsa::KeySize::Rsa8192,
                other => {
                    return Err(forced(&format!(
                        "RSA modulusLength {other} (use 2048, 3072, 4096 or 8192)"
                    )));
                }
            };
            let kp = rsa::KeyPair::generate(size).map_err(unspecified("RSA keygen"))?;
            let priv_der = kp.as_der().map_err(unspecified("RSA private key encode"))?;
            let pub_der = kp
                .public_key()
                .as_der()
                .map_err(unspecified("RSA public key encode"))?;
            Ok(keypair_json(priv_der.as_ref(), pub_der.as_ref()))
        }
        "ec" => {
            let curve = opts
                .get("namedCurve")
                .and_then(|v| v.as_str())
                .unwrap_or("P-256");
            let alg = ecdsa_alg(curve, false)?;
            let kp = signature::EcdsaKeyPair::generate(alg).map_err(unspecified("EC keygen"))?;
            let priv_der = kp
                .to_pkcs8v1()
                .map_err(unspecified("EC private key encode"))?;
            let pub_der = kp
                .public_key()
                .as_der()
                .map_err(unspecified("EC public key encode"))?;
            Ok(keypair_json(priv_der.as_ref(), pub_der.as_ref()))
        }
        "ed25519" => Err(forced("Ed25519")),
        other => Err(anyhow::anyhow!("unsupported key type: {other}")),
    }
}

fn digest_alg(algorithm: &str) -> anyhow::Result<&'static digest::Algorithm> {
    match norm_alg(algorithm).as_str() {
        "sha1" => Ok(&digest::SHA1_FOR_LEGACY_USE_ONLY),
        "sha224" => Ok(&digest::SHA224),
        "sha256" => Ok(&digest::SHA256),
        "sha384" => Ok(&digest::SHA384),
        "sha512" => Ok(&digest::SHA512),
        "md5" => Err(forced("MD5")),
        other => Err(anyhow::anyhow!("unsupported hash algorithm: {other}")),
    }
}

pub(super) fn do_hash(algorithm: String, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    Ok(digest::digest(digest_alg(&algorithm)?, &data)
        .as_ref()
        .to_vec())
}

pub(super) fn do_hmac(algorithm: String, key: Vec<u8>, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let alg = match norm_alg(&algorithm).as_str() {
        "sha1" => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        "sha224" => hmac::HMAC_SHA224,
        "sha256" => hmac::HMAC_SHA256,
        "sha384" => hmac::HMAC_SHA384,
        "sha512" => hmac::HMAC_SHA512,
        other => return Err(anyhow::anyhow!("unsupported HMAC algorithm: {other}")),
    };
    Ok(hmac::sign(&hmac::Key::new(alg, &key), &data)
        .as_ref()
        .to_vec())
}

pub(super) fn do_random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    // The FIPS DRBG only fails if the module is in an error state; nothing
    // after that point may hand out key material.
    rand::fill(&mut buf).expect("FIPS DRBG failure");
    buf
}

fn pbkdf2_run(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    keylen: usize,
    digest: &str,
) -> anyhow::Result<Vec<u8>> {
    let alg = match norm_alg(digest).as_str() {
        "sha1" => pbkdf2::PBKDF2_HMAC_SHA1,
        "sha256" => pbkdf2::PBKDF2_HMAC_SHA256,
        "sha384" => pbkdf2::PBKDF2_HMAC_SHA384,
        "sha512" => pbkdf2::PBKDF2_HMAC_SHA512,
        "sha224" => return Err(forced("PBKDF2 with SHA-224")),
        other => return Err(anyhow::anyhow!("unsupported PBKDF2 digest: {other}")),
    };
    let iterations = NonZeroU32::new(iterations)
        .ok_or_else(|| anyhow::anyhow!("PBKDF2 iterations must be > 0"))?;
    let mut out = vec![0u8; keylen.min(64 * 1024)];
    // `derive` panics only on an internal module error; never unwind into V8.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pbkdf2::derive(alg, iterations, salt, password, &mut out)
    }))
    .map_err(|_| anyhow::anyhow!("PBKDF2 derive failed"))?;
    Ok(out)
}

pub(super) async fn do_pbkdf2(
    password: Vec<u8>,
    salt: Vec<u8>,
    iterations: u32,
    keylen: usize,
    digest: String,
) -> anyhow::Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || pbkdf2_run(&password, &salt, iterations, keylen, &digest))
        .await?
}

pub(super) fn do_pbkdf2_sync(
    password: Vec<u8>,
    salt: Vec<u8>,
    iterations: u32,
    keylen: usize,
    digest: String,
) -> anyhow::Result<Vec<u8>> {
    pbkdf2_run(&password, &salt, iterations, keylen, &digest)
}

pub(super) async fn do_scrypt(
    _password: Vec<u8>,
    _salt: Vec<u8>,
    _n: u64,
    _r: u32,
    _p: u32,
    _keylen: usize,
) -> anyhow::Result<Vec<u8>> {
    Err(forced("scrypt"))
}

pub(super) fn do_scrypt_sync(
    _password: &[u8],
    _salt: &[u8],
    _n: u64,
    _r: u32,
    _p: u32,
    _keylen: usize,
) -> Result<Vec<u8>, String> {
    Err(forced("scrypt").to_string())
}

pub(super) fn do_cipher_one_shot(
    alg: &str,
    key: &[u8],
    iv: &[u8],
    data: &[u8],
    encrypt: bool,
) -> anyhow::Result<Vec<u8>> {
    let alg_lower = alg.to_lowercase();
    let (aes, cbc) = match alg_lower.as_str() {
        "aes-128-cbc" | "aes128" => (&cipher::AES_128, true),
        "aes-192-cbc" => (&cipher::AES_192, true),
        "aes-256-cbc" | "aes256" => (&cipher::AES_256, true),
        "aes-128-ctr" => (&cipher::AES_128, false),
        "aes-192-ctr" => (&cipher::AES_192, false),
        "aes-256-ctr" => (&cipher::AES_256, false),
        other => return Err(anyhow::anyhow!("unsupported cipher algorithm: {other}")),
    };
    let key = cipher::UnboundCipherKey::new(aes, key)
        .map_err(|_| anyhow::anyhow!("{alg_lower} key/iv error: invalid key length"))?;
    let iv: [u8; 16] = iv
        .try_into()
        .map_err(|_| anyhow::anyhow!("{alg_lower} key/iv error: IV must be 16 bytes"))?;
    let mut buf = data.to_vec();
    let err = |_| {
        anyhow::anyhow!(
            "{alg_lower} {} error",
            if encrypt { "encrypt" } else { "decrypt" }
        )
    };
    match (cbc, encrypt) {
        (true, true) => {
            let k = cipher::PaddedBlockEncryptingKey::cbc_pkcs7(key).map_err(err)?;
            k.less_safe_encrypt(&mut buf, cipher::EncryptionContext::Iv128(iv.into()))
                .map_err(err)?;
            Ok(buf)
        }
        (true, false) => {
            let k = cipher::PaddedBlockDecryptingKey::cbc_pkcs7(key).map_err(err)?;
            let n = k
                .decrypt(&mut buf, cipher::DecryptionContext::Iv128(iv.into()))
                .map_err(err)?
                .len();
            buf.truncate(n);
            Ok(buf)
        }
        // CTR is its own inverse.
        (false, _) => {
            let k = cipher::EncryptingKey::ctr(key).map_err(err)?;
            k.less_safe_encrypt(&mut buf, cipher::EncryptionContext::Iv128(iv.into()))
                .map_err(err)?;
            Ok(buf)
        }
    }
}

fn gcm_key(
    key_len: usize,
    key: &[u8],
    iv: &[u8],
) -> anyhow::Result<(aead::LessSafeKey, aead::Nonce)> {
    if key.len() != key_len {
        anyhow::bail!("AES-GCM key must be {} bytes, got {}", key_len, key.len());
    }
    let alg = match key_len {
        16 => &aead::AES_128_GCM,
        32 => &aead::AES_256_GCM,
        n => anyhow::bail!("unsupported AES key length: {}", n),
    };
    let nonce = aead::Nonce::try_assume_unique_for_key(iv)
        .map_err(|_| anyhow::anyhow!("AES-GCM IV must be 12 bytes, got {}", iv.len()))?;
    let key =
        aead::UnboundKey::new(alg, key).map_err(|_| anyhow::anyhow!("invalid AES-GCM key"))?;
    Ok((aead::LessSafeKey::new(key), nonce))
}

pub(super) fn do_aes_gcm_encrypt(
    key_len: usize,
    key: Vec<u8>,
    iv: Vec<u8>,
    plaintext: Vec<u8>,
    aad: Vec<u8>,
) -> anyhow::Result<Vec<u8>> {
    let (key, nonce) = gcm_key(key_len, &key, &iv)?;
    let mut buf = plaintext;
    key.seal_in_place_append_tag(nonce, aead::Aad::from(&aad), &mut buf)
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    Ok(buf)
}

pub(super) fn do_aes_gcm_decrypt(
    key_len: usize,
    key: Vec<u8>,
    iv: Vec<u8>,
    ciphertext_and_tag: Vec<u8>,
    aad: Vec<u8>,
) -> anyhow::Result<Vec<u8>> {
    let (key, nonce) = gcm_key(key_len, &key, &iv)?;
    let mut buf = ciphertext_and_tag;
    let n = key
        .open_in_place(nonce, aead::Aad::from(&aad), &mut buf)
        .map_err(|_| anyhow::anyhow!("decryption failed"))?
        .len();
    buf.truncate(n);
    Ok(buf)
}

fn rsa_signing_alg(digest: &str, pss: bool) -> anyhow::Result<&'static dyn signature::RsaEncoding> {
    Ok(match (norm_alg(digest).as_str(), pss) {
        ("sha384", false) => &signature::RSA_PKCS1_SHA384,
        ("sha512", false) => &signature::RSA_PKCS1_SHA512,
        ("sha384", true) => &signature::RSA_PSS_SHA384,
        ("sha512", true) => &signature::RSA_PSS_SHA512,
        ("sha1" | "sha224", _) => return Err(forced("RSA signing with SHA-1/SHA-224")),
        (_, false) => &signature::RSA_PKCS1_SHA256,
        (_, true) => &signature::RSA_PSS_SHA256,
    })
}

fn rsa_sign(digest: &str, pem: &str, data: &[u8], pss: bool) -> anyhow::Result<Vec<u8>> {
    let kp = signature::RsaKeyPair::from_pkcs8(&pem_to_der(pem)?)
        .map_err(|e| anyhow::anyhow!("RSA private key parse error: {e}"))?;
    let mut sig = vec![0u8; kp.public_modulus_len()];
    kp.sign(
        rsa_signing_alg(digest, pss)?,
        &rand::SystemRandom::new(),
        data,
        &mut sig,
    )
    .map_err(unspecified("RSA sign"))?;
    Ok(sig)
}

fn rsa_verify(digest: &str, pem: &str, data: &[u8], sig: &[u8], pss: bool) -> anyhow::Result<bool> {
    let alg: &'static dyn signature::VerificationAlgorithm = match (norm_alg(digest).as_str(), pss)
    {
        // SP 800-131A: SHA-1 stays approved for verifying legacy signatures.
        ("sha1", false) => &signature::RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY,
        ("sha384", false) => &signature::RSA_PKCS1_2048_8192_SHA384,
        ("sha512", false) => &signature::RSA_PKCS1_2048_8192_SHA512,
        ("sha384", true) => &signature::RSA_PSS_2048_8192_SHA384,
        ("sha512", true) => &signature::RSA_PSS_2048_8192_SHA512,
        ("sha1" | "sha224", _) => return Err(forced("RSA verification with this digest")),
        (_, false) => &signature::RSA_PKCS1_2048_8192_SHA256,
        (_, true) => &signature::RSA_PSS_2048_8192_SHA256,
    };
    Ok(parse_public_key(alg, pem)?.verify_sig(data, sig).is_ok())
}

/// Parse up front so a key of the wrong type is an error, not `false`: the JS
/// glue tries each EC curve (then RSA) until one accepts the key.
fn parse_public_key(
    alg: &'static dyn signature::VerificationAlgorithm,
    pem: &str,
) -> anyhow::Result<signature::ParsedPublicKey> {
    signature::ParsedPublicKey::new(alg, pem_to_der(pem)?)
        .map_err(|e| anyhow::anyhow!("public key parse error: {e}"))
}

pub(super) fn do_rsa_sign(digest_alg: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    rsa_sign(digest_alg, pem, data, false)
}

pub(super) fn do_rsa_verify(
    digest_alg: &str,
    pem: &str,
    data: &[u8],
    sig_bytes: &[u8],
) -> anyhow::Result<bool> {
    rsa_verify(digest_alg, pem, data, sig_bytes, false)
}

pub(super) fn do_rsa_pss_sign(digest_alg: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    rsa_sign(digest_alg, pem, data, true)
}

pub(super) fn do_rsa_pss_verify(
    digest_alg: &str,
    pem: &str,
    data: &[u8],
    sig_bytes: &[u8],
) -> anyhow::Result<bool> {
    rsa_verify(digest_alg, pem, data, sig_bytes, true)
}

/// P-256 signs with SHA-256 and P-384 with SHA-384, matching the RustCrypto path.
fn ecdsa_alg(
    curve: &str,
    fixed: bool,
) -> anyhow::Result<&'static signature::EcdsaSigningAlgorithm> {
    Ok(match (norm_alg(curve).as_str(), fixed) {
        ("p256" | "prime256v1" | "secp256r1", false) => &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        ("p256" | "prime256v1" | "secp256r1", true) => &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        ("p384" | "secp384r1", false) => &signature::ECDSA_P384_SHA384_ASN1_SIGNING,
        ("p384" | "secp384r1", true) => &signature::ECDSA_P384_SHA384_FIXED_SIGNING,
        (other, _) => return Err(anyhow::anyhow!("unsupported EC curve: {other}")),
    })
}

fn ec_sign(curve: &str, pem: &str, data: &[u8], fixed: bool) -> anyhow::Result<Vec<u8>> {
    let kp = signature::EcdsaKeyPair::from_pkcs8(ecdsa_alg(curve, fixed)?, &pem_to_der(pem)?)
        .map_err(|e| anyhow::anyhow!("EC private key: {e}"))?;
    let sig = kp
        .sign(&rand::SystemRandom::new(), data)
        .map_err(unspecified("ECDSA sign"))?;
    Ok(sig.as_ref().to_vec())
}

pub(super) fn do_ec_sign(named_curve: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    ec_sign(named_curve, pem, data, false)
}

pub(super) fn do_ec_sign_raw(named_curve: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    ec_sign(named_curve, pem, data, true)
}

/// Accepts DER or fixed r||s signatures, like the RustCrypto path.
pub(super) fn do_ec_verify(
    named_curve: &str,
    pem: &str,
    data: &[u8],
    sig_bytes: &[u8],
) -> anyhow::Result<bool> {
    let (asn1, fixed, raw_len): (
        &'static dyn signature::VerificationAlgorithm,
        &'static dyn signature::VerificationAlgorithm,
        usize,
    ) = match norm_alg(named_curve).as_str() {
        "p256" | "prime256v1" | "secp256r1" => (
            &signature::ECDSA_P256_SHA256_ASN1,
            &signature::ECDSA_P256_SHA256_FIXED,
            64,
        ),
        "p384" | "secp384r1" => (
            &signature::ECDSA_P384_SHA384_ASN1,
            &signature::ECDSA_P384_SHA384_FIXED,
            96,
        ),
        other => return Err(anyhow::anyhow!("unsupported EC curve for verify: {other}")),
    };
    let alg = if sig_bytes.len() == raw_len {
        fixed
    } else {
        asn1
    };
    Ok(parse_public_key(alg, pem)?
        .verify_sig(data, sig_bytes)
        .is_ok())
}

pub(super) fn do_ecdh_compute(
    curve: &str,
    priv_pem: &str,
    other_pub: &[u8],
) -> Result<Vec<u8>, String> {
    let c = curve.to_lowercase();
    let alg = if c.contains("384") {
        &agreement::ECDH_P384
    } else {
        &agreement::ECDH_P256
    };
    let der = pem_to_der(priv_pem).map_err(|e| format!("ECDH: invalid private key: {e}"))?;
    let sk = agreement::PrivateKey::from_private_key_der(alg, &der)
        .map_err(|e| format!("ECDH: invalid private key: {e}"))?;
    agreement::agree(
        &sk,
        agreement::UnparsedPublicKey::new(alg, other_pub),
        "ECDH: invalid public key".to_string(),
        |secret| Ok(secret.to_vec()),
    )
}
