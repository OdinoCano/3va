use crate::builtins::v8_compat::{uint8array_from_bytes, uint8array_to_vec};
use brotli::enc::BrotliEncoderParams;
use flate2::Compression;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use std::io::{Read, Write};
use v8::{ContextScope, Function, HandleScope, Script, String};

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

fn brotli_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli::BrotliCompress(&mut &data[..], &mut out, &BrotliEncoderParams::default())?;
    Ok(out)
}

fn brotli_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli::BrotliDecompress(&mut &data[..], &mut out)?;
    Ok(out)
}

fn run_compress_async<F>(data: Vec<u8>, f: F) -> std::result::Result<Vec<u8>, std::string::String>
where
    F: FnOnce(Vec<u8>) -> anyhow::Result<Vec<u8>> + Send,
{
    tokio::task::block_in_place(|| f(data)).map_err(|e| e.to_string())
}

pub fn inject_zlib(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    let gzip_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, gzip_compress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibGzip").unwrap().into(),
        gzip_fn.into(),
    );

    let gunzip_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, gzip_decompress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibGunzip").unwrap().into(),
        gunzip_fn.into(),
    );

    let deflate_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, deflate_compress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibDeflate").unwrap().into(),
        deflate_fn.into(),
    );

    let inflate_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, deflate_decompress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibInflate").unwrap().into(),
        inflate_fn.into(),
    );

    let raw_deflate_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, raw_deflate_compress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibRawDeflate").unwrap().into(),
        raw_deflate_fn.into(),
    );

    let raw_inflate_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, raw_deflate_decompress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibRawInflate").unwrap().into(),
        raw_inflate_fn.into(),
    );

    let brotli_compress_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, brotli_compress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibBrotliCompress").unwrap().into(),
        brotli_compress_fn.into(),
    );

    let brotli_decompress_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match run_compress_async(data, brotli_decompress) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibBrotliDecompress").unwrap().into(),
        brotli_decompress_fn.into(),
    );

    let gzip_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match gzip_compress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibGzipSync").unwrap().into(),
        gzip_sync_fn.into(),
    );

    let gunzip_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match gzip_decompress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibGunzipSync").unwrap().into(),
        gunzip_sync_fn.into(),
    );

    let deflate_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match deflate_compress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibDeflateSync").unwrap().into(),
        deflate_sync_fn.into(),
    );

    let inflate_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match deflate_decompress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibInflateSync").unwrap().into(),
        inflate_sync_fn.into(),
    );

    let raw_deflate_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match raw_deflate_compress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibRawDeflateSync").unwrap().into(),
        raw_deflate_sync_fn.into(),
    );

    let raw_inflate_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match raw_deflate_decompress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibRawInflateSync").unwrap().into(),
        raw_inflate_sync_fn.into(),
    );

    let brotli_compress_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match brotli_compress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibBrotliCompressSync")
            .unwrap()
            .into(),
        brotli_compress_sync_fn.into(),
    );

    let brotli_decompress_sync_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let data_arg = args.get(0);
            let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                uint8array_to_vec(_scope, arr)
            } else {
                vec![]
            };

            match brotli_decompress(data) {
                Ok(result) => {
                    let result_arr = uint8array_from_bytes(_scope, &result);
                    rv.set(result_arr.into());
                }
                Err(e) => {
                    let err_str = String::new(_scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        String::new(scope, "__zlibBrotliDecompressSync")
            .unwrap()
            .into(),
        brotli_decompress_sync_fn.into(),
    );

    let js_code = r#"
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
                return function() { throw new Error('zlib sync methods not available in async context'); };
            }

            var zlib = {
                gzip:        makeCallback(__zlibGzip,       'gzip'),
                gunzip:      makeCallback(__zlibGunzip,     'gunzip'),
                deflate:     makeCallback(__zlibDeflate,    'deflate'),
                inflate:     makeCallback(__zlibInflate,    'inflate'),
                deflateRaw:  makeCallback(__zlibRawDeflate, 'deflateRaw'),
                inflateRaw:  makeCallback(__zlibRawInflate, 'inflateRaw'),

                gzipSync:       function(buf) { return new Uint8Array(__zlibGzipSync(Array.from(bufToUint8(buf)))); },
                gunzipSync:     function(buf) { return new Uint8Array(__zlibGunzipSync(Array.from(bufToUint8(buf)))); },
                deflateSync:    function(buf) { return new Uint8Array(__zlibDeflateSync(Array.from(bufToUint8(buf)))); },
                inflateSync:    function(buf) { return new Uint8Array(__zlibInflateSync(Array.from(bufToUint8(buf)))); },
                deflateRawSync: function(buf) { return new Uint8Array(__zlibRawDeflateSync(Array.from(bufToUint8(buf)))); },
                inflateRawSync: function(buf) { return new Uint8Array(__zlibRawInflateSync(Array.from(bufToUint8(buf)))); },
                brotliCompress:     makeCallback(__zlibBrotliCompress, 'brotliCompress'),
                brotliDecompress:   makeCallback(__zlibBrotliDecompress, 'brotliDecompress'),
                brotliCompressSync: function(buf) { return new Uint8Array(__zlibBrotliCompressSync(Array.from(bufToUint8(buf)))); },
                brotliDecompressSync: function(buf) { return new Uint8Array(__zlibBrotliDecompressSync(Array.from(bufToUint8(buf)))); },

                createGzip:    function(opts) { return zlib._makeTransform(__zlibGzip,      __zlibGunzip,      opts); },
                createGunzip:  function(opts) { return zlib._makeTransform(__zlibGunzip,    __zlibGzip,        opts); },
                createDeflate: function(opts) { return zlib._makeTransform(__zlibDeflate,   __zlibInflate,     opts); },
                createInflate: function(opts) { return zlib._makeTransform(__zlibInflate,   __zlibDeflate,     opts); },
                createDeflateRaw: function(opts) { return zlib._makeTransform(__zlibRawDeflate, __zlibRawInflate, opts); },
                createInflateRaw: function(opts) { return zlib._makeTransform(__zlibRawInflate, __zlibRawDeflate, opts); },

                _makeTransform: function(processFn, _reverseFn, _opts) {
                    var listeners = {};
                    var ended = false;
                    var endCb = null;
                    var pending = 0;
                    var piped = [];
                    var stream = {
                        readable: true, writable: true,
                        on: function(ev, fn) {
                            if (!listeners[ev]) listeners[ev] = [];
                            listeners[ev].push(fn); return this;
                        },
                        once: function(ev, fn) {
                            var self = this;
                            function w() { self.removeListener(ev, w); fn.apply(null, arguments); }
                            w._orig = fn; return this.on(ev, w);
                        },
                        addListener: function(ev, fn) { return this.on(ev, fn); },
                        removeListener: function(ev, fn) {
                            if (!listeners[ev]) return this;
                            listeners[ev] = listeners[ev].filter(function(f) { return f !== fn && f._orig !== fn; });
                            return this;
                        },
                        off: function(ev, fn) { return this.removeListener(ev, fn); },
                        emit: function(ev) {
                            var args = Array.prototype.slice.call(arguments, 1);
                            var fns = (listeners[ev] || []).slice();
                            fns.forEach(function(f) { f.apply(null, args); });
                            piped.forEach(function(dest) {
                                if (ev === 'data' && dest.write) dest.write(args[0]);
                                if (ev === 'end' && dest.end) dest.end();
                            });
                            return fns.length > 0;
                        },
                        write: function(chunk, _enc, cb) {
                            var self = this;
                            var data;
                            if (chunk instanceof Uint8Array) data = Array.from(chunk);
                            else if (typeof chunk === 'string') data = Array.from(new TextEncoder().encode(chunk));
                            else data = Array.from(chunk);
                            pending++;
                            processFn(data).then(function(result) {
                                pending--;
                                self.emit('data', new Uint8Array(result));
                                if (typeof cb === 'function') cb(null);
                                if (pending === 0 && ended) self._finish();
                            }).catch(function(e) {
                                pending--;
                                self.emit('error', e);
                                if (typeof cb === 'function') cb(e);
                            });
                            return true;
                        },
                        _finish: function() {
                            this.emit('end');
                            this.emit('finish');
                            if (typeof endCb === 'function') { var f = endCb; endCb = null; f(null); }
                        },
                        end: function(chunk, enc, cb) {
                            if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                            if (typeof enc === 'function') { cb = enc; enc = null; }
                            endCb = cb || null;
                            var self = this;
                            if (chunk != null) {
                                this.write(chunk, enc, function(e) {
                                    if (e) { if (typeof cb === 'function') cb(e); return; }
                                    ended = true;
                                    if (pending === 0) self._finish();
                                });
                            } else {
                                ended = true;
                                if (pending === 0) this._finish();
                            }
                        },
                        pipe: function(dest) { piped.push(dest); return dest; },
                        unpipe: function(dest) {
                            piped = dest ? piped.filter(function(d) { return d !== dest; }) : [];
                            return this;
                        },
                        pause: function() { return this; },
                        resume: function() { return this; },
                        destroy: function(e) {
                            if (e) this.emit('error', e);
                            this.emit('close'); return this;
                        },
                        setEncoding: function() { return this; },
                        read: function() { return null; },
                    };
                    return stream;
                },

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
    "#;
    let source = String::new(scope, js_code).unwrap();
    let _ = Script::compile(scope, source, None).and_then(|s| s.run(scope));

    Ok(())
}
