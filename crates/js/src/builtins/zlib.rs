use crate::builtins::v8_compat::{js_value_to_bytes, uint8array_from_bytes};
use brotli::enc::BrotliEncoderParams;
use flate2::Compression;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use std::io::{Read, Write};
use v8::{ContextScope, Function, HandleScope, Script, String};

/// Decompression-bomb guards for the zlib builtins.
///
/// Node itself does not bound decompressed output by default; this runtime
/// does, because scripts routinely inflate untrusted payloads and a small
/// gzip/brotli input can otherwise expand to gigabytes in memory.
///
/// These are the defaults used by the JS-visible bindings. Tests inject lower
/// values through the `*_with_limits` functions instead of relying on huge
/// payloads.
const MAX_DECOMPRESSED_OUTPUT_BYTES: usize = 512 * 1024 * 1024;
/// Incremental expansion-ratio tripwire: abort once produced/consumed exceeds
/// this ratio while streaming. Deflate's practical ceiling is ~1032:1, so
/// legitimate streams never trip it; bombs do, well before the size cap.
const MAX_DECOMPRESSION_RATIO: u64 = 4096;
/// Ratio accounting starts only after this much compressed input has been
/// consumed, so tiny inputs near stream headers cannot false-positive.
const RATIO_MIN_INPUT_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy)]
struct DecompressionLimits {
    max_output_bytes: usize,
    max_ratio: u64,
    ratio_min_input_bytes: u64,
}

impl Default for DecompressionLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: MAX_DECOMPRESSED_OUTPUT_BYTES,
            max_ratio: MAX_DECOMPRESSION_RATIO,
            ratio_min_input_bytes: RATIO_MIN_INPUT_BYTES,
        }
    }
}

/// Wraps the compressed input and records how many bytes the decoder pulled.
struct CountingReader<'a, R: Read> {
    inner: R,
    consumed: &'a std::cell::Cell<u64>,
}

impl<R: Read> Read for CountingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.consumed.set(self.consumed.get() + n as u64);
        Ok(n)
    }
}

/// Streams `dec` to memory enforcing both guards after every chunk read:
/// absolute output size and incremental expansion ratio. Aborts with a clear
/// error as soon as either is exceeded instead of letting the buffer grow.
fn read_bounded(
    mut dec: impl Read,
    consumed: &std::cell::Cell<u64>,
    lim: DecompressionLimits,
) -> anyhow::Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = dec.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > lim.max_output_bytes {
            anyhow::bail!(
                "decompression aborted: output exceeded {} bytes (possible decompression bomb)",
                lim.max_output_bytes
            );
        }
        let consumed_now = consumed.get();
        if consumed_now >= lim.ratio_min_input_bytes
            && out.len() as u64 > consumed_now.saturating_mul(lim.max_ratio)
        {
            anyhow::bail!(
                "decompression aborted: expansion ratio exceeded {}:1 ({} output bytes from {} input bytes; possible decompression bomb)",
                lim.max_ratio,
                out.len(),
                consumed_now
            );
        }
    }
}

fn gzip_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&data)?;
    Ok(enc.finish()?)
}

fn deflate_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&data)?;
    Ok(enc.finish()?)
}

fn raw_deflate_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&data)?;
    Ok(enc.finish()?)
}

fn brotli_compress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli::BrotliCompress(&mut &data[..], &mut out, &BrotliEncoderParams::default())?;
    Ok(out)
}

fn gunzip_with_limits(data: Vec<u8>, lim: DecompressionLimits) -> anyhow::Result<Vec<u8>> {
    let consumed = std::cell::Cell::new(0u64);
    let mut dec = GzDecoder::new(CountingReader {
        inner: &data[..],
        consumed: &consumed,
    });
    read_bounded(&mut dec, &consumed, lim)
}

fn inflate_with_limits(data: Vec<u8>, lim: DecompressionLimits) -> anyhow::Result<Vec<u8>> {
    let consumed = std::cell::Cell::new(0u64);
    let mut dec = ZlibDecoder::new(CountingReader {
        inner: &data[..],
        consumed: &consumed,
    });
    read_bounded(&mut dec, &consumed, lim)
}

fn raw_inflate_with_limits(data: Vec<u8>, lim: DecompressionLimits) -> anyhow::Result<Vec<u8>> {
    let consumed = std::cell::Cell::new(0u64);
    let mut dec = DeflateDecoder::new(CountingReader {
        inner: &data[..],
        consumed: &consumed,
    });
    read_bounded(&mut dec, &consumed, lim)
}

