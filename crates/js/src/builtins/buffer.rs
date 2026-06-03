use rquickjs::Ctx;

pub fn inject_buffer(ctx: &Ctx) -> rquickjs::Result<()> {
    // ── TextEncoder / TextDecoder ─────────────────────────────────────────────
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
                        bytes.push((cp>>18)|0xF0,((cp>>12)&0x3F)|0x80,((cp>>6)&0x3F)|0x80,(cp&0x3F)|0x80);
                    } else {
                        bytes.push((c>>12)|0xE0,((c>>6)&0x3F)|0x80,(c&0x3F)|0x80);
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
                var str = '', i = 0;
                while (i < bytes.length) {
                    var b = bytes[i++];
                    if (b < 0x80) { str += String.fromCharCode(b); }
                    else if ((b & 0xE0) === 0xC0) { str += String.fromCharCode(((b&0x1F)<<6)|(bytes[i++]&0x3F)); }
                    else if ((b & 0xF0) === 0xE0) { str += String.fromCharCode(((b&0x0F)<<12)|((bytes[i++]&0x3F)<<6)|(bytes[i++]&0x3F)); }
                    else { i += 3; str += '�'; }
                }
                return str;
            };
        }
    "#)?;

    // ── Buffer — real Uint8Array subclass via prototype swap ──────────────────
    //
    // Strategy: construct a real Uint8Array, then swap its prototype to
    // Buffer.prototype (which itself inherits from Uint8Array.prototype).
    // This gives:
    //   buf instanceof Uint8Array  → true   (prototype chain)
    //   buf instanceof Buffer      → true
    //   buf[0]                     → correct byte value (native TypedArray proxy)
    //   buf.readUInt8(0)           → works (Buffer method)
    ctx.eval::<(), _>(r#"
(function() {
  // ── string ↔ bytes helpers ─────────────────────────────────────────────────
  function _encodeString(str, enc) {
    enc = (enc || 'utf8').toLowerCase().replace(/[^a-z0-9]/g, '');
    if (enc === 'hex') {
      var b = [];
      for (var i = 0; i < str.length; i += 2) b.push(parseInt(str.substr(i,2),16)||0);
      return new Uint8Array(b);
    }
    if (enc === 'base64' || enc === 'base64url') {
      var s = str.replace(/-/g,'+').replace(/_/g,'/');
      while (s.length % 4) s += '=';
      var bin = atob(s), b = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) b[i] = bin.charCodeAt(i);
      return b;
    }
    // latin1 / binary / ascii — byte-preserving: each char maps to its low byte
    if (enc === 'latin1' || enc === 'binary' || enc === 'ascii') {
      var b = new Uint8Array(str.length);
      for (var i = 0; i < str.length; i++) b[i] = str.charCodeAt(i) & 0xFF;
      return b;
    }
    // utf8 (default)
    if (typeof TextEncoder !== 'undefined') return new TextEncoder().encode(str);
    var b = new Uint8Array(str.length);
    for (var i = 0; i < str.length; i++) b[i] = str.charCodeAt(i) & 0xFF;
    return b;
  }

  function _decodeBytes(bytes, enc, start, end) {
    var b = bytes.subarray(start||0, end!==undefined ? end : bytes.length);
    enc = (enc || 'utf8').toLowerCase().replace(/[^a-z0-9]/g, '');
    if (enc === 'hex') {
      var h = '';
      for (var i=0;i<b.length;i++) h += ('0'+b[i].toString(16)).slice(-2);
      return h;
    }
    if (enc === 'base64') {
      var s = '';
      for (var i=0;i<b.length;i++) s += String.fromCharCode(b[i]);
      return btoa(s);
    }
    if (enc === 'base64url') {
      var s = '';
      for (var i=0;i<b.length;i++) s += String.fromCharCode(b[i]);
      return btoa(s).replace(/\+/g,'-').replace(/\//g,'_').replace(/=/g,'');
    }
    if (typeof TextDecoder !== 'undefined') {
      return new TextDecoder(enc === 'latin1' || enc === 'binary' ? 'latin1' : 'utf-8').decode(b);
    }
    var s = '';
    for (var i=0;i<b.length;i++) s += String.fromCharCode(b[i]);
    return s;
  }

  // ── Buffer constructor ─────────────────────────────────────────────────────
  // Returns a real Uint8Array with Buffer.prototype in its chain.
  function Buffer(arg, enc) {
    if (!(this instanceof Buffer)) return new Buffer(arg, enc);
    var bytes;
    if (typeof arg === 'number') {
      bytes = new Uint8Array(arg < 0 ? 0 : arg);
    } else if (arg instanceof Uint8Array) {
      bytes = new Uint8Array(arg); // copy
    } else if (arg instanceof ArrayBuffer) {
      bytes = new Uint8Array(arg);
    } else if (Array.isArray(arg)) {
      bytes = new Uint8Array(arg);
    } else if (typeof arg === 'string') {
      bytes = _encodeString(arg, enc || 'utf8');
    } else if (arg && typeof arg === 'object' && arg.type === 'Buffer' && Array.isArray(arg.data)) {
      bytes = new Uint8Array(arg.data);
    } else {
      bytes = new Uint8Array(0);
    }
    // Swap prototype so the resulting object is a Buffer AND a Uint8Array
    Object.setPrototypeOf(bytes, Buffer.prototype);
    return bytes;
  }

  // Buffer.prototype inherits from Uint8Array.prototype so:
  //   new Buffer(n) instanceof Uint8Array  → true
  Object.setPrototypeOf(Buffer.prototype, Uint8Array.prototype);
  Buffer.prototype.constructor = Buffer;

  // ── Static factory methods ─────────────────────────────────────────────────
  Buffer.from = function(data, enc, len) {
    if (typeof data === 'string') return new Buffer(_encodeString(data, enc || 'utf8'));
    if (data instanceof Uint8Array) return new Buffer(new Uint8Array(data));
    if (data instanceof ArrayBuffer) return new Buffer(new Uint8Array(data));
    if (Array.isArray(data)) return new Buffer(new Uint8Array(data));
    if (data && typeof data === 'object' && data.type === 'Buffer') return new Buffer(new Uint8Array(data.data));
    throw new TypeError('Buffer.from: unsupported argument type ' + typeof data);
  };

  Buffer.alloc = function(size, fill, enc) {
    var b = new Buffer(size < 0 ? 0 : size);
    if (fill !== undefined && size > 0) {
      if (typeof fill === 'number') {
        b.fill(fill & 0xFF);
      } else {
        var fb = _encodeString(String(fill), enc || 'utf8');
        for (var i = 0; i < size; i++) b[i] = fb[i % fb.length];
      }
    }
    return b;
  };
  Buffer.allocUnsafe = function(size) { return new Buffer(size < 0 ? 0 : size); };
  Buffer.allocUnsafeSlow = Buffer.allocUnsafe;

  Buffer.isBuffer = function(obj) { return obj instanceof Buffer || obj instanceof Uint8Array; };
  Buffer.isEncoding = function(enc) {
    return ['utf8','utf-8','hex','base64','base64url','ascii','latin1','binary','ucs2','ucs-2','utf16le','utf-16le']
      .indexOf((enc||'').toLowerCase()) !== -1;
  };
  Buffer.byteLength = function(str, enc) { return _encodeString(String(str), enc || 'utf8').length; };

  Buffer.concat = function(list, totalLen) {
    if (!Array.isArray(list)) throw new TypeError('list must be an Array');
    var len = 0;
    for (var i = 0; i < list.length; i++) len += list[i].length;
    if (totalLen !== undefined) len = Math.min(len, totalLen);
    var result = new Uint8Array(len);
    var offset = 0;
    for (var i = 0; i < list.length; i++) {
      // Accept Buffer (which IS a Uint8Array now) or plain Uint8Array
      var src = list[i] instanceof Uint8Array ? list[i] : new Uint8Array(list[i]);
      var chunk = Math.min(src.length, len - offset);
      result.set(src.subarray(0, chunk), offset);
      offset += chunk;
      if (offset >= len) break;
    }
    return new Buffer(result);
  };

  Buffer.compare = function(a, b) { return a.compare(b); };

  // ── Instance methods ───────────────────────────────────────────────────────
  // `this` IS the Uint8Array — no ._b indirection needed.

  Buffer.prototype.toString = function(enc, start, end) {
    return _decodeBytes(this, enc || 'utf8', start, end);
  };

  Buffer.prototype.slice = function(s, e) {
    return new Buffer(this.subarray(s || 0, e !== undefined ? e : this.length));
  };
  // Override Uint8Array.subarray to return a Buffer
  Buffer.prototype.subarray = function(s, e) {
    return new Buffer(Uint8Array.prototype.subarray.call(this, s, e));
  };

  Buffer.prototype.fill = function(val, s, e) {
    s = s || 0;
    e = e !== undefined ? e : this.length;
    var v = typeof val === 'number' ? val & 0xFF : (val.charCodeAt ? val.charCodeAt(0) & 0xFF : 0);
    for (var i = s; i < e; i++) this[i] = v;
    return this;
  };

  Buffer.prototype.copy = function(tgt, tStart, sStart, sEnd) {
    tStart = tStart || 0;
    sStart = sStart || 0;
    sEnd = sEnd !== undefined ? sEnd : this.length;
    var src = this.subarray(sStart, sEnd);
    var dest = tgt instanceof Uint8Array ? tgt : new Uint8Array(tgt);
    dest.set(src.subarray(0, Math.min(src.length, dest.length - tStart)), tStart);
    return src.length;
  };

  Buffer.prototype.equals = function(o) {
    var ob = o instanceof Uint8Array ? o : new Uint8Array(o);
    if (this.length !== ob.length) return false;
    for (var i = 0; i < this.length; i++) if (this[i] !== ob[i]) return false;
    return true;
  };

  Buffer.prototype.compare = function(o, ts, te, ss, se) {
    var a = this.subarray(ss || 0, se !== undefined ? se : this.length);
    var ob = o instanceof Uint8Array ? o : new Uint8Array(o);
    var b = ob.subarray(ts || 0, te !== undefined ? te : ob.length);
    for (var i = 0; i < Math.min(a.length, b.length); i++) {
      if (a[i] < b[i]) return -1;
      if (a[i] > b[i]) return 1;
    }
    return a.length - b.length;
  };

  Buffer.prototype.indexOf = function(val, off, enc) {
    off = off || 0;
    var s = typeof val === 'string' ? _encodeString(val, enc || 'utf8')
          : val instanceof Uint8Array ? val
          : new Uint8Array([val & 0xFF]);
    for (var i = off; i <= this.length - s.length; i++) {
      var ok = true;
      for (var j = 0; j < s.length; j++) { if (this[i+j] !== s[j]) { ok = false; break; } }
      if (ok) return i;
    }
    return -1;
  };

  Buffer.prototype.lastIndexOf = function(val, off) {
    var s = typeof val === 'string' ? _encodeString(val, 'utf8')
          : val instanceof Uint8Array ? val
          : new Uint8Array([val & 0xFF]);
    var start = off !== undefined ? Math.min(off, this.length - s.length) : this.length - s.length;
    for (var i = start; i >= 0; i--) {
      var ok = true;
      for (var j = 0; j < s.length; j++) { if (this[i+j] !== s[j]) { ok = false; break; } }
      if (ok) return i;
    }
    return -1;
  };

  Buffer.prototype.includes = function(val, off) { return this.indexOf(val, off) !== -1; };

  Buffer.prototype.write = function(str, offset, length, enc) {
    if (typeof offset === 'string') { enc = offset; offset = 0; length = this.length; }
    else if (typeof length === 'string') { enc = length; length = this.length - (offset || 0); }
    offset = offset || 0;
    length = length !== undefined ? length : this.length - offset;
    var bytes = _encodeString(str, enc || 'utf8');
    var n = Math.min(bytes.length, length);
    for (var i = 0; i < n; i++) this[offset + i] = bytes[i];
    return n;
  };

  Buffer.prototype.toJSON = function() {
    return { type: 'Buffer', data: Array.from(this) };
  };

  Buffer.prototype.swap16 = function() {
    for (var i = 0; i < this.length; i += 2) {
      var t = this[i]; this[i] = this[i+1]; this[i+1] = t;
    }
    return this;
  };
  Buffer.prototype.swap32 = function() {
    for (var i = 0; i < this.length; i += 4) {
      var a = this[i], b = this[i+1], c = this[i+2], d = this[i+3];
      this[i] = d; this[i+1] = c; this[i+2] = b; this[i+3] = a;
    }
    return this;
  };

  // ── Integer reads (using `this` as Uint8Array) ─────────────────────────────
  Buffer.prototype.readUInt8    = function(o) { return this[o] >>> 0; };
  Buffer.prototype.readInt8     = function(o) { var v = this[o]; return v >= 0x80 ? v - 0x100 : v; };
  Buffer.prototype.readUInt16LE = function(o) { return (this[o] | (this[o+1] << 8)) >>> 0; };
  Buffer.prototype.readUInt16BE = function(o) { return ((this[o] << 8) | this[o+1]) >>> 0; };
  Buffer.prototype.readInt16LE  = function(o) { var v = this.readUInt16LE(o); return v >= 0x8000 ? v - 0x10000 : v; };
  Buffer.prototype.readInt16BE  = function(o) { var v = this.readUInt16BE(o); return v >= 0x8000 ? v - 0x10000 : v; };
  Buffer.prototype.readUInt32LE = function(o) { return ((this[o] | (this[o+1]<<8) | (this[o+2]<<16)) + this[o+3]*0x1000000) >>> 0; };
  Buffer.prototype.readUInt32BE = function(o) { return (this[o]*0x1000000 + ((this[o+1]<<16) | (this[o+2]<<8) | this[o+3])) >>> 0; };
  Buffer.prototype.readInt32LE  = function(o) { return this[o] | (this[o+1]<<8) | (this[o+2]<<16) | (this[o+3]<<24); };
  Buffer.prototype.readInt32BE  = function(o) { return (this[o]<<24) | (this[o+1]<<16) | (this[o+2]<<8) | this[o+3]; };
  Buffer.prototype.readFloatLE  = function(o) { return new DataView(this.buffer, this.byteOffset).getFloat32(o, true); };
  Buffer.prototype.readFloatBE  = function(o) { return new DataView(this.buffer, this.byteOffset).getFloat32(o, false); };
  Buffer.prototype.readDoubleLE = function(o) { return new DataView(this.buffer, this.byteOffset).getFloat64(o, true); };
  Buffer.prototype.readDoubleBE = function(o) { return new DataView(this.buffer, this.byteOffset).getFloat64(o, false); };
  Buffer.prototype.readBigUInt64LE = function(o) {
    var lo = this.readUInt32LE(o), hi = this.readUInt32LE(o+4);
    return BigInt(lo) + BigInt(hi) * BigInt(0x100000000);
  };
  Buffer.prototype.readBigUInt64BE = function(o) {
    var hi = this.readUInt32BE(o), lo = this.readUInt32BE(o+4);
    return BigInt(hi) * BigInt(0x100000000) + BigInt(lo);
  };
  Buffer.prototype.readBigInt64LE = function(o) {
    var v = this.readBigUInt64LE(o);
    return v >= BigInt('9223372036854775808') ? v - BigInt('18446744073709551616') : v;
  };
  Buffer.prototype.readBigInt64BE = function(o) {
    var v = this.readBigUInt64BE(o);
    return v >= BigInt('9223372036854775808') ? v - BigInt('18446744073709551616') : v;
  };

  // ── Integer writes ─────────────────────────────────────────────────────────
  Buffer.prototype.writeUInt8    = function(v,o) { this[o] = v & 0xFF; return o+1; };
  Buffer.prototype.writeInt8     = function(v,o) { this[o] = (v < 0 ? v + 0x100 : v) & 0xFF; return o+1; };
  Buffer.prototype.writeUInt16LE = function(v,o) { this[o] = v & 0xFF; this[o+1] = (v>>8) & 0xFF; return o+2; };
  Buffer.prototype.writeUInt16BE = function(v,o) { this[o] = (v>>8) & 0xFF; this[o+1] = v & 0xFF; return o+2; };
  Buffer.prototype.writeUInt32LE = function(v,o) { this[o]=v&0xFF;this[o+1]=(v>>8)&0xFF;this[o+2]=(v>>16)&0xFF;this[o+3]=(v>>>24)&0xFF;return o+4; };
  Buffer.prototype.writeUInt32BE = function(v,o) { this[o]=(v>>>24)&0xFF;this[o+1]=(v>>16)&0xFF;this[o+2]=(v>>8)&0xFF;this[o+3]=v&0xFF;return o+4; };
  Buffer.prototype.writeInt32LE  = Buffer.prototype.writeUInt32LE;
  Buffer.prototype.writeInt32BE  = Buffer.prototype.writeUInt32BE;
  Buffer.prototype.writeFloatLE  = function(v,o) { new DataView(this.buffer,this.byteOffset).setFloat32(o,v,true); return o+4; };
  Buffer.prototype.writeFloatBE  = function(v,o) { new DataView(this.buffer,this.byteOffset).setFloat32(o,v,false); return o+4; };
  Buffer.prototype.writeDoubleLE = function(v,o) { new DataView(this.buffer,this.byteOffset).setFloat64(o,v,true); return o+8; };
  Buffer.prototype.writeDoubleBE = function(v,o) { new DataView(this.buffer,this.byteOffset).setFloat64(o,v,false); return o+8; };
  Buffer.prototype.writeBigUInt64LE = function(v,o) {
    var lo = BigInt.asUintN(32, v), hi = BigInt.asUintN(32, v >> BigInt(32));
    this.writeUInt32LE(Number(lo), o); this.writeUInt32LE(Number(hi), o+4); return o+8;
  };
  Buffer.prototype.writeBigUInt64BE = function(v,o) {
    var lo = BigInt.asUintN(32, v), hi = BigInt.asUintN(32, v >> BigInt(32));
    this.writeUInt32BE(Number(hi), o); this.writeUInt32BE(Number(lo), o+4); return o+8;
  };
  Buffer.prototype.writeBigInt64LE = Buffer.prototype.writeBigUInt64LE;
  Buffer.prototype.writeBigInt64BE = Buffer.prototype.writeBigUInt64BE;

  // Int aliases (Node.js compatibility)
  Buffer.prototype.readUInt8 = Buffer.prototype.readUint8 = Buffer.prototype.readUInt8;
  Buffer.prototype.readUInt16LE = Buffer.prototype.readUint16LE = Buffer.prototype.readUInt16LE;
  Buffer.prototype.readUInt16BE = Buffer.prototype.readUint16BE = Buffer.prototype.readUInt16BE;
  Buffer.prototype.readUInt32LE = Buffer.prototype.readUint32LE = Buffer.prototype.readUInt32LE;
  Buffer.prototype.readUInt32BE = Buffer.prototype.readUint32BE = Buffer.prototype.readUInt32BE;

  globalThis.Buffer = Buffer;
})();
    "#)?;

    Ok(())
}
