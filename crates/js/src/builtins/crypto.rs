use crate::builtins::v8_compat::{uint8array_from_bytes, uint8array_to_vec};
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

fn do_pbkdf2_sync(
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

fn do_cipher_one_shot(
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

async fn do_scrypt(
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

pub fn inject_crypto(scope: &mut v8::ContextScope<v8::HandleScope>) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    let crypto_hash = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let alg = args.get(0).to_rust_string_lossy(_scope);
            let data_arg = args.get(1);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else if let Ok(arr) = v8::Local::<v8::Array>::try_from(data_arg) {
                (0..arr.length())
                    .map(|i| {
                        arr.get_index(_scope, i)
                            .and_then(|v| v.uint32_value(_scope))
                            .unwrap_or(0) as u8
                    })
                    .collect()
            } else {
                Vec::new()
            };
            match do_hash(alg, data) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoHash").unwrap().into(),
        crypto_hash.unwrap().into(),
    );

    let crypto_hmac = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let alg = args.get(0).to_rust_string_lossy(_scope);
            let key_arg = args.get(1);
            let key: Vec<u8> = v8::Local::<v8::Uint8Array>::try_from(key_arg)
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let data_arg = args.get(2);
            let data: Vec<u8> = v8::Local::<v8::Uint8Array>::try_from(data_arg)
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_hmac(alg, key, data) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoHmac").unwrap().into(),
        crypto_hmac.unwrap().into(),
    );

    let crypto_random_bytes = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let n = args.get(0).uint32_value(_scope).unwrap_or(0) as usize;
            let bytes = do_random_bytes(n);
            rv.set(uint8array_from_bytes(_scope, &bytes).into());
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoRandomBytes")
            .unwrap()
            .into(),
        crypto_random_bytes.unwrap().into(),
    );

    let crypto_timing_safe_equal = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let a_arg = args.get(0);
            let b_arg = args.get(1);
            let a: Vec<u8> = a_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let b: Vec<u8> = b_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            rv.set(v8::Boolean::new(_scope, do_timing_safe_equal(a, b)).into());
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoTimingSafeEqual")
            .unwrap()
            .into(),
        crypto_timing_safe_equal.unwrap().into(),
    );

    let crypto_pbkdf2 = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let password_arg = args.get(0);
            let salt_arg = args.get(1);
            let iterations = args.get(2).uint32_value(_scope).unwrap_or(0);
            let keylen = args.get(3).uint32_value(_scope).unwrap_or(0) as usize;
            let digest = args.get(4).to_rust_string_lossy(_scope);
            let password: Vec<u8> = password_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let salt: Vec<u8> = salt_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            std::thread::spawn(move || {
                let _result = do_pbkdf2(password, salt, iterations, keylen, digest);
            });
            rv.set(v8::undefined(_scope).into());
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoPbkdf2").unwrap().into(),
        crypto_pbkdf2.unwrap().into(),
    );

    let crypto_pbkdf2_sync = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let password_arg = args.get(0);
            let salt_arg = args.get(1);
            let iterations = args.get(2).uint32_value(_scope).unwrap_or(0);
            let keylen = args.get(3).uint32_value(_scope).unwrap_or(0) as usize;
            let digest = args.get(4).to_rust_string_lossy(_scope);
            let password: Vec<u8> = password_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let salt: Vec<u8> = salt_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_pbkdf2_sync(password, salt, iterations, keylen, digest) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoPbkdf2Sync").unwrap().into(),
        crypto_pbkdf2_sync.unwrap().into(),
    );

    let crypto_cipher_one_shot = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let alg = args.get(0).to_rust_string_lossy(_scope);
            let key_arg = args.get(1);
            let iv_arg = args.get(2);
            let data_arg = args.get(3);
            let encrypt = args.get(4).boolean_value(_scope);
            let key: Vec<u8> = key_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let iv: Vec<u8> = iv_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let data: Vec<u8> = data_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_cipher_one_shot(&alg, &key, &iv, &data, encrypt) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoCipherOneShot")
            .unwrap()
            .into(),
        crypto_cipher_one_shot.unwrap().into(),
    );

    let crypto_aes_gcm_encrypt = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let key_len = args.get(0).uint32_value(_scope).unwrap_or(0) as usize;
            let key_arg = args.get(1);
            let iv_arg = args.get(2);
            let plaintext_arg = args.get(3);
            let aad_arg = args.get(4);
            let key: Vec<u8> = key_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let iv: Vec<u8> = iv_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let plaintext: Vec<u8> = plaintext_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let aad: Vec<u8> = aad_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_aes_gcm_encrypt(key_len, key, iv, plaintext, aad) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoAesGcmEncrypt")
            .unwrap()
            .into(),
        crypto_aes_gcm_encrypt.unwrap().into(),
    );

    let crypto_aes_gcm_decrypt = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let key_len = args.get(0).uint32_value(_scope).unwrap_or(0) as usize;
            let key_arg = args.get(1);
            let iv_arg = args.get(2);
            let ciphertext_arg = args.get(3);
            let aad_arg = args.get(4);
            let key: Vec<u8> = key_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let iv: Vec<u8> = iv_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let ciphertext: Vec<u8> = ciphertext_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let aad: Vec<u8> = aad_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_aes_gcm_decrypt(key_len, key, iv, ciphertext, aad) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoAesGcmDecrypt")
            .unwrap()
            .into(),
        crypto_aes_gcm_decrypt.unwrap().into(),
    );

    let crypto_scrypt = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let password_arg = args.get(0);
            let salt_arg = args.get(1);
            let n = args.get(2).uint32_value(_scope).unwrap_or(0) as u64;
            let r = args.get(3).uint32_value(_scope).unwrap_or(0);
            let p = args.get(4).uint32_value(_scope).unwrap_or(0);
            let keylen = args.get(5).uint32_value(_scope).unwrap_or(0) as usize;
            let password: Vec<u8> = password_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let salt: Vec<u8> = salt_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            std::thread::spawn(move || {
                let _result = do_scrypt(password, salt, n, r, p, keylen);
            });
            rv.set(v8::undefined(_scope).into());
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoScrypt").unwrap().into(),
        crypto_scrypt.unwrap().into(),
    );

    let crypto_rsa_sign = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let digest_alg = args.get(0).to_rust_string_lossy(_scope);
            let pem = args.get(1).to_rust_string_lossy(_scope);
            let data_arg = args.get(2);
            let data: Vec<u8> = data_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_rsa_sign(&digest_alg, &pem, &data) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoRsaSign").unwrap().into(),
        crypto_rsa_sign.unwrap().into(),
    );

    let crypto_rsa_verify = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let digest_alg = args.get(0).to_rust_string_lossy(_scope);
            let pem = args.get(1).to_rust_string_lossy(_scope);
            let data_arg = args.get(2);
            let sig_arg = args.get(3);
            let data: Vec<u8> = data_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let sig: Vec<u8> = sig_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_rsa_verify(&digest_alg, &pem, &data, &sig) {
                Ok(ok) => rv.set(v8::Boolean::new(_scope, ok).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoRsaVerify").unwrap().into(),
        crypto_rsa_verify.unwrap().into(),
    );

    let crypto_ec_sign = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let named_curve = args.get(0).to_rust_string_lossy(_scope);
            let pem = args.get(1).to_rust_string_lossy(_scope);
            let data_arg = args.get(2);
            let data: Vec<u8> = data_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_ec_sign(&named_curve, &pem, &data) {
                Ok(bytes) => rv.set(uint8array_from_bytes(_scope, &bytes).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoEcSign").unwrap().into(),
        crypto_ec_sign.unwrap().into(),
    );

    let crypto_ec_verify = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let named_curve = args.get(0).to_rust_string_lossy(_scope);
            let pem = args.get(1).to_rust_string_lossy(_scope);
            let data_arg = args.get(2);
            let sig_arg = args.get(3);
            let data: Vec<u8> = data_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let sig: Vec<u8> = sig_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            match do_ec_verify(&named_curve, &pem, &data, &sig) {
                Ok(ok) => rv.set(v8::Boolean::new(_scope, ok).into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoEcVerify").unwrap().into(),
        crypto_ec_verify.unwrap().into(),
    );

    let crypto_scrypt_sync = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let password_arg = args.get(0);
            let salt_arg = args.get(1);
            let n = args.get(2).uint32_value(_scope).unwrap_or(0) as u64;
            let r = args.get(3).uint32_value(_scope).unwrap_or(0);
            let p = args.get(4).uint32_value(_scope).unwrap_or(0);
            let keylen = args.get(5).uint32_value(_scope).unwrap_or(0) as usize;
            let password: Vec<u8> = password_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let salt: Vec<u8> = salt_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            if n == 0 || (n & (n - 1)) != 0 {
                rv.set(
                    v8::String::new(_scope, "N must be a power of 2 greater than 1")
                        .unwrap()
                        .into(),
                );
                return;
            }
            let log_n = n.ilog2() as u8;
            let params = match scrypt::Params::new(log_n, r, p, keylen) {
                Ok(p) => p,
                Err(e) => {
                    rv.set(
                        v8::String::new(_scope, &format!("invalid scrypt params: {e}"))
                            .unwrap()
                            .into(),
                    );
                    return;
                }
            };
            let mut out = vec![0u8; keylen];
            if let Err(e) = scrypt::scrypt(&password, &salt, &params, &mut out) {
                rv.set(
                    v8::String::new(_scope, &format!("scrypt error: {e}"))
                        .unwrap()
                        .into(),
                );
                return;
            }
            rv.set(uint8array_from_bytes(_scope, &out).into());
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoScryptSync").unwrap().into(),
        crypto_scrypt_sync.unwrap().into(),
    );

    let crypto_generate_keypair = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let key_type = args.get(0).to_rust_string_lossy(_scope);
            let options_json = args.get(1).to_rust_string_lossy(_scope);
            std::thread::spawn(move || {
                let _result = do_generate_keypair(key_type, options_json);
            });
            rv.set(v8::undefined(_scope).into());
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoGenerateKeyPair")
            .unwrap()
            .into(),
        crypto_generate_keypair.unwrap().into(),
    );

    let crypto_generate_keypair_sync = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let key_type = args.get(0).to_rust_string_lossy(_scope);
            let options_json = args.get(1).to_rust_string_lossy(_scope);
            match do_generate_keypair_sync_inner(&key_type, &options_json) {
                Ok(s) => rv.set(v8::String::new(_scope, &s).unwrap().into()),
                Err(e) => rv.set(v8::String::new(_scope, &e.to_string()).unwrap().into()),
            }
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoGenerateKeyPairSync")
            .unwrap()
            .into(),
        crypto_generate_keypair_sync.unwrap().into(),
    );

    let crypto_pbkdf2_sync2 = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            let password_arg = args.get(0);
            let salt_arg = args.get(1);
            let iterations = args.get(2).uint32_value(_scope).unwrap_or(0);
            let keylen = args.get(3).uint32_value(_scope).unwrap_or(0) as usize;
            let digest = args.get(4).to_rust_string_lossy(_scope);
            let password: Vec<u8> = password_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let salt: Vec<u8> = salt_arg
                .try_into()
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();
            let mut out = vec![0u8; keylen];
            match norm_alg(&digest).as_str() {
                "sha1" => pbkdf2_hmac::<Sha1>(&password, &salt, iterations, &mut out),
                "sha224" => pbkdf2_hmac::<Sha224>(&password, &salt, iterations, &mut out),
                "sha256" => pbkdf2_hmac::<Sha256>(&password, &salt, iterations, &mut out),
                "sha384" => pbkdf2_hmac::<Sha384>(&password, &salt, iterations, &mut out),
                "sha512" => pbkdf2_hmac::<Sha512>(&password, &salt, iterations, &mut out),
                other => {
                    rv.set(
                        v8::String::new(_scope, &format!("unsupported digest: {other}"))
                            .unwrap()
                            .into(),
                    );
                    return;
                }
            }
            rv.set(uint8array_from_bytes(_scope, &out).into());
        },
    );
    global.set(
        scope,
        v8::String::new(scope, "__cryptoPbkdf2Sync").unwrap().into(),
        crypto_pbkdf2_sync2.unwrap().into(),
    );

    let js_code = r#"
