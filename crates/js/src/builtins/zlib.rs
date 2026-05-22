use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::Compression;
use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use std::io::{Read, Write};

fn gzip_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&data)?;
    Ok(enc.finish()?)
}

fn gzip_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut dec = GzDecoder::new(&data[..]);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(out)
}

fn deflate_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&data)?;
    Ok(enc.finish()?)
}

fn deflate_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut dec = ZlibDecoder::new(&data[..]);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(out)
}

fn raw_deflate_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&data)?;
    Ok(enc.finish()?)
}

fn raw_deflate_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut dec = DeflateDecoder::new(&data[..]);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(out)
}

macro_rules! inject_async_fn {
    ($ctx:expr, $name:expr, $fn:expr) => {
        $ctx.globals().set(
            $name,
            Function::new(
                $ctx.clone(),
                Async(move |data: Vec<u8>| async move {
                    tokio::task::spawn_blocking(move || $fn(data))
                        .await
                        .map_err(|e| {
                            rquickjs::Error::new_from_js_message(
                                "zlib",
                                "spawn",
                                e.to_string(),
                            )
                        })?
                        .map_err(|e| {
                            rquickjs::Error::new_from_js_message("zlib", "zlib", e.to_string())
                        })
                }),
            )?,
        )?;
    };
}

pub fn inject_zlib(ctx: &Ctx) -> Result<()> {
    inject_async_fn!(ctx, "__zlibGzip", gzip_compress);
    inject_async_fn!(ctx, "__zlibGunzip", gzip_decompress);
    inject_async_fn!(ctx, "__zlibDeflate", deflate_compress);
    inject_async_fn!(ctx, "__zlibInflate", deflate_decompress);
    inject_async_fn!(ctx, "__zlibRawDeflate", raw_deflate_compress);
    inject_async_fn!(ctx, "__zlibRawInflate", raw_deflate_decompress);

    // JS wrapper: replaces the stub in modules.rs
    ctx.eval::<(), _>(
        r#"
        (function() {
            function bufToUint8(buf) {
                if (buf instanceof Uint8Array) return buf;
                if (typeof buf === 'string') {
                    var a = new Uint8Array(buf.length);
                    for (var i = 0; i < buf.length; i++) a[i] = buf.charCodeAt(i) & 0xff;
                    return a;
                }
                return new Uint8Array(buf);
            }

            function makeCallback(rustFn, name) {
                return function(buf, opts, cb) {
                    if (typeof opts === 'function') { cb = opts; opts = {}; }
                    var data = Array.from(bufToUint8(buf));
                    rustFn(data).then(function(result) {
                        if (cb) cb(null, new Uint8Array(result));
                    }).catch(function(e) {
                        if (cb) cb(e);
                    });
                };
            }

            function makeSync(rustFn) {
                // No true sync available in async runtime; throw if called sync
                return function() { throw new Error('zlib sync methods not available in async context'); };
            }

            var zlib = {
                gzip:        makeCallback(__zlibGzip,       'gzip'),
                gunzip:      makeCallback(__zlibGunzip,     'gunzip'),
                deflate:     makeCallback(__zlibDeflate,    'deflate'),
                inflate:     makeCallback(__zlibInflate,    'inflate'),
                deflateRaw:  makeCallback(__zlibRawDeflate, 'deflateRaw'),
                inflateRaw:  makeCallback(__zlibRawInflate, 'inflateRaw'),

                gzipSync:    function(buf) { throw new Error('gzipSync not available; use zlib.gzip() with callback'); },
                gunzipSync:  function(buf) { throw new Error('gunzipSync not available; use zlib.gunzip() with callback'); },
                deflateSync: function(buf) { throw new Error('deflateSync not available; use zlib.deflate() with callback'); },
                inflateSync: function(buf) { throw new Error('inflateSync not available; use zlib.inflate() with callback'); },

                createGzip:    function() { return {}; },
                createGunzip:  function() { return {}; },
                createDeflate: function() { return {}; },
                createInflate: function() { return {}; },

                constants: {
                    Z_NO_COMPRESSION: 0, Z_BEST_SPEED: 1, Z_BEST_COMPRESSION: 9,
                    Z_DEFAULT_COMPRESSION: -1, Z_FILTERED: 1, Z_HUFFMAN_ONLY: 2,
                    Z_RLE: 3, Z_FIXED: 4, Z_DEFAULT_STRATEGY: 0,
                    Z_DEFLATED: 8, Z_OK: 0, Z_STREAM_END: 1,
                }
            };

            if (globalThis.__requireCache) {
                globalThis.__requireCache['zlib'] = zlib;
                globalThis.__requireCache['node:zlib'] = zlib;
            }
        })();
        "#,
    )?;

    Ok(())
}
