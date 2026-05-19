use rquickjs::Ctx;

pub fn inject_buffer(ctx: &Ctx) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(
        "globalThis.Buffer = class Buffer { constructor(data) { this.data = data || []; } }",
    )?;
    ctx.eval::<(), _>(r#"
        if (typeof globalThis.TextEncoder === 'undefined') {
            globalThis.TextEncoder = function TextEncoder() { this.encoding = 'utf-8'; };
            globalThis.TextEncoder.prototype.encode = function(str) {
                str = String(str || '');
                var bytes = [];
                for (var i = 0; i < str.length; i++) {
                    var c = str.charCodeAt(i);
                    if (c < 0x80) {
                        bytes.push(c);
                    } else if (c < 0x800) {
                        bytes.push((c >> 6) | 0xC0, (c & 0x3F) | 0x80);
                    } else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < str.length) {
                        var c2 = str.charCodeAt(++i);
                        var cp = 0x10000 + ((c - 0xD800) << 10) + (c2 - 0xDC00);
                        bytes.push((cp >> 18) | 0xF0, ((cp >> 12) & 0x3F) | 0x80, ((cp >> 6) & 0x3F) | 0x80, (cp & 0x3F) | 0x80);
                    } else {
                        bytes.push((c >> 12) | 0xE0, ((c >> 6) & 0x3F) | 0x80, (c & 0x3F) | 0x80);
                    }
                }
                return new Uint8Array(bytes);
            };
        }
        if (typeof globalThis.TextDecoder === 'undefined') {
            globalThis.TextDecoder = function TextDecoder(enc) { this.encoding = enc || 'utf-8'; };
            globalThis.TextDecoder.prototype.decode = function(buf) {
                if (!buf) return '';
                var bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
                var str = '';
                var i = 0;
                while (i < bytes.length) {
                    var b = bytes[i++];
                    if (b < 0x80) { str += String.fromCharCode(b); }
                    else if ((b & 0xE0) === 0xC0) { str += String.fromCharCode(((b & 0x1F) << 6) | (bytes[i++] & 0x3F)); }
                    else if ((b & 0xF0) === 0xE0) { str += String.fromCharCode(((b & 0x0F) << 12) | ((bytes[i++] & 0x3F) << 6) | (bytes[i++] & 0x3F)); }
                    else { i += 3; str += '?'; }
                }
                return str;
            };
        }
    "#)?;
    Ok(())
}
