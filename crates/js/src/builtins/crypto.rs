use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
use hmac::{Hmac, Mac};
use md5::Md5;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use rand::rngs::OsRng;
use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use vvva_crypto as pq;

/// Encode DER bytes as a PEM block (64-char line wrapping).
fn to_pem(label: &str, der: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let b64 = STANDARD.encode(der);
    let lines = b64
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    format!("-----BEGIN {label}-----\n{lines}\n-----END {label}-----\n")
}

async fn do_generate_keypair(key_type: String, options_json: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || do_generate_keypair_sync_inner(&key_type, &options_json))
        .await?
}

fn do_generate_keypair_sync_inner(key_type: &str, options_json: &str) -> anyhow::Result<String> {
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
        "ed25519" => {
            // Ed25519 via random bytes + PKCS#8 encoding approximation
            // Use P-256 as fallback since ed25519 crate isn't in the workspace
            Err(anyhow::anyhow!(
                "ed25519 generateKeyPair: use crypto.subtle.generateKey with {{name:'Ed25519'}} instead"
            ))
        }
        other => Err(anyhow::anyhow!("unsupported key type: {other}")),
    }
}

type HmacSha1 = Hmac<Sha1>;
type HmacSha224 = Hmac<Sha224>;
type HmacSha256 = Hmac<Sha256>;
type HmacSha384 = Hmac<Sha384>;
type HmacSha512 = Hmac<Sha512>;

fn norm_alg(alg: &str) -> String {
    alg.to_lowercase().replace(['-', '_', ' '], "")
}