(function() {
    function toBytes(v) {
        if (v instanceof Uint8Array) return Array.from(v);
        if (Array.isArray(v)) return v;
        if (typeof v === 'string') {
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
        return Array.from(new Uint8Array(v.buffer ? v.buffer : v));
    }

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

    var _dhGroups = {
        modp2:  { prime: 'FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF' },
        modp5:  { prime: 'FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA237327FFFFFFFFFFFFFFFF' },
        modp14: { prime: 'FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF' },
        modp15: { prime: 'FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA0510157256E5A8AACAA68FFFFFFFFFFFFFFFF' },
        modp16: { prime: 'FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF' }
    };

    function _bigModpow(base, exp, mod) {
        var result = 1n;
        base = base % mod;
        while (exp > 0n) {
            if (exp & 1n) result = result * base % mod;
            exp >>= 1n;
            base = base * base % mod;
        }
        return result;
    }

    function _bigIntToBytes(n) {
        var hex = n.toString(16);
        if (hex.length % 2 !== 0) hex = '0' + hex;
        var bytes = new Uint8Array(hex.length / 2);
        for (var i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i*2, i*2+2), 16);
        return bytes;
    }

    function _makeDH(primeHex, generator) {
        var p = BigInt('0x' + primeHex.toUpperCase());
        var g = BigInt(generator);
        var privKeyBits = Math.min(256, Math.floor(primeHex.length * 4 / 2));
        var privateKey = null;
        var publicKey  = null;

        function randomPrivate() {
            var bytes = Math.ceil(privKeyBits / 8);
            var randBytes = __cryptoRandomBytes(bytes);
            var hex = Array.from(randBytes).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join('');
            return (BigInt('0x' + hex) % (p - 4n)) + 2n;
        }

        return {
            generateKeys: function(encoding) {
                privateKey = randomPrivate();
                publicKey  = _bigModpow(g, privateKey, p);
                return encodeBytes(Array.from(_bigIntToBytes(publicKey)), encoding || 'buffer');
            },
            computeSecret: function(otherPublicKey, inputEncoding, outputEncoding) {
                if (privateKey === null) throw new Error('DH: generateKeys must be called first');
                var bytes = toBytes(otherPublicKey);
                var hex = Array.from(bytes).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join('');
                var otherPub = BigInt('0x' + (hex || '0'));
                var secret = _bigModpow(otherPub, privateKey, p);
                return encodeBytes(Array.from(_bigIntToBytes(secret)), outputEncoding || 'buffer');
            },
            getPublicKey: function(encoding) {
                if (publicKey === null) throw new Error('DH: generateKeys must be called first');
                return encodeBytes(Array.from(_bigIntToBytes(publicKey)), encoding || 'buffer');
            },
            getPrivateKey: function(encoding) {
                if (privateKey === null) throw new Error('DH: generateKeys must be called first');
                return encodeBytes(Array.from(_bigIntToBytes(privateKey)), encoding || 'buffer');
            },
            getPrime: function(encoding) {
                return encodeBytes(Array.from(_bigIntToBytes(p)), encoding || 'buffer');
            },
            getGenerator: function(encoding) {
                return encodeBytes(Array.from(_bigIntToBytes(g)), encoding || 'buffer');
            },
            setPublicKey: function(key) {
                var bytes = toBytes(key);
                var hex = Array.from(bytes).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join('');
                publicKey = BigInt('0x' + (hex || '0'));
            },
            setPrivateKey: function(key) {
                var bytes = toBytes(key);
                var hex = Array.from(bytes).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join('');
                privateKey = BigInt('0x' + (hex || '0'));
                publicKey  = _bigModpow(g, privateKey, p);
            },
            verifyError: 0
        };
    }

    var crypto = {
        createHash: function(alg) {
            var chunks = [];
            var h = {
                update: function(data) {
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
                    var clone = crypto.createHash(alg);
                    for (var i = 0; i < chunks.length; i++) clone.update(chunks[i]);
                    return clone;
                }
            };
            return h;
        },

        hash: function(alg, data, outputEncoding) {
            var raw = __cryptoHash(alg, toBytes(data));
            return encodeBytes(raw, outputEncoding || 'hex');
        },

        createHmac: function(alg, key) {
            var keyBytes = toBytes(key);
            var chunks = [];
            return {
                update: function(data) {
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
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            var h = b.map(function(x) { return ('0' + x.toString(16)).slice(-2); }).join('');
            return h.slice(0,8)+'-'+h.slice(8,12)+'-'+h.slice(12,16)+'-'+h.slice(16,20)+'-'+h.slice(20);
        },

        getRandomValues: function(arr) {
            var bytes = __cryptoRandomBytes(arr.length);
            for (var i = 0; i < arr.length; i++) arr[i] = bytes[i];
            return arr;
        },

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

        createCipheriv: function(algorithm, key, iv, options) {
            var alg = algorithm.toLowerCase();
            var isGcm = alg.indexOf('gcm') !== -1;
            var keyBytes = toBytes(key);
            var ivBytes = iv ? toBytes(iv) : new Uint8Array(0);
            var chunks = [];
            var aad = new Uint8Array(0);
            var self = {
                setAAD: function(buf) { aad = toBytes(buf); return this; },
                update: function(data, inputEncoding, outputEncoding) {
                    var bytes = toBytes(data);
                    if (!isGcm) {
                        chunks.push(bytes);
                        return new Uint8Array(0);
                    }
                    chunks.push(bytes);
                    return new Uint8Array(0);
                },
                final: function(outputEncoding) {
                    var all = [];
                    for (var i = 0; i < chunks.length; i++)
                        for (var j = 0; j < chunks[i].length; j++) all.push(chunks[i][j]);
                    if (isGcm) {
                        var result = __cryptoAesGcmEncrypt(keyBytes.length, Array.from(keyBytes), Array.from(ivBytes), all, Array.from(aad));
                        var ct = new Uint8Array(result.slice(0, result.length - 16));
                        self._tag = new Uint8Array(result.slice(result.length - 16));
                        return ct;
                    } else {
                        var result2 = __cryptoCipherOneShot(alg, Array.from(keyBytes), Array.from(ivBytes), all, true);
                        return new Uint8Array(result2);
                    }
                },
                getAuthTag: function() { return self._tag || new Uint8Array(16); }
            };
            return self;
        },

        createDecipheriv: function(algorithm, key, iv, options) {
            var alg = algorithm.toLowerCase();
            var isGcm = alg.indexOf('gcm') !== -1;
            var keyBytes = toBytes(key);
            var ivBytes = iv ? toBytes(iv) : new Uint8Array(0);
            var chunks = [];
            var aad = new Uint8Array(0);
            var authTag = null;
            return {
                setAAD: function(buf) { aad = toBytes(buf); return this; },
                setAuthTag: function(tag) { authTag = toBytes(tag); return this; },
                update: function(data, inputEncoding, outputEncoding) {
                    chunks.push(toBytes(data));
                    return new Uint8Array(0);
                },
                final: function(outputEncoding) {
                    var all = [];
                    for (var i = 0; i < chunks.length; i++)
                        for (var j = 0; j < chunks[i].length; j++) all.push(chunks[i][j]);
                    if (isGcm) {
                        var tag = authTag || new Uint8Array(16);
                        var ct_and_tag = all.concat(Array.from(tag));
                        var result = __cryptoAesGcmDecrypt(keyBytes.length, Array.from(keyBytes), Array.from(ivBytes), ct_and_tag, Array.from(aad));
                        return new Uint8Array(result);
                    } else {
                        var result2 = __cryptoCipherOneShot(alg, Array.from(keyBytes), Array.from(ivBytes), all, false);
                        return new Uint8Array(result2);
                    }
                }
            };
        },

        KeyObject: (function() {
            function KeyObject(type, props) {
                this.type = type;
                for (var k in props) if (props.hasOwnProperty(k)) this[k] = props[k];
            }
            KeyObject.prototype.toString = function() { return this._pem || '[KeyObject]'; };
            return KeyObject;
        })(),

        createPrivateKey: function(key) {
            var pem = typeof key === 'string' ? key : (key && key.key ? key.key : (key instanceof Uint8Array ? new TextDecoder().decode(key) : (key && typeof key.toString === 'function' ? key.toString() : String(key))));
            if (pem.indexOf('-----BEGIN') === -1) throw new Error('Invalid key: must be PEM-encoded');
            var kt = pem.indexOf('EC PRIVATE') !== -1 ? 'ec' : 'rsa';
            var curve = pem.indexOf('P-384') !== -1 || pem.indexOf('secp384r1') !== -1 ? 'P-384' : 'P-256';
            var obj = new crypto.KeyObject('private', { asymmetricKeyType: kt, _pem: pem, _curve: curve });
            obj.export = function(opts) {
                if (!opts || opts.format === 'pem') return pem;
                var b64 = pem.replace(/-----[^-]+-----/g,'').replace(/\s/g,'');
                var bin = atob(b64), bytes = new Uint8Array(bin.length);
                for (var i=0;i<bin.length;i++) bytes[i]=bin.charCodeAt(i);
                return bytes;
            };
            return obj;
        },

        createPublicKey: function(key) {
            var pem;
            if (key && key.type === 'private') {
                pem = key._pem;
            } else if (typeof key === 'string') {
                pem = key;
            } else {
                pem = key && typeof key.toString === 'function' ? key.toString() : String(key);
            }
            if (pem.indexOf('-----BEGIN') === -1) throw new Error('Invalid key: must be PEM-encoded');
            var kt = pem.indexOf('EC') !== -1 || pem.indexOf('BEGIN PUBLIC KEY') !== -1 ? 'rsa' : 'rsa';
            var curve = pem.indexOf('P-384') !== -1 || pem.indexOf('secp384r1') !== -1 ? 'P-384' : 'P-256';
            var obj = new crypto.KeyObject('public', { asymmetricKeyType: kt, _pem: pem, _curve: curve });
            obj.export = function(opts) {
                if (!opts || opts.format === 'pem') return pem;
                var b64 = pem.replace(/-----[^-]+-----/g,'').replace(/\s/g,'');
                var bin = atob(b64), bytes = new Uint8Array(bin.length);
                for (var i=0;i<bin.length;i++) bytes[i]=bin.charCodeAt(i);
                return bytes;
            };
            return obj;
        },

        createSecretKey: function(key, encoding) {
            var bytes = key instanceof Uint8Array ? key : toBytes(key);
            return new crypto.KeyObject('secret', { buffer: bytes });
        },

        publicEncrypt: function(opts, data) {
            var key = opts;
            if (key && key.type === 'private') key = key._pem;
            var result = __cryptoRsaSign('sha256', String(key), toBytes(data));
            if (typeof result === 'string') throw new Error(result);
            return new Uint8Array(result);
        },

        privateDecrypt: function(opts, data) {
            var key = opts;
            if (key && key.type === 'private') key = key._pem;
            var result = __cryptoRsaSign('sha256', String(key), toBytes(data));
            if (typeof result === 'string') throw new Error(result);
            return new Uint8Array(result);
        },

        generateKeyPair: function(type, opts, callback) {
            if (typeof opts === 'function') { callback = opts; opts = {}; }
            __cryptoGenerateKeyPair(type, JSON.stringify(opts || {}))
                .then(function(raw) { callback(null, JSON.parse(raw)); })
                .catch(function(err) { callback(err); });
        },

        generateKeyPairSync: function(type, opts) {
            var result = __cryptoGenerateKeyPairSync(type, JSON.stringify(opts || {}));
            if (typeof result === 'string') throw new Error(result);
            return JSON.parse(result);
        },

        DiffieHellman: function(prime, generator) {
            if (typeof prime === 'string') {
                if (_dhGroups[prime]) {
                    return _makeDH(_dhGroups[prime].prime, generator || 2);
                }
                prime = BigInt('0x' + prime);
            }
            var p = typeof prime === 'bigint' ? prime : BigInt(prime);
            var g = BigInt(generator || 2);
            var privKeyBits = 256;
            var privateKey = null;
            var publicKey = null;

            function randomPrivate() {
                var bytes = Math.ceil(privKeyBits / 8);
                var randBytes = __cryptoRandomBytes(bytes);
                var hex = Array.from(randBytes).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join('');
                return (BigInt('0x' + hex) % (p - 4n)) + 2n;
            }

            return {
                generateKeys: function() {
                    privateKey = randomPrivate();
                    publicKey = _bigModpow(g, privateKey, p);
                    return _bigIntToBytes(publicKey);
                },
                computeSecret: function(otherPublicKey) {
                    if (privateKey === null) throw new Error('generateKeys must be called first');
                    var otherPub = typeof otherPublicKey === 'bigint' ? otherPublicKey : BigInt('0x' + Array.from(toBytes(otherPublicKey)).map(function(b) { return ('0' + b.toString(16)).slice(-2); }).join(''));
                    return _bigIntToBytes(_bigModpow(otherPub, privateKey, p));
                },
                getPublicKey: function() { return _bigIntToBytes(publicKey); },
                getPrivateKey: function() { return _bigIntToBytes(privateKey); },
                getPrime: function() { return _bigIntToBytes(p); },
                getGenerator: function() { return _bigIntToBytes(g); },
                verifyError: 0
            };
        },

        createDiffieHellman: function(prime, generator) { return crypto.DiffieHellman(prime, generator); },

        setEngine: function() {},
        constants: {},
        fips: false,
        provider: 'default'
    };

    globalThis.crypto = crypto;
    globalThis.Crypto = crypto;
    if (globalThis.__requireCache) {
        globalThis.__requireCache['crypto'] = crypto;
        globalThis.__requireCache['node:crypto'] = crypto;
    }
})();
"#;

    let script = v8::Script::compile(scope, v8::String::new(scope, js_code).unwrap(), None)
        .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    Ok(())
}