fn brotli_decompress_with_limits(
    data: Vec<u8>,
    lim: DecompressionLimits,
) -> anyhow::Result<Vec<u8>> {
    let consumed = std::cell::Cell::new(0u64);
    let mut dec = brotli::Decompressor::new(
        CountingReader {
            inner: &data[..],
            consumed: &consumed,
        },
        65_536,
    );
    read_bounded(&mut dec, &consumed, lim)
}

fn gzip_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    gunzip_with_limits(data, DecompressionLimits::default())
}

fn deflate_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    inflate_with_limits(data, DecompressionLimits::default())
}

fn raw_deflate_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    raw_inflate_with_limits(data, DecompressionLimits::default())
}

fn brotli_decompress(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    brotli_decompress_with_limits(data, DecompressionLimits::default())
}

fn run_compress_async<F>(data: Vec<u8>, f: F) -> std::result::Result<Vec<u8>, std::string::String>
where
    F: FnOnce(Vec<u8>) -> anyhow::Result<Vec<u8>> + Send,
{
    f(data).map_err(|e| e.to_string())
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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
            let data: Vec<u8> = js_value_to_bytes(_scope, data_arg);

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
                    // rustFn is a plain synchronous native call (no real async
                    // work happens on the Rust side); defer via setTimeout so
                    // the callback still fires asynchronously like Node's does.
                    setTimeout(function() {
                        var result = rustFn(data);
                        if (typeof result === 'string') {
                            if (cb) cb(new Error(result));
                        } else if (cb) {
                            cb(null, Buffer.from(result));
                        }
                    }, 0);
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

                gzipSync:       function(buf) { return Buffer.from(__zlibGzipSync(Array.from(bufToUint8(buf)))); },
                gunzipSync:     function(buf) { return Buffer.from(__zlibGunzipSync(Array.from(bufToUint8(buf)))); },
                deflateSync:    function(buf) { return Buffer.from(__zlibDeflateSync(Array.from(bufToUint8(buf)))); },
                inflateSync:    function(buf) { return Buffer.from(__zlibInflateSync(Array.from(bufToUint8(buf)))); },
                deflateRawSync: function(buf) { return Buffer.from(__zlibRawDeflateSync(Array.from(bufToUint8(buf)))); },
                inflateRawSync: function(buf) { return Buffer.from(__zlibRawInflateSync(Array.from(bufToUint8(buf)))); },
                brotliCompress:     makeCallback(__zlibBrotliCompress, 'brotliCompress'),
                brotliDecompress:   makeCallback(__zlibBrotliDecompress, 'brotliDecompress'),
                brotliCompressSync: function(buf) { return Buffer.from(__zlibBrotliCompressSync(Array.from(bufToUint8(buf)))); },
                brotliDecompressSync: function(buf) { return Buffer.from(__zlibBrotliDecompressSync(Array.from(bufToUint8(buf)))); },

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
                    // Eagerly buffer data/end so asyncIterator never misses events
                    var _iterQueue = [], _iterDone = false, _iterWaiting = null;
                    // Buffer incoming chunks; decompress all at once on end
                    var _inputBuf = [];
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
                            if (ev === 'data') {
                                if (_iterWaiting) { var w = _iterWaiting; _iterWaiting = null; w({ value: args[0], done: false }); }
                                else _iterQueue.push(args[0]);
                            } else if (ev === 'end') {
                                _iterDone = true;
                                if (_iterWaiting) { var w2 = _iterWaiting; _iterWaiting = null; w2({ done: true }); }
                            }
                            var fns = (listeners[ev] || []).slice();
                            fns.forEach(function(f) { f.apply(null, args); });
                            piped.forEach(function(dest) {
                                if (ev === 'data' && dest.write) dest.write(args[0]);
                                if (ev === 'end' && dest.end) dest.end();
                            });
                            return fns.length > 0;
                        },
                        write: function(chunk, _enc, cb) {
                            // Buffer chunks; actual decompression happens in end() once all data arrives
                            var data;
                            if (chunk instanceof Uint8Array) data = chunk;
                            else if (typeof chunk === 'string') data = new TextEncoder().encode(chunk);
                            else data = new Uint8Array(chunk);
                            _inputBuf.push(data);
                            if (typeof cb === 'function') setTimeout(cb, 0);
                            return true;
                        },
                        _finish: function() {
                            var self = this;
                            if (_inputBuf.length > 0) {
                                // Concatenate all buffered input and decompress once
                                var totalLen = _inputBuf.reduce(function(s, b) { return s + b.length; }, 0);
                                var merged = new Uint8Array(totalLen);
                                var offset = 0;
                                _inputBuf.forEach(function(b) { merged.set(b, offset); offset += b.length; });
                                _inputBuf = [];
                                var result = processFn(Array.from(merged));
                                if (typeof result === 'string') {
                                    self.emit('error', new Error(result));
                                    if (typeof endCb === 'function') { var f = endCb; endCb = null; f(new Error(result)); }
                                    return;
                                }
                                self.emit('data', Buffer.from(result));
                            }
                            this.emit('end');
                            this.emit('finish');
                            if (typeof endCb === 'function') { var f2 = endCb; endCb = null; f2(null); }
                        },
                        end: function(chunk, enc, cb) {
                            if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                            if (typeof enc === 'function') { cb = enc; enc = null; }
                            endCb = cb || null;
                            if (chunk != null) this.write(chunk, enc, null);
                            // All chunks are buffered; decompress now
                            this._finish();
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
                    stream[Symbol.asyncIterator] = function() {
                        return {
                            next: function() {
                                return new Promise(function(resolve) {
                                    if (_iterQueue.length) return resolve({ value: _iterQueue.shift(), done: false });
                                    if (_iterDone) return resolve({ done: true });
                                    _iterWaiting = resolve;
                                });
                            },
                            return: function() { _iterDone = true; return Promise.resolve({ done: true }); }
                        };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn zeros(n: usize) -> Vec<u8> {
        vec![0u8; n]
    }

    #[test]
    fn roundtrip_under_default_limits() {
        let payload = b"hello 3va zlib bomb guard".repeat(100);
        let gz = gzip_compress(payload.clone()).unwrap();
        assert_eq!(gzip_decompress(gz).unwrap(), payload);

        let br = brotli_compress(payload.clone()).unwrap();
        assert_eq!(brotli_decompress(br).unwrap(), payload);
    }

    #[test]
    fn gunzip_aborts_when_output_exceeds_injected_cap() {
        // 8 MiB of zeros compress to a few KB — a classic small-bomb shape.
        let data = gzip_compress(zeros(8 * 1024 * 1024)).unwrap();
        let lim = DecompressionLimits {
            max_output_bytes: 1024 * 1024,
            ..DecompressionLimits::default()
        };
        let err = gunzip_with_limits(data, lim).unwrap_err();
        assert!(
            err.to_string().contains("output exceeded"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn gunzip_ratio_tripwire_fires_before_size_cap() {
        // ~1024:1 real ratio; with the size cap far out of reach (64 MiB) only
        // the incremental ratio guard can reject this stream.
        let data = gzip_compress(zeros(8 * 1024 * 1024)).unwrap();
        let lim = DecompressionLimits {
            max_output_bytes: 64 * 1024 * 1024,
            max_ratio: 128,
            ratio_min_input_bytes: 1024,
        };
        let err = gunzip_with_limits(data, lim).unwrap_err();
        assert!(
            err.to_string().contains("expansion ratio"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ratio_guard_ignores_small_inputs_below_floor() {
        // Small legit payload: even at high effective ratio, input stays below
        // the floor so no false positive fires.
        let payload = zeros(64 * 1024);
        let data = gzip_compress(payload.clone()).unwrap();
        let lim = DecompressionLimits {
            max_output_bytes: 512 * 1024,
            max_ratio: 2,
            ratio_min_input_bytes: 256 * 1024,
        };
        assert_eq!(gunzip_with_limits(data, lim).unwrap(), payload);
    }

    #[test]
    fn inflate_and_raw_inflate_are_bounded() {
        let z = deflate_compress(zeros(4 * 1024 * 1024)).unwrap();
        let raw = raw_deflate_compress(zeros(4 * 1024 * 1024)).unwrap();
        for (err_data, f) in [
            (
                z,
                (inflate_with_limits
                    as fn(Vec<u8>, DecompressionLimits) -> anyhow::Result<Vec<u8>>),
            ),
            (raw, raw_inflate_with_limits),
        ] {
            let lim = DecompressionLimits {
                max_output_bytes: 1024 * 1024,
                ..DecompressionLimits::default()
            };
            let err = f(err_data, lim).unwrap_err();
            assert!(err.to_string().contains("output exceeded"));
        }
    }

    #[test]
    fn brotli_decompress_is_bounded() {
        let data = brotli_compress(zeros(8 * 1024 * 1024)).unwrap();
        let lim = DecompressionLimits {
            max_output_bytes: 1024 * 1024,
            ..DecompressionLimits::default()
        };
        let err = brotli_decompress_with_limits(data, lim).unwrap_err();
        assert!(
            err.to_string().contains("output exceeded"),
            "unexpected error: {err}"
        );
    }
}
