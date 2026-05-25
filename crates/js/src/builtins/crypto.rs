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
            let mut mac =
                <$T>::new_from_slice(&key).map_err(|e| anyhow::anyhow!("invalid HMAC key: {e}"))?;
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

        pbkdf2Sync: function() {
            throw new Error(
                'pbkdf2Sync is not available in async context; use crypto.pbkdf2() with a callback'
            );
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

        scryptSync: function() {
            throw new Error(
                'scryptSync is not available in async context; use crypto.scrypt() with a callback'
            );
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

        // Stub: Web Crypto subtle — only available if the runtime provides it.
        get subtle() {
            return (globalThis.crypto && globalThis.crypto.subtle) ? globalThis.crypto.subtle : undefined;
        },

        constants: {
            POINT_CONVERSION_COMPRESSED: 2,
            POINT_CONVERSION_HYBRID: 3,
            POINT_CONVERSION_UNCOMPRESSED: 4,
        }
    };

    if (globalThis.__requireCache) {
        globalThis.__requireCache['crypto'] = crypto;
        globalThis.__requireCache['node:crypto'] = crypto;
    }
})();
        "#,
    )?;

    Ok(())
}