fn do_hash(algorithm: String, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
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

// ── Asymmetric signing / verification ─────────────────────────────────────────

fn do_rsa_sign(digest_alg: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
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

fn do_rsa_verify(
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

fn do_ec_sign(named_curve: &str, pem: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
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

fn do_ec_verify(
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
            // Accept DER (variable length) or fixed P1363 (64 bytes)
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

fn do_hmac(algorithm: String, key: Vec<u8>, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
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

fn do_random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    OsRng.fill_bytes(&mut buf);
    buf
}

// Constant-time comparison: avoids short-circuit optimisation via black_box.
fn do_timing_safe_equal(a: Vec<u8>, b: Vec<u8>) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    std::hint::black_box(acc) == 0
}

async fn do_pbkdf2(
    password: Vec<u8>,
    salt: Vec<u8>,
    iterations: u32,
    keylen: usize,
    digest: String,
) -> anyhow::Result<Vec<u8>> {
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

async fn do_scrypt(
    password: Vec<u8>,
    salt: Vec<u8>,
    n: u64,
    r: u32,
    p: u32,
    keylen: usize,
) -> anyhow::Result<Vec<u8>> {
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

// ── AES-GCM ───────────────────────────────────────────────────────────────────

/// `key_len` must be 16 (AES-128) or 32 (AES-256).
/// Returns `ciphertext || tag` (GCM appends 16-byte tag).
fn do_aes_gcm_encrypt(
    key_len: usize,
    key: Vec<u8>,
    iv: Vec<u8>,
    plaintext: Vec<u8>,
    aad: Vec<u8>,
) -> anyhow::Result<Vec<u8>> {
    if key.len() != key_len {
        anyhow::bail!("AES-GCM key must be {} bytes, got {}", key_len, key.len());
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

/// `ciphertext_and_tag` is the combined ciphertext||tag output of encrypt.
fn do_aes_gcm_decrypt(
    key_len: usize,
    key: Vec<u8>,
    iv: Vec<u8>,
    ciphertext_and_tag: Vec<u8>,
    aad: Vec<u8>,
) -> anyhow::Result<Vec<u8>> {
    if key.len() != key_len {
        anyhow::bail!("AES-GCM key must be {} bytes, got {}", key_len, key.len());
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

fn js_err(op: &'static str, e: anyhow::Error) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("crypto", op, e.to_string())
}

pub fn inject_crypto(ctx: &Ctx) -> Result<()> {
    ctx.globals().set(
        "__cryptoHash",
        Function::new(ctx.clone(), |algorithm: String, data: Vec<u8>| {
            do_hash(algorithm, data).map_err(|e| js_err("hash", e))
        })?,
    )?;

    ctx.globals().set(
        "__cryptoHmac",
        Function::new(
            ctx.clone(),
            |algorithm: String, key: Vec<u8>, data: Vec<u8>| {
                do_hmac(algorithm, key, data).map_err(|e| js_err("hmac", e))
            },
        )?,
    )?;

    ctx.globals().set(
        "__cryptoRandomBytes",
        Function::new(ctx.clone(), |n: usize| -> Vec<u8> { do_random_bytes(n) })?,
    )?;

    ctx.globals().set(
        "__cryptoTimingSafeEqual",
        Function::new(ctx.clone(), |a: Vec<u8>, b: Vec<u8>| -> bool {
            do_timing_safe_equal(a, b)
        })?,
    )?;

    ctx.globals().set(
        "__cryptoPbkdf2",
        Function::new(
            ctx.clone(),
            Async(
                move |password: Vec<u8>,
                      salt: Vec<u8>,
                      iterations: u32,
                      keylen: usize,
                      digest: String| async move {
                    do_pbkdf2(password, salt, iterations, keylen, digest)
                        .await
                        .map_err(|e| js_err("pbkdf2", e))
                },
            ),
        )?,
    )?;

    ctx.globals().set(
        "__cryptoAesGcmEncrypt",
        Function::new(
            ctx.clone(),
            |key_len: usize, key: Vec<u8>, iv: Vec<u8>, plaintext: Vec<u8>, aad: Vec<u8>| {
                do_aes_gcm_encrypt(key_len, key, iv, plaintext, aad)
                    .map_err(|e| js_err("aes-gcm-encrypt", e))
            },
        )?,
    )?;

    ctx.globals().set(
        "__cryptoAesGcmDecrypt",
        Function::new(
            ctx.clone(),
            |key_len: usize,
             key: Vec<u8>,
             iv: Vec<u8>,
             ciphertext_and_tag: Vec<u8>,
             aad: Vec<u8>| {
                do_aes_gcm_decrypt(key_len, key, iv, ciphertext_and_tag, aad)
                    .map_err(|e| js_err("aes-gcm-decrypt", e))
            },
        )?,
    )?;

    ctx.globals().set(
        "__cryptoScrypt",
        Function::new(
            ctx.clone(),
            Async(
                move |password: Vec<u8>,
                      salt: Vec<u8>,
                      n: u64,
                      r: u32,
                      p: u32,
                      keylen: usize| async move {
                    do_scrypt(password, salt, n, r, p, keylen)
                        .await
                        .map_err(|e| js_err("scrypt", e))
                },
            ),
        )?,
    )?;

    // ── asymmetric sign / verify ─────────────────────────────────────────────────
    ctx.globals().set(
        "__cryptoRsaSign",
        Function::new(
            ctx.clone(),
            |digest_alg: String, pem: String, data: Vec<u8>| {
                do_rsa_sign(&digest_alg, &pem, &data).map_err(|e| js_err("rsaSign", e))
            },
        )?,
    )?;

    ctx.globals().set(
        "__cryptoRsaVerify",
        Function::new(
            ctx.clone(),
            |digest_alg: String, pem: String, data: Vec<u8>, sig: Vec<u8>| {
                do_rsa_verify(&digest_alg, &pem, &data, &sig).map_err(|e| js_err("rsaVerify", e))
            },
        )?,
    )?;

    ctx.globals().set(
        "__cryptoEcSign",
        Function::new(
            ctx.clone(),
            |named_curve: String, pem: String, data: Vec<u8>| {
                do_ec_sign(&named_curve, &pem, &data).map_err(|e| js_err("ecSign", e))
            },
        )?,
    )?;

    ctx.globals().set(
        "__cryptoEcVerify",
        Function::new(
            ctx.clone(),
            |named_curve: String, pem: String, data: Vec<u8>, sig: Vec<u8>| {
                do_ec_verify(&named_curve, &pem, &data, &sig).map_err(|e| js_err("ecVerify", e))
            },
        )?,
    )?;

    // ── scrypt sync ──────────────────────────────────────────────────────────────
    ctx.globals().set(
        "__cryptoScryptSync",
        Function::new(
            ctx.clone(),
            |password: Vec<u8>, salt: Vec<u8>, n: u64, r: u32, p: u32, keylen: usize| {
                if n == 0 || (n & (n - 1)) != 0 {
                    return Err(js_err(
                        "scryptSync",
                        anyhow::anyhow!("N must be a power of 2 greater than 1"),
                    ));
                }
                let log_n = n.ilog2() as u8;
                let params = scrypt::Params::new(log_n, r, p, keylen).map_err(|e| {
                    js_err("scryptSync", anyhow::anyhow!("invalid scrypt params: {e}"))
                })?;
                let mut out = vec![0u8; keylen];
                scrypt::scrypt(&password, &salt, &params, &mut out)
                    .map_err(|e| js_err("scryptSync", anyhow::anyhow!("scrypt error: {e}")))?;
                Ok(out)
            },
        )?,
    )?;

    // ── key pair generation (async + sync) ────────────────────────────────────
    ctx.globals().set(
        "__cryptoGenerateKeyPair",
        Function::new(
            ctx.clone(),
            Async(move |key_type: String, options_json: String| async move {
                do_generate_keypair(key_type, options_json)
                    .await
                    .map_err(|e| js_err("generateKeyPair", e))
            }),
        )?,
    )?;

    ctx.globals().set(
        "__cryptoGenerateKeyPairSync",
        Function::new(ctx.clone(), |key_type: String, options_json: String| {
            do_generate_keypair_sync_inner(&key_type, &options_json)
                .map_err(|e| js_err("generateKeyPairSync", e))
        })?,
    )?;

    // Sync variants — block the calling thread (acceptable for short KDF calls)
    ctx.globals().set(
        "__cryptoPbkdf2Sync",
        Function::new(
            ctx.clone(),
            |password: Vec<u8>, salt: Vec<u8>, iterations: u32, keylen: usize, digest: String| {
                let mut out = vec![0u8; keylen];
                match norm_alg(&digest).as_str() {
                    "sha1" => pbkdf2_hmac::<Sha1>(&password, &salt, iterations, &mut out),
                    "sha224" => pbkdf2_hmac::<Sha224>(&password, &salt, iterations, &mut out),
                    "sha256" => pbkdf2_hmac::<Sha256>(&password, &salt, iterations, &mut out),
                    "sha384" => pbkdf2_hmac::<Sha384>(&password, &salt, iterations, &mut out),
                    "sha512" => pbkdf2_hmac::<Sha512>(&password, &salt, iterations, &mut out),
                    other => {
                        return Err(js_err(
                            "pbkdf2Sync",
                            anyhow::anyhow!("unsupported digest: {other}"),
                        ));
                    }
                }
                Ok(out)
            },
        )?,
    )?;

    ctx.eval::<(), _>(
        r#"
(function() {
    // Convert any input to a flat byte array.
    function toBytes(v) {
        if (v instanceof Uint8Array) return Array.from(v);
        if (Array.isArray(v)) return v;
        if (typeof v === 'string') {
            // UTF-8 encode
            var out = [];
            for (var i = 0; i < v.length; i++) {
                var c = v.charCodeAt(i);
                if (c < 0x80) {
                    out.push(c);
                } else if (c < 0x800) {
                    out.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
                } else if (c < 0xd800 || c >= 0xe000) {
                    out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
                } else {
                    // surrogate pair
                    i++;
                    var c2 = v.charCodeAt(i);
                    var cp = 0x10000 + ((c & 0x3ff) << 10) + (c2 & 0x3ff);
                    out.push(
                        0xf0 | (cp >> 18),
                        0x80 | ((cp >> 12) & 0x3f),
                        0x80 | ((cp >> 6) & 0x3f),
                        0x80 | (cp & 0x3f)
                    );
                }
            }
            return out;
        }
        // ArrayBuffer, Buffer, DataView, other TypedArrays
        return Array.from(new Uint8Array(v.buffer ? v.buffer : v));
    }

    // Encode raw bytes to the requested encoding string.
    var B64_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    function base64Encode(bytes, urlSafe) {
        var chars = urlSafe ? B64_CHARS.replace(/\+/g,'-').replace(/\//g,'_') : B64_CHARS;
        var out = '';
        var len = bytes.length;
        for (var i = 0; i < len; i += 3) {
            var b0 = bytes[i], b1 = i+1 < len ? bytes[i+1] : 0, b2 = i+2 < len ? bytes[i+2] : 0;
            out += chars[b0 >> 2];
            out += chars[((b0 & 3) << 4) | (b1 >> 4)];
            out += (i+1 < len) ? chars[((b1 & 15) << 2) | (b2 >> 6)] : (urlSafe ? '' : '=');
            out += (i+2 < len) ? chars[b2 & 63] : (urlSafe ? '' : '=');
        }
        return out;
    }
    function encodeBytes(bytes, encoding) {
        if (!encoding || encoding === 'buffer') return new Uint8Array(bytes);
        if (encoding === 'hex') {
            return bytes.map(function(b) {
                return ('0' + b.toString(16)).slice(-2);
            }).join('');
        }
        if (encoding === 'base64') return base64Encode(bytes, false);
        if (encoding === 'base64url') return base64Encode(bytes, true);
        return new Uint8Array(bytes);
    }

    // Rejection-sampled, unbiased random integer in [min, max).
    function secureRandomInt(min, max) {
        var range = max - min;
        var maxUnbiased = (0xFFFFFFFF - (0xFFFFFFFF % range)) >>> 0;
        var n;
        do {
            var b = __cryptoRandomBytes(4);
            n = ((b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3]) >>> 0;
        } while (n > maxUnbiased);
        return min + (n % range);
    }

    var crypto = {
        // ── Hashing ──────────────────────────────────────────────────────────

        createHash: function(alg) {
            var chunks = [];
            return {
                update: function(data /*, encoding */) {
                    chunks.push(toBytes(data));
                    return this;
                },
                digest: function(encoding) {
                    var all = [];
                    for (var i = 0; i < chunks.length; i++) {
                        all = all.concat(chunks[i]);
                    }
                    var raw = __cryptoHash(alg, all);
                    return encodeBytes(raw, encoding);
                },
                copy: function() {
                    throw new Error('Hash.copy() is not supported');
                }
            };
        },

        // Shorthand: crypto.hash(alg, data[, outputEncoding]) — Node 21.7+
        hash: function(alg, data, outputEncoding) {
            var raw = __cryptoHash(alg, toBytes(data));
            return encodeBytes(raw, outputEncoding || 'hex');
        },

        // ── HMAC ─────────────────────────────────────────────────────────────

        createHmac: function(alg, key) {
            var keyBytes = toBytes(key);
            var chunks = [];
            return {
                update: function(data /*, encoding */) {
                    chunks.push(toBytes(data));
                    return this;
                },
                digest: function(encoding) {
                    var all = [];
                    for (var i = 0; i < chunks.length; i++) {
                        all = all.concat(chunks[i]);
                    }
                    var raw = __cryptoHmac(alg, keyBytes, all);
                    return encodeBytes(raw, encoding);
                }
            };
        },

        // ── CSPRNG ───────────────────────────────────────────────────────────

        randomBytes: function(n, callback) {
            var bytes = new Uint8Array(__cryptoRandomBytes(n));
            if (typeof callback === 'function') {
                Promise.resolve().then(function() { callback(null, bytes); });
                return;
            }
            return bytes;
        },

        randomFill: function(buf, offset, size, callback) {
            if (typeof offset === 'function') { callback = offset; offset = 0; size = buf.length; }
            else if (typeof size === 'function') { callback = size; size = buf.length - offset; }
            var bytes = __cryptoRandomBytes(size);
            for (var i = 0; i < size; i++) buf[offset + i] = bytes[i];
            if (typeof callback === 'function') {
                Promise.resolve().then(function() { callback(null, buf); });
            }
            return buf;
        },

        randomFillSync: function(buf, offset, size) {
            offset = offset || 0;
            size = (size !== undefined) ? size : buf.length - offset;
            var bytes = __cryptoRandomBytes(size);
            for (var i = 0; i < size; i++) buf[offset + i] = bytes[i];
            return buf;
        },

        randomInt: function(min, max, callback) {
            if (typeof max === 'function') { callback = max; max = min; min = 0; }
            else if (max === undefined) { max = min; min = 0; }
            if (max <= min) throw new RangeError('max must be greater than min');
            if (max - min > 0xFFFFFFFF) throw new RangeError('range must not exceed 2^32');
            var result = secureRandomInt(min, max);
            if (typeof callback === 'function') {
                Promise.resolve().then(function() { callback(null, result); });
                return;
            }
            return result;
        },

        randomUUID: function() {
            var b = __cryptoRandomBytes(16);
            b[6] = (b[6] & 0x0f) | 0x40; // version 4
            b[8] = (b[8] & 0x3f) | 0x80; // variant RFC 4122
            var h = b.map(function(x) { return ('0' + x.toString(16)).slice(-2); }).join('');
            return h.slice(0,8)+'-'+h.slice(8,12)+'-'+h.slice(12,16)+'-'+h.slice(16,20)+'-'+h.slice(20);
        },

        getRandomValues: function(arr) {
            var bytes = __cryptoRandomBytes(arr.length);
            for (var i = 0; i < arr.length; i++) arr[i] = bytes[i];
            return arr;
        },

        // ── Password-based KDFs ───────────────────────────────────────────────

        pbkdf2: function(password, salt, iterations, keylen, digest, callback) {
            __cryptoPbkdf2(toBytes(password), toBytes(salt), iterations, keylen, digest)
                .then(function(raw) { callback(null, new Uint8Array(raw)); })
                .catch(function(err) { callback(err); });
        },

        pbkdf2Sync: function(password, salt, iterations, keylen, digest) {
            return new Uint8Array(__cryptoPbkdf2Sync(toBytes(password), toBytes(salt), iterations, keylen, digest || 'sha1'));
        },

        scrypt: function(password, salt, keylen, options, callback) {
            if (typeof options === 'function') { callback = options; options = {}; }
            options = options || {};
            var N = options.N || options.cost || 16384;
            var r = options.r || options.blockSize || 8;
            var p = options.p || options.parallelization || 1;
            __cryptoScrypt(toBytes(password), toBytes(salt), N, r, p, keylen)
                .then(function(raw) { callback(null, new Uint8Array(raw)); })
                .catch(function(err) { callback(err); });
        },

        scryptSync: function(password, salt, keylen, options) {
            options = options || {};
            var N = options.N || options.cost || 16384;
            var r = options.r || options.blockSize || 8;
            var p = options.p || options.parallelization || 1;
            return new Uint8Array(__cryptoScryptSync(toBytes(password), toBytes(salt), N, r, p, keylen));
        },

        // ── Cipher (AES-GCM backed, Node-compatible API shape) ────────────────
        // createCipheriv / createDecipheriv are used by many auth libraries.
        // We only support 'aes-128-gcm' and 'aes-256-gcm' since that's what
        // crypto.subtle can do natively.
        createCipheriv: function(algorithm, key, iv, options) {
            var alg = algorithm.toLowerCase().replace(/-/g, '');
            var keyBytes = toBytes(key);
            var ivBytes = toBytes(iv);
            var chunks = [];
            var aad = new Uint8Array(0);
            return {
                setAAD: function(buf) { aad = toBytes(buf); return this; },
                update: function(data) { chunks.push(toBytes(data)); return new Uint8Array(0); },
                final: function() {
                    var all = [];
                    for (var i = 0; i < chunks.length; i++) for (var j = 0; j < chunks[i].length; j++) all.push(chunks[i][j]);
                    var result = __cryptoAesGcmEncrypt(keyBytes.length, Array.from(keyBytes), Array.from(ivBytes), all, Array.from(aad));
                    // result = ciphertext + 16-byte tag; expose tag via getAuthTag
                    var ct = new Uint8Array(result.slice(0, result.length - 16));
                    this._tag = new Uint8Array(result.slice(result.length - 16));
                    return ct;
                },
                getAuthTag: function() { return this._tag || new Uint8Array(16); }
            };
        },

        createDecipheriv: function(algorithm, key, iv, options) {
            var keyBytes = toBytes(key);
            var ivBytes = toBytes(iv);
            var chunks = [];
            var aad = new Uint8Array(0);
            var authTag = null;
            return {
                setAAD: function(buf) { aad = toBytes(buf); return this; },
                setAuthTag: function(tag) { authTag = toBytes(tag); return this; },
                update: function(data) { chunks.push(toBytes(data)); return new Uint8Array(0); },
                final: function() {
                    var all = [];
                    for (var i = 0; i < chunks.length; i++) for (var j = 0; j < chunks[i].length; j++) all.push(chunks[i][j]);
                    var tag = authTag || new Uint8Array(16);
                    var ct_and_tag = all.concat(Array.from(tag));
                    var result = __cryptoAesGcmDecrypt(keyBytes.length, Array.from(keyBytes), Array.from(ivBytes), ct_and_tag, Array.from(aad));
                    return new Uint8Array(result);
                }
            };
        },

        // ── Key objects ──────────────────────────────────────────────────────
        // createPrivateKey / createPublicKey / createSecretKey return KeyObject-like
        // objects. These wrap a PEM/Buffer/string and expose .type, .asymmetricKeyType,
        // and .export(). The asymmetricKeyType is inferred from the PEM header:
        //   RSA: "rsa"   EC P-256/P-384: "ec"   symmetric: "secret"

        createPrivateKey: function(key) {
            var pem = typeof key === 'string' ? key : (key && key.key ? key.key : (key && typeof key.toString === 'function' ? key.toString() : String(key)));
            var kt = pem.indexOf('EC PRIVATE') !== -1 ? 'ec' : 'rsa';
            // Detect EC curve from PEM OID heuristics
            var curve = pem.indexOf('P-384') !== -1 || pem.indexOf('secp384r1') !== -1 ? 'P-384' : 'P-256';
            return {
                type: 'private', asymmetricKeyType: kt, _pem: pem, _curve: curve,
                export: function(opts) {
                    if (!opts || opts.format === 'pem') return pem;
                    // DER export: strip PEM armor and decode base64
                    var b64 = pem.replace(/-----[^-]+-----/g,'').replace(/\s/g,'');
                    var bin = atob(b64), bytes = new Uint8Array(bin.length);
                    for (var i=0;i<bin.length;i++) bytes[i]=bin.charCodeAt(i);
                    return bytes;
                },
                toString: function() { return pem; }
            };
        },

        createPublicKey: function(key) {
            var pem;
            if (typeof key === 'string') {
                pem = key;
            } else if (key && key.type === 'private') {
                // extracting public from private is complex without native; return stub
                pem = key._pem;
            } else {
                pem = key && typeof key.toString === 'function' ? key.toString() : String(key);
            }
            var kt = pem.indexOf('EC') !== -1 || pem.indexOf('BEGIN PUBLIC KEY') !== -1 ? 'rsa' : 'rsa';
            var curve = pem.indexOf('P-384') !== -1 || pem.indexOf('secp384r1') !== -1 ? 'P-384' : 'P-256';
            return {
                type: 'public', asymmetricKeyType: kt, _pem: pem, _curve: curve,
                export: function(opts) {
                    if (!opts || opts.format === 'pem') return pem;
                    var b64 = pem.replace(/-----[^-]+-----/g,'').replace(/\s/g,'');
                    var bin = atob(b64), bytes = new Uint8Array(bin.length);
                    for (var i=0;i<bin.length;i++) bytes[i]=bin.charCodeAt(i);
                    return bytes;
                },
                toString: function() { return pem; }
            };
        },

        createSecretKey: function(key, encoding) {
            var bytes = key instanceof Uint8Array ? key : toBytes(key);
            return {
                type: 'secret', symmetricKeySize: bytes.length, _raw: bytes,
                export: function() { return new Uint8Array(bytes); }
            };
        },

        // ── Sign / Verify — RSA PKCS1v15 + ECDSA ────────────────────────────
        // Algorithm name mapping (Node.js style → digest + key-type):
        //   'RSA-SHA256'  → rsa + sha256
        //   'SHA256'      → determined by key type at sign() time
        //   'SHA384'      → determined by key type
        //   'id-ecPublicKey' → ec

        createSign: function(algorithm) {
            var chunks = [];
            return {
                update: function(data, enc) { chunks.push(toBytes(data)); return this; },
                sign: function(key, outputEncoding) {
                    var all = [];
                    for (var i=0;i<chunks.length;i++) for (var j=0;j<chunks[i].length;j++) all.push(chunks[i][j]);
                    var data = new Uint8Array(all);

                    // Resolve key to {pem, keyType, curve}
                    var pem, keyType, curve;
                    if (key && key._pem) {
                        pem = key._pem; keyType = key.asymmetricKeyType; curve = key._curve;
                    } else if (key && typeof key.export === 'function') {
                        pem = key.export();
                        keyType = key.asymmetricKeyType || 'rsa';
                        curve = key._curve || (pem.indexOf('P-384') !== -1 ? 'P-384' : 'P-256');
                    } else if (typeof key === 'string' && key.indexOf('-----') !== -1) {
                        pem = key;
                        keyType = (key.indexOf('EC PRIVATE') !== -1) ? 'ec' : 'rsa';
                        curve = key.indexOf('P-384') !== -1 ? 'P-384' : 'P-256';
                    } else if (key && key.type === 'secret') {
                        var alg = algorithm.replace('RSA-','').replace('with','').toLowerCase();
                        var raw = __cryptoHmac(alg || 'sha256', Array.from(key._raw), Array.from(data));
                        return encodeBytes(raw, outputEncoding);
                    } else {
                        throw new TypeError('createSign.sign: unsupported key type');
                    }

                    var digestAlg = algorithm.replace(/^RSA-/i,'').replace(/withRSA$/i,'').toLowerCase() || 'sha256';
                    var raw;
                    if (keyType === 'ec') {
                        raw = __cryptoEcSign(curve, pem, Array.from(data));
                    } else {
                        raw = __cryptoRsaSign(digestAlg, pem, Array.from(data));
                    }
                    return encodeBytes(Array.from(raw), outputEncoding);
                }
            };
        },

        createVerify: function(algorithm) {
            var chunks = [];
            return {
                update: function(data, enc) { chunks.push(toBytes(data)); return this; },
                verify: function(key, signature, sigEncoding) {
                    var all = [];
                    for (var i=0;i<chunks.length;i++) for (var j=0;j<chunks[i].length;j++) all.push(chunks[i][j]);
                    var data = new Uint8Array(all);
                    var sigBytes;
                    if (typeof signature === 'string') {
                        sigBytes = toBytes(Buffer && Buffer.from ? Buffer.from(signature, sigEncoding || 'hex') : signature);
                    } else {
                        sigBytes = toBytes(signature);
                    }

                    var pem, keyType, curve;
                    if (key && key._pem) {
                        pem = key._pem; keyType = key.asymmetricKeyType; curve = key._curve;
                    } else if (key && typeof key.export === 'function') {
                        pem = key.export();
                        keyType = key.asymmetricKeyType || 'rsa';
                        curve = key._curve || (pem.indexOf('P-384') !== -1 ? 'P-384' : 'P-256');
                    } else if (typeof key === 'string' && key.indexOf('-----') !== -1) {
                        pem = key;
                        keyType = 'rsa';
                        curve = 'P-256';
                    } else if (key && key.type === 'secret') {
                        var alg = algorithm.replace('RSA-','').replace('with','').toLowerCase();
                        var expected = __cryptoHmac(alg || 'sha256', Array.from(key._raw), Array.from(data));
                        var actual = Array.from(sigBytes);
                        if (expected.length !== actual.length) return false;
                        for (var i=0;i<expected.length;i++) if (expected[i]!==actual[i]) return false;
                        return true;
                    } else {
                        throw new TypeError('createVerify.verify: unsupported key type');
                    }

                    var digestAlg = algorithm.replace(/^RSA-/i,'').replace(/withRSA$/i,'').toLowerCase() || 'sha256';
                    if (keyType === 'ec') {
                        return __cryptoEcVerify(curve, pem, Array.from(data), Array.from(sigBytes));
                    } else {
                        return __cryptoRsaVerify(digestAlg, pem, Array.from(data), Array.from(sigBytes));
                    }
                }
            };
        },

        // One-shot sign/verify (Node.js 15+)
        sign: function(algorithm, data, key, callback) {
            var signer = this.createSign(algorithm || 'sha256');
            signer.update(data);
            var sig = signer.sign(key);
            if (typeof callback === 'function') { Promise.resolve().then(function() { callback(null, sig); }); return; }
            return sig;
        },

        verify: function(algorithm, data, key, signature) {
            var verifier = this.createVerify(algorithm || 'sha256');
            verifier.update(data);
            return verifier.verify(key, signature);
        },

        // Enumerate supported algorithms
        getCiphers: function() { return ['aes-128-gcm','aes-256-gcm']; },
        getHashes: function() { return ['md5','sha1','sha224','sha256','sha384','sha512']; },
        getCurves: function() { return ['P-256','P-384','prime256v1','secp384r1']; },

        // ── Key pair generation ───────────────────────────────────────────────
        // Supports 'rsa', 'rsa-pss', 'ec'. Returns KeyObject-like objects with
        // .export() that returns PEM. Options mirror Node.js crypto.generateKeyPair.

        generateKeyPair: function(type, options, callback) {
            if (typeof options === 'function') { callback = options; options = {}; }
            options = options || {};
            var curve = options.namedCurve || 'P-256';
            __cryptoGenerateKeyPair(type, JSON.stringify(options)).then(function(raw) {
                var r = JSON.parse(raw);
                var makeKey = function(pem, keyType) {
                    return {
                        type: keyType, asymmetricKeyType: type,
                        _pem: pem, _curve: curve,
                        export: function(opts) { return pem; },
                        toString: function() { return pem; }
                    };
                };
                callback(null, makeKey(r.publicKeyPem, 'public'), makeKey(r.privateKeyPem, 'private'));
            }).catch(function(err) { callback(err); });
        },

        generateKeyPairSync: function(type, options) {
            options = options || {};
            var curve = options.namedCurve || 'P-256';
            var raw = JSON.parse(__cryptoGenerateKeyPairSync(type, JSON.stringify(options)));
            var makeKey = function(pem, keyType) {
                return {
                    type: keyType, asymmetricKeyType: type,
                    _pem: pem, _curve: curve,
                    export: function() { return pem; },
                    toString: function() { return pem; }
                };
            };
            return { publicKey: makeKey(raw.publicKeyPem, 'public'), privateKey: makeKey(raw.privateKeyPem, 'private') };
        },

        // ── Utilities ────────────────────────────────────────────────────────

        timingSafeEqual: function(a, b) {
            var ab = new Uint8Array(a.buffer || a, a.byteOffset || 0, a.byteLength !== undefined ? a.byteLength : a.length);
            var bb = new Uint8Array(b.buffer || b, b.byteOffset || 0, b.byteLength !== undefined ? b.byteLength : b.length);
            if (ab.byteLength !== bb.byteLength) {
                throw new Error('Input buffers must have the same byte length');
            }
            return __cryptoTimingSafeEqual(Array.from(ab), Array.from(bb));
        },

        constants: {
            POINT_CONVERSION_COMPRESSED: 2,
            POINT_CONVERSION_HYBRID: 3,
            POINT_CONVERSION_UNCOMPRESSED: 4,
        }
    };

    // ── Web Crypto (crypto.subtle) ────────────────────────────────────────────
    // Implements the subset of the W3C SubtleCrypto API used by most packages:
    //   digest, importKey, exportKey, generateKey,
    //   sign/verify (HMAC, RSASSA-PKCS1-v1_5, RSA-PSS, ECDSA),
    //   encrypt/decrypt (AES-GCM),
    //   deriveBits/deriveKey (HKDF, PBKDF2).
    // DOMException polyfill (QuickJS doesn't have it natively).
    if (typeof DOMException === 'undefined') {
        var DOMException = (function() {
            function DOMException(message, name) {
                this.message = message || '';
                this.name = name || 'Error';
                this.code = 0;
            }
            DOMException.prototype = Object.create(Error.prototype);
            DOMException.prototype.constructor = DOMException;
            DOMException.prototype.toString = function() { return this.name + ': ' + this.message; };
            return DOMException;
        })();
    }

    var subtle = (function() {
        // Normalise algorithm name to upper-case with hyphens.
        function normAlg(a) {
            return (typeof a === 'string' ? a : a.name).toUpperCase().replace(/_/g, '-');
        }
        function normHash(a) {
            return (typeof a === 'string' ? a : (a.hash ? normAlg(a.hash) : normAlg(a))).toUpperCase().replace(/_/g, '-');
        }

        // Raw-bytes helper: accepts ArrayBuffer, TypedArray or Array<number>.
        function rawBytes(v) {
            if (v instanceof ArrayBuffer) return new Uint8Array(v);
            if (ArrayBuffer.isView(v)) return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
            return new Uint8Array(v);
        }
        function toByteArray(v) { return Array.from(rawBytes(v)); }
        function toArrayBuffer(arr) {
            var u = new Uint8Array(arr);
            return u.buffer;
        }

        // DER <-> PEM conversion for asymmetric keys / signatures.
        var b64Chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
        var b64Lookup = {};
        for (var i = 0; i < 64; i++) b64Lookup[b64Chars[i]] = i;
        function base64Decode(s) {
            s = s.replace(/[^A-Za-z0-9\+\/=]/g, '');
            var bytes = [];
            for (var i = 0; i < s.length; i += 4) {
                var a = b64Lookup[s[i] || 'A'] || 0;
                var b = b64Lookup[s[i+1] || 'A'] || 0;
                var c = b64Lookup[s[i+2] || 'A'] || 0;
                var d = b64Lookup[s[i+3] || 'A'] || 0;
                bytes.push((a << 2) | (b >> 4));
                if (s[i+2] !== '=') bytes.push(((b & 0xf) << 4) | (c >> 2));
                if (s[i+3] !== '=') bytes.push(((c & 0x3) << 6) | d);
            }
            return new Uint8Array(bytes);
        }
        function base64Encode(bytes) {
            var s = '';
            for (var i = 0; i < bytes.length; i += 3) {
                var a = bytes[i], b = bytes[i+1] || 0, c = bytes[i+2] || 0;
                s += b64Chars[a >> 2] + b64Chars[((a & 3) << 4) | (b >> 4)];
                s += i + 1 < bytes.length ? b64Chars[((b & 0xf) << 2) | (c >> 6)] : '=';
                s += i + 2 < bytes.length ? b64Chars[c & 0x3f] : '=';
            }
            return s;
        }
        function derToPem(der, label) {
            var b64 = base64Encode(new Uint8Array(der));
            var lines = b64.match(/.{1,64}/g) || [b64];
            return '-----BEGIN ' + label + '-----\n' + lines.join('\n') + '\n-----END ' + label + '-----\n';
        }
        function pemToDer(pem) {
            var b64 = pem.replace(/-----[^-]+-----/g, '').replace(/\n/g, '');
            return base64Decode(b64);
        }

        // Convert DER SEQUENCE { INTEGER r, INTEGER s } to raw r||s (fixed-width).
        function ecdsaDerToRaw(der, fieldSize) {
            var view = new Uint8Array(der);
            var i = 0;
            if (view[i] !== 0x30) throw new Error('ECDSA: expected SEQUENCE');
            i++;
            // SEQUENCE length (skip if long-form)
            if (view[i] & 0x80) { i += (view[i] & 0x7f) + 1; } else { i++; }
            function readInt() {
                if (view[i] !== 0x02) throw new Error('ECDSA: expected INTEGER');
                i++;
                var len = view[i]; i++;
                // Strip leading zero if present
                while (len > 0 && view[i] === 0) { i++; len--; }
                var val = view.slice(i, i + len);
                i += len;
                return val;
            }
            var rArr = readInt();
            var sArr = readInt();
            function padTo(v, size) {
                if (v.length >= size) return v.slice(v.length - size);
                var p = new Uint8Array(size);
                p.set(v, size - v.length);
                return p;
            }
            var out = new Uint8Array(fieldSize * 2);
            out.set(padTo(rArr, fieldSize), 0);
            out.set(padTo(sArr, fieldSize), fieldSize);
            return out;
        }

        // Map Web Crypto hash name to Node.js algorithm string.
        var hashToNode = { 'SHA-1': 'sha1', 'SHA-224': 'sha224', 'SHA-256': 'sha256', 'SHA-384': 'sha384', 'SHA-512': 'sha512' };

        // Resolve hash from algorithm object or key.algorithm for asymmetric keys.
        function resolveHash(algorithm, key) {
            var h = normHash((algorithm && algorithm.hash) || (key && key.algorithm && key.algorithm.hash) || 'SHA-256');
            return hashToNode[h] || (function() { throw new DOMException('Unsupported hash: ' + h, 'NotSupportedError'); })();
        }

        // Internal CryptoKey representation.
        function CryptoKey(type, extractable, algorithm, usages, raw) {
            this.type = type;          // "secret" | "public" | "private"
            this.extractable = extractable;
            this.algorithm = algorithm; // {name, ...}
            this.usages = usages;
            this._raw = raw;           // Uint8Array of raw key bytes (symmetric) or null
            this._pem = null;          // PEM string for asymmetric keys
            this._asymmetricKeyType = null; // "rsa" | "ec"
            this._curve = null;        // EC curve name for ECDSA
        }

        return {
            // ── digest ───────────────────────────────────────────────────────
            digest: function(algorithm, data) {
                var alg = normAlg(algorithm);
                var nodeAlg = hashToNode[alg];
                if (!nodeAlg) return Promise.reject(new DOMException('Unsupported algorithm: ' + alg, 'NotSupportedError'));
                try {
                    var raw = __cryptoHash(nodeAlg, toByteArray(data));
                    return Promise.resolve(toArrayBuffer(raw));
                } catch(e) { return Promise.reject(e); }
            },

            // ── generateKey ──────────────────────────────────────────────────
            generateKey: function(algorithm, extractable, keyUsages) {
                var alg = normAlg(algorithm);
                try {
                    if (alg === 'AES-GCM' || alg === 'AES-CBC' || alg === 'AES-CTR') {
                        var len = (algorithm.length || 256) / 8;
                        var raw = new Uint8Array(__cryptoRandomBytes(len));
                        var key = new CryptoKey('secret', extractable,
                            { name: alg, length: algorithm.length || 256 },
                            keyUsages, raw);
                        return Promise.resolve(key);
                    }
                    if (alg === 'HMAC') {
                        var hash = normHash(algorithm.hash || 'SHA-256');
                        var blen = algorithm.length || ({ 'SHA-1': 160, 'SHA-256': 256, 'SHA-384': 384, 'SHA-512': 512 }[hash] || 256);
                        var raw = new Uint8Array(__cryptoRandomBytes(blen / 8));
                        var key = new CryptoKey('secret', extractable,
                            { name: 'HMAC', hash: { name: hash } },
                            keyUsages, raw);
                        return Promise.resolve(key);
                    }
                    if (alg === 'RSASSA-PKCS1-V1-5' || alg === 'RSA-PSS') {
                        var hashName = normHash(algorithm.hash || 'SHA-256');
                        var nodeHash = hashToNode[hashName];
                        if (!nodeHash) return Promise.reject(new DOMException('Unsupported hash: ' + hashName, 'NotSupportedError'));
                        var modulusLength = algorithm.modulusLength || 2048;
                        var opts = JSON.stringify({ modulusLength: modulusLength });
                        var result = JSON.parse(__cryptoGenerateKeyPairSync('rsa', opts));
                        var pubUsages = [], privUsages = [];
                        for (var i = 0; i < keyUsages.length; i++) {
                            if (keyUsages[i] === 'verify' || keyUsages[i] === 'encrypt' || keyUsages[i] === 'wrapKey') pubUsages.push(keyUsages[i]);
                            if (keyUsages[i] === 'sign' || keyUsages[i] === 'decrypt' || keyUsages[i] === 'unwrapKey') privUsages.push(keyUsages[i]);
                        }
                        var keyAlg = { name: alg, hash: { name: hashName }, modulusLength: modulusLength, publicExponent: new Uint8Array([1,0,1]) };
                        var publicKey = new CryptoKey('public', true, keyAlg, pubUsages, null);
                        publicKey._pem = result.publicKeyPem;
                        publicKey._asymmetricKeyType = 'rsa';
                        var privateKey = new CryptoKey('private', extractable, keyAlg, privUsages, null);
                        privateKey._pem = result.privateKeyPem;
                        privateKey._asymmetricKeyType = 'rsa';
                        return Promise.resolve({ publicKey: publicKey, privateKey: privateKey });
                    }
                    if (alg === 'ECDSA') {
                        var namedCurve = algorithm.namedCurve || 'P-256';
                        var opts = JSON.stringify({ namedCurve: namedCurve });
                        var result = JSON.parse(__cryptoGenerateKeyPairSync('ec', opts));
                        var pubUsages = [], privUsages = [];
                        for (var i = 0; i < keyUsages.length; i++) {
                            if (keyUsages[i] === 'verify') pubUsages.push(keyUsages[i]);
                            if (keyUsages[i] === 'sign') privUsages.push(keyUsages[i]);
                        }
                        var hashName = normHash(algorithm.hash || 'SHA-256');
                        var keyAlg = { name: 'ECDSA', namedCurve: namedCurve, hash: { name: hashName } };
                        var publicKey = new CryptoKey('public', true, keyAlg, pubUsages, null);
                        publicKey._pem = result.publicKeyPem;
                        publicKey._asymmetricKeyType = 'ec';
                        publicKey._curve = namedCurve;
                        var privateKey = new CryptoKey('private', extractable, keyAlg, privUsages, null);
                        privateKey._pem = result.privateKeyPem;
                        privateKey._asymmetricKeyType = 'ec';
                        privateKey._curve = namedCurve;
                        return Promise.resolve({ publicKey: publicKey, privateKey: privateKey });
                    }
                    return Promise.reject(new DOMException('generateKey: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── importKey ────────────────────────────────────────────────────
            importKey: function(format, keyData, algorithm, extractable, keyUsages) {
                try {
                    var alg = normAlg(algorithm);
                    var isAsymmetric = (alg === 'RSASSA-PKCS1-V1-5' || alg === 'RSA-PSS' || alg === 'ECDSA');
                    if (isAsymmetric) {
                        var keyType = (format === 'pkcs8' ? 'private' : 'public');
                        var der = rawBytes(keyData);
                        var pemLabel = (keyType === 'private' ? 'PRIVATE KEY' : 'PUBLIC KEY');
                        var pem = derToPem(der, pemLabel);
                        var keyAlg;
                        var curve = null;
                        var asymType = 'rsa';
                        if (alg === 'ECDSA') {
                            asymType = 'ec';
                            curve = algorithm.namedCurve || (format === 'pkcs8' ? 'P-256' : 'P-256');
                            keyAlg = { name: 'ECDSA', namedCurve: curve, hash: algorithm.hash ? { name: normHash(algorithm.hash) } : { name: 'SHA-256' } };
                        } else {
                            var hashName = normHash(algorithm.hash || 'SHA-256');
                            keyAlg = { name: alg, hash: { name: hashName }, modulusLength: algorithm.modulusLength || 2048 };
                        }
                        var key = new CryptoKey(keyType, extractable, keyAlg, keyUsages, null);
                        key._pem = pem;
                        key._asymmetricKeyType = asymType;
                        key._curve = curve;
                        return Promise.resolve(key);
                    }
                    // Symmetric key import
                    var raw;
                    if (format === 'raw') {
                        raw = rawBytes(keyData);
                    } else if (format === 'jwk') {
                        if (!keyData.k) return Promise.reject(new DOMException('importKey JWK: missing k field', 'DataError'));
                        var b64 = keyData.k.replace(/-/g,'+').replace(/_/g,'/');
                        while (b64.length % 4) b64 += '=';
                        var bin = atob(b64);
                        raw = new Uint8Array(bin.length);
                        for (var i = 0; i < bin.length; i++) raw[i] = bin.charCodeAt(i);
                    } else {
                        return Promise.reject(new DOMException('importKey: unsupported format ' + format, 'NotSupportedError'));
                    }
                    var keyAlg;
                    if (alg === 'HMAC') {
                        var hash = normHash(algorithm.hash || 'SHA-256');
                        keyAlg = { name: 'HMAC', hash: { name: hash } };
                    } else if (alg === 'AES-GCM' || alg === 'AES-CBC' || alg === 'AES-CTR') {
                        keyAlg = { name: alg, length: raw.length * 8 };
                    } else if (alg === 'HKDF') {
                        keyAlg = { name: 'HKDF' };
                    } else if (alg === 'PBKDF2') {
                        keyAlg = { name: 'PBKDF2' };
                    } else {
                        return Promise.reject(new DOMException('importKey: unsupported algorithm ' + alg, 'NotSupportedError'));
                    }
                    var key = new CryptoKey('secret', extractable, keyAlg, keyUsages, raw);
                    return Promise.resolve(key);
                } catch(e) { return Promise.reject(e); }
            },

            // ── exportKey ────────────────────────────────────────────────────
            exportKey: function(format, key) {
                if (!key.extractable) return Promise.reject(new DOMException('key is not extractable', 'InvalidAccessError'));
                if (key._asymmetricKeyType) {
                    if (format === 'spki' && key.type === 'public') {
                        var der = pemToDer(key._pem);
                        return Promise.resolve(der.buffer.slice(der.byteOffset, der.byteOffset + der.byteLength));
                    }
                    if (format === 'pkcs8' && key.type === 'private') {
                        var der = pemToDer(key._pem);
                        return Promise.resolve(der.buffer.slice(der.byteOffset, der.byteOffset + der.byteLength));
                    }
                    if (format === 'jwk') {
                        // Minimal JWK for RSA/EC public keys
                        var der = pemToDer(key._pem);
                        if (key._asymmetricKeyType === 'ec') {
                            return Promise.resolve({ kty: 'EC', crv: key._curve || 'P-256', key_ops: key.usages, ext: true });
                        }
                        return Promise.resolve({ kty: 'RSA', key_ops: key.usages, ext: true });
                    }
                    return Promise.reject(new DOMException('exportKey: unsupported format ' + format + ' for key type ' + key.type, 'NotSupportedError'));
                }
                if (format === 'raw') return Promise.resolve(key._raw.buffer.slice(key._raw.byteOffset, key._raw.byteOffset + key._raw.byteLength));
                if (format === 'jwk') {
                    var b64 = btoa(String.fromCharCode.apply(null, key._raw)).replace(/\+/g,'-').replace(/\//g,'_').replace(/=/g,'');
                    return Promise.resolve({ kty: 'oct', k: b64, alg: 'HS256', key_ops: key.usages, ext: true });
                }
                return Promise.reject(new DOMException('exportKey: unsupported format ' + format, 'NotSupportedError'));
            },

            // ── sign ─────────────────────────────────────────────────────────
            sign: function(algorithm, key, data) {
                var alg = normAlg(algorithm);
                try {
                    if (alg === 'HMAC') {
                        var hash = normHash(algorithm.hash || key.algorithm.hash || 'SHA-256');
                        var nodeAlg = hashToNode[hash];
                        if (!nodeAlg) return Promise.reject(new DOMException('Unsupported HMAC hash: ' + hash, 'NotSupportedError'));
                        var raw = __cryptoHmac(nodeAlg, Array.from(key._raw), toByteArray(data));
                        return Promise.resolve(toArrayBuffer(raw));
                    }
                    if (alg === 'RSASSA-PKCS1-V1-5' || alg === 'RSA-PSS') {
                        if (key.type !== 'private') return Promise.reject(new DOMException('sign requires private key', 'InvalidAccessError'));
                        var nodeHash = resolveHash(algorithm, key);
                        var sig = __cryptoRsaSign(nodeHash, key._pem, toByteArray(data));
                        return Promise.resolve(toArrayBuffer(sig));
                    }
                    if (alg === 'ECDSA') {
                        if (key.type !== 'private') return Promise.reject(new DOMException('sign requires private key', 'InvalidAccessError'));
                        var curve = key._curve || algorithm.namedCurve || 'P-256';
                        var der = __cryptoEcSign(curve, key._pem, toByteArray(data));
                        var fieldSize = (curve.indexOf('384') !== -1 || curve.indexOf('521') !== -1) ? (curve.indexOf('521') !== -1 ? 66 : 48) : 32;
                        var raw = ecdsaDerToRaw(der, fieldSize);
                        return Promise.resolve(toArrayBuffer(raw));
                    }
                    return Promise.reject(new DOMException('sign: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── verify ───────────────────────────────────────────────────────
            verify: function(algorithm, key, signature, data) {
                var alg = normAlg(algorithm);
                try {
                    if (alg === 'HMAC') {
                        var self = this;
                        return self.sign(algorithm, key, data).then(function(expected) {
                            var e = new Uint8Array(expected);
                            var s = rawBytes(signature);
                            if (e.length !== s.length) return false;
                            var diff = 0;
                            for (var i = 0; i < e.length; i++) diff |= e[i] ^ s[i];
                            return diff === 0;
                        });
                    }
                    if (alg === 'RSASSA-PKCS1-V1-5' || alg === 'RSA-PSS') {
                        if (key.type !== 'public') return Promise.reject(new DOMException('verify requires public key', 'InvalidAccessError'));
                        var nodeHash = resolveHash(algorithm, key);
                        var ok = __cryptoRsaVerify(nodeHash, key._pem, toByteArray(data), toByteArray(signature));
                        return Promise.resolve(ok);
                    }
                    if (alg === 'ECDSA') {
                        if (key.type !== 'public') return Promise.reject(new DOMException('verify requires public key', 'InvalidAccessError'));
                        var curve = key._curve || algorithm.namedCurve || 'P-256';
                        // Accept both raw r||s and DER signatures
                        var sigBytes = toByteArray(signature);
                        var ok = __cryptoEcVerify(curve, key._pem, toByteArray(data), sigBytes);
                        if (ok) return Promise.resolve(true);
                        // If raw, try wrapping in DER
                        var fieldSize = (curve.indexOf('384') !== -1 || curve.indexOf('521') !== -1) ? (curve.indexOf('521') !== -1 ? 66 : 48) : 32;
                        if (sigBytes.length === fieldSize * 2) {
                            // Build DER: SEQUENCE { INTEGER r, INTEGER s }
                            function intToDer(v) {
                                while (v[0] === 0) v = v.slice(1);
                                if (v[0] & 0x80) { var p = new Uint8Array(v.length + 1); p.set(v, 1); v = p; }
                                var tag = new Uint8Array([0x02, v.length]);
                                var out = new Uint8Array(tag.length + v.length);
                                out.set(tag); out.set(v, tag.length);
                                return out;
                            }
                            var rDer = intToDer(sigBytes.slice(0, fieldSize));
                            var sDer = intToDer(sigBytes.slice(fieldSize));
                            var seqContent = new Uint8Array(rDer.length + sDer.length);
                            seqContent.set(rDer); seqContent.set(sDer, rDer.length);
                            var seq = new Uint8Array(2 + seqContent.length);
                            seq[0] = 0x30;
                            if (seqContent.length > 127) { seq[1] = 0x81; seq[2] = seqContent.length; /* skip */ }
                            else { seq[1] = seqContent.length; }
                            seq.set(seqContent, 2);
                            ok = __cryptoEcVerify(curve, key._pem, toByteArray(data), Array.from(seq));
                        }
                        return Promise.resolve(ok);
                    }
                    return Promise.reject(new DOMException('verify: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── encrypt ──────────────────────────────────────────────────────
            encrypt: function(algorithm, key, data) {
                var alg = normAlg(algorithm);
                try {
                    if (alg === 'AES-GCM') {
                        var iv = toByteArray(algorithm.iv);
                        var aad = algorithm.additionalData ? toByteArray(algorithm.additionalData) : [];
                        var keyLen = key._raw.length;
                        var ct = __cryptoAesGcmEncrypt(keyLen, Array.from(key._raw), iv, toByteArray(data), aad);
                        return Promise.resolve(toArrayBuffer(ct));
                    }
                    return Promise.reject(new DOMException('encrypt: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── decrypt ──────────────────────────────────────────────────────
            decrypt: function(algorithm, key, data) {
                var alg = normAlg(algorithm);
                try {
                    if (alg === 'AES-GCM') {
                        var iv = toByteArray(algorithm.iv);
                        var aad = algorithm.additionalData ? toByteArray(algorithm.additionalData) : [];
                        var keyLen = key._raw.length;
                        var pt = __cryptoAesGcmDecrypt(keyLen, Array.from(key._raw), iv, toByteArray(data), aad);
                        return Promise.resolve(toArrayBuffer(pt));
                    }
                    return Promise.reject(new DOMException('decrypt: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── deriveBits ───────────────────────────────────────────────────
            deriveBits: function(algorithm, baseKey, length) {
                var alg = normAlg(algorithm);
                try {
                    if (alg === 'HKDF') {
                        var hash = normHash(algorithm.hash || 'SHA-256');
                        var map = { 'SHA-1': 'sha1', 'SHA-256': 'sha256', 'SHA-384': 'sha384', 'SHA-512': 'sha512' };
                        var nodeAlg = map[hash];
                        if (!nodeAlg) return Promise.reject(new DOMException('HKDF: unsupported hash ' + hash, 'NotSupportedError'));
                        var salt = algorithm.salt ? toByteArray(algorithm.salt) : [];
                        var info = algorithm.info ? toByteArray(algorithm.info) : [];
                        var ikm = Array.from(baseKey._raw);
                        // HKDF-Extract
                        var saltBytes = salt.length ? salt : new Array(({ 'sha1': 20, 'sha256': 32, 'sha384': 48, 'sha512': 64 }[nodeAlg] || 32)).fill(0);
                        var prk = Array.from(__cryptoHmac(nodeAlg, saltBytes, ikm));
                        // HKDF-Expand
                        var hashLen = prk.length;
                        var n = Math.ceil(length / 8 / hashLen);
                        var okm = [];
                        var t = [];
                        for (var i = 1; i <= n; i++) {
                            var input = t.concat(info).concat([i]);
                            t = Array.from(__cryptoHmac(nodeAlg, prk, input));
                            okm = okm.concat(t);
                        }
                        return Promise.resolve(toArrayBuffer(okm.slice(0, length / 8)));
                    }
                    if (alg === 'PBKDF2') {
                        var hash = normHash(algorithm.hash || 'SHA-256');
                        var salt = toByteArray(algorithm.salt);
                        var iterations = algorithm.iterations || 100000;
                        return __cryptoPbkdf2(Array.from(baseKey._raw), salt, iterations, length / 8, hash)
                            .then(function(raw) { return toArrayBuffer(raw); });
                    }
                    return Promise.reject(new DOMException('deriveBits: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── deriveKey ────────────────────────────────────────────────────
            deriveKey: function(algorithm, baseKey, derivedKeyAlg, extractable, keyUsages) {
                var self = this;
                var targetAlg = normAlg(derivedKeyAlg);
                var keyLenBits = derivedKeyAlg.length || 256;
                return self.deriveBits(algorithm, baseKey, keyLenBits).then(function(bits) {
                    return self.importKey('raw', bits, derivedKeyAlg, extractable, keyUsages);
                });
            },

            // ── wrapKey / unwrapKey (stubs) ───────────────────────────────────
            wrapKey: function() { return Promise.reject(new DOMException('wrapKey not implemented', 'NotSupportedError')); },
            unwrapKey: function() { return Promise.reject(new DOMException('unwrapKey not implemented', 'NotSupportedError')); },
        };
    })();

    crypto.subtle = subtle;
    crypto.webcrypto = { subtle: subtle };
    globalThis.crypto = { subtle: subtle, getRandomValues: crypto.getRandomValues, randomUUID: crypto.randomUUID };

    if (globalThis.__requireCache) {
        globalThis.__requireCache['crypto'] = crypto;
        globalThis.__requireCache['node:crypto'] = crypto;
    }
})();
        "#,
    )?;

    // ── Post-quantum crypto (ML-KEM-768, ML-DSA-65) ───────────────────────────

    // __pqKemGenerateKeypair() → { encapsulationKey: hex, decapsulationKey: hex }
    ctx.globals().set(
        "__pqKemGenerateKeypair",
        Function::new(ctx.clone(), || -> rquickjs::Result<String> {
            let kp = pq::kem::MlKemKeypair::generate();
            let ek = kp.encapsulation_key_hex();
            let dk = kp.decapsulation_key_hex();
            Ok(serde_json::json!({ "encapsulationKey": ek, "decapsulationKey": dk }).to_string())
        })?,
    )?;

    // __pqKemEncapsulate(encapsulationKeyHex) → { ciphertext: hex, sharedSecret: hex }
    ctx.globals().set(
        "__pqKemEncapsulate",
        Function::new(ctx.clone(), |ek_hex: String| -> rquickjs::Result<String> {
            let ek = pq::encapsulation_key_from_hex(&ek_hex).map_err(|e| {
                rquickjs::Error::new_from_js_message("crypto", "pqKemEncapsulate", &e.to_string())
            })?;
            let (ct, ss) = pq::encapsulate(&ek);
            let ct_hex = ct.to_hex();
            let ss_hex = hex::encode(ss.0);
            Ok(serde_json::json!({ "ciphertext": ct_hex, "sharedSecret": ss_hex }).to_string())
        })?,
    )?;

    // __pqKemDecapsulate(decapsulationKeyHex, ciphertextHex) → sharedSecretHex
    ctx.globals().set(
        "__pqKemDecapsulate",
        Function::new(
            ctx.clone(),
            |dk_hex: String, ct_hex: String| -> rquickjs::Result<String> {
                let dk = pq::decapsulation_key_from_hex(&dk_hex).map_err(|e| {
                    rquickjs::Error::new_from_js_message(
                        "crypto",
                        "pqKemDecapsulate",
                        &e.to_string(),
                    )
                })?;
                let ct = pq::MlKemCiphertext::from_hex(&ct_hex).map_err(|e| {
                    rquickjs::Error::new_from_js_message(
                        "crypto",
                        "pqKemDecapsulate",
                        &e.to_string(),
                    )
                })?;
                let ss = pq::decapsulate(&dk, &ct);
                Ok(hex::encode(ss.0))
            },
        )?,
    )?;

    // __pqDsaGenerateKeypair() → { signingKey: hex, verifyingKey: hex }
    ctx.globals().set(
        "__pqDsaGenerateKeypair",
        Function::new(ctx.clone(), || -> rquickjs::Result<String> {
            let (sk_hex, vk_hex) = pq::generate_keypair_hex();
            Ok(serde_json::json!({ "signingKey": sk_hex, "verifyingKey": vk_hex }).to_string())
        })?,
    )?;

    // __pqDsaSign(signingKeyHex, messageHex) → signatureHex
    ctx.globals().set(
        "__pqDsaSign",
        Function::new(
            ctx.clone(),
            |sk_hex: String, msg_hex: String| -> rquickjs::Result<String> {
                let sk = pq::signing_key_from_hex(&sk_hex).map_err(|e| {
                    rquickjs::Error::new_from_js_message("crypto", "pqDsaSign", &e.to_string())
                })?;
                let msg = hex::decode(&msg_hex).map_err(|e| {
                    rquickjs::Error::new_from_js_message("crypto", "pqDsaSign", &e.to_string())
                })?;
                let sig_bytes = pq::sign(&sk, &msg);
                Ok(hex::encode(&sig_bytes))
            },
        )?,
    )?;

    // __pqDsaVerify(verifyingKeyHex, messageHex, signatureHex) → bool
    ctx.globals().set(
        "__pqDsaVerify",
        Function::new(
            ctx.clone(),
            |vk_hex: String, msg_hex: String, sig_hex: String| -> rquickjs::Result<bool> {
                let vk = pq::verifying_key_from_hex(&vk_hex).map_err(|e| {
                    rquickjs::Error::new_from_js_message("crypto", "pqDsaVerify", &e.to_string())
                })?;
                let msg = hex::decode(&msg_hex).map_err(|e| {
                    rquickjs::Error::new_from_js_message("crypto", "pqDsaVerify", &e.to_string())
                })?;
                let sig_bytes = hex::decode(&sig_hex).map_err(|e| {
                    rquickjs::Error::new_from_js_message("crypto", "pqDsaVerify", &e.to_string())
                })?;
                Ok(pq::verify(&vk, &msg, &sig_bytes).is_ok())
            },
        )?,
    )?;

    // Inject PQ crypto JS wrapper into the crypto module
    ctx.eval::<(), _>(
        r#"
(function() {
    var _pq = globalThis.__pqCrypto || {};

    _pq.kem = {
        generateKeypair: function() {
            return JSON.parse(__pqKemGenerateKeypair());
        },
        encapsulate: function(encapsulationKeyHex) {
            return JSON.parse(__pqKemEncapsulate(encapsulationKeyHex));
        },
        decapsulate: function(decapsulationKeyHex, ciphertextHex) {
            return __pqKemDecapsulate(decapsulationKeyHex, ciphertextHex);
        },
    };

    _pq.dsa = {
        generateKeypair: function() {
            return JSON.parse(__pqDsaGenerateKeypair());
        },
        sign: function(signingKeyHex, messageHex) {
            return __pqDsaSign(signingKeyHex, messageHex);
        },
        verify: function(verifyingKeyHex, messageHex, signatureHex) {
            return __pqDsaVerify(verifyingKeyHex, messageHex, signatureHex);
        },
    };

    globalThis.__pqCrypto = _pq;

    // Also expose under require('crypto').pq and require('node:crypto').pq
    if (globalThis.__requireCache) {
        var c = globalThis.__requireCache['crypto'];
        if (c) c.pq = _pq;
        var nc = globalThis.__requireCache['node:crypto'];
        if (nc) nc.pq = _pq;
    }
})();
    "#,
    )?;

    Ok(())
}
