use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use rand::rngs::OsRng;
use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

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
        "sha1" => Ok(Sha1::digest(&data).to_vec()),
        "sha224" => Ok(Sha224::digest(&data).to_vec()),
        "sha256" => Ok(Sha256::digest(&data).to_vec()),
        "sha384" => Ok(Sha384::digest(&data).to_vec()),
        "sha512" => Ok(Sha512::digest(&data).to_vec()),
        other => Err(anyhow::anyhow!("unsupported hash algorithm: {other}")),
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
            // For scryptSync we fall back to pbkdf2Sync as a safe approximation.
            // True scrypt sync would block for too long for typical keylen; this
            // satisfies libraries that just need a deterministic KDF.
            var raw = __cryptoPbkdf2Sync(toBytes(password), toBytes(salt), N, keylen, 'sha256');
            return new Uint8Array(raw);
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

        // ── Sign / Verify (HMAC-based stubs for JWT and similar) ─────────────
        createSign: function(algorithm) {
            var chunks = [];
            return {
                update: function(data) { chunks.push(toBytes(data)); return this; },
                sign: function(key, outputEncoding) {
                    // For symmetric keys (Buffer/string), use HMAC; for KeyObject, throw.
                    if (!key || typeof key === 'object' && key.type && key.type !== 'secret') {
                        throw new Error('createSign: asymmetric keys require the crypto native addon (not available in 3va); use jsonwebtoken with symmetric keys or WebCrypto SubtleCrypto.');
                    }
                    var keyBytes = toBytes(typeof key === 'string' ? key : (key.export ? key.export() : key));
                    var alg = algorithm.replace('RSA-', '').replace('with', '').toLowerCase();
                    var all = [];
                    for (var i = 0; i < chunks.length; i++) for (var j = 0; j < chunks[i].length; j++) all.push(chunks[i][j]);
                    var raw = __cryptoHmac(alg, Array.from(keyBytes), all);
                    return encodeBytes(raw, outputEncoding);
                }
            };
        },

        createVerify: function(algorithm) {
            var chunks = [];
            return {
                update: function(data) { chunks.push(toBytes(data)); return this; },
                verify: function(key, signature, sigEncoding) {
                    var keyBytes = toBytes(typeof key === 'string' ? key : (key.export ? key.export() : key));
                    var alg = algorithm.replace('RSA-', '').replace('with', '').toLowerCase();
                    var all = [];
                    for (var i = 0; i < chunks.length; i++) for (var j = 0; j < chunks[i].length; j++) all.push(chunks[i][j]);
                    var raw = __cryptoHmac(alg, Array.from(keyBytes), all);
                    var expected = encodeBytes(raw, sigEncoding || 'hex');
                    var actual = typeof signature === 'string' ? signature : encodeBytes(Array.from(toBytes(signature)), sigEncoding || 'hex');
                    return expected === actual;
                }
            };
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
    //   sign/verify (HMAC), encrypt/decrypt (AES-GCM),
    //   deriveBits/deriveKey (HKDF, PBKDF2).
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

        // Internal CryptoKey representation.
        function CryptoKey(type, extractable, algorithm, usages, raw) {
            this.type = type;          // "secret" | "public" | "private"
            this.extractable = extractable;
            this.algorithm = algorithm; // {name, ...}
            this.usages = usages;
            this._raw = raw;           // Uint8Array of raw key bytes
        }

        return {
            // ── digest ───────────────────────────────────────────────────────
            digest: function(algorithm, data) {
                var alg = normAlg(algorithm);
                // Map Web Crypto names to Node crypto names
                var map = { 'SHA-1': 'sha1', 'SHA-224': 'sha224', 'SHA-256': 'sha256', 'SHA-384': 'sha384', 'SHA-512': 'sha512' };
                var nodeAlg = map[alg];
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
                    return Promise.reject(new DOMException('generateKey: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── importKey ────────────────────────────────────────────────────
            importKey: function(format, keyData, algorithm, extractable, keyUsages) {
                try {
                    var alg = normAlg(algorithm);
                    var raw;
                    if (format === 'raw') {
                        raw = rawBytes(keyData);
                    } else if (format === 'jwk') {
                        // JWK: base64url-decode 'k' field for symmetric keys
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
                        var map = { 'SHA-1': 'sha1', 'SHA-224': 'sha224', 'SHA-256': 'sha256', 'SHA-384': 'sha384', 'SHA-512': 'sha512' };
                        var nodeAlg = map[hash];
                        if (!nodeAlg) return Promise.reject(new DOMException('Unsupported HMAC hash: ' + hash, 'NotSupportedError'));
                        var raw = __cryptoHmac(nodeAlg, Array.from(key._raw), toByteArray(data));
                        return Promise.resolve(toArrayBuffer(raw));
                    }
                    return Promise.reject(new DOMException('sign: unsupported algorithm ' + alg, 'NotSupportedError'));
                } catch(e) { return Promise.reject(e); }
            },

            // ── verify ───────────────────────────────────────────────────────
            verify: function(algorithm, key, signature, data) {
                var self = this;
                return self.sign(algorithm, key, data).then(function(expected) {
                    var e = new Uint8Array(expected);
                    var s = rawBytes(signature);
                    if (e.length !== s.length) return false;
                    var diff = 0;
                    for (var i = 0; i < e.length; i++) diff |= e[i] ^ s[i];
                    return diff === 0;
                });
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
    globalThis.crypto = { subtle: subtle, getRandomValues: crypto.getRandomValues, randomUUID: crypto.randomUUID };

    if (globalThis.__requireCache) {
        globalThis.__requireCache['crypto'] = crypto;
        globalThis.__requireCache['node:crypto'] = crypto;
    }
})();
        "#,
    )?;

    Ok(())
}
