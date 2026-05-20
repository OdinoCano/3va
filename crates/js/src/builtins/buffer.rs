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

    // ── Buffer (Node.js compatible polyfill over Uint8Array) ──────────────────
    ctx.eval::<(), _>(r#"
(function() {
  function _encodeString(str, enc) {
    enc = (enc || 'utf8').toLowerCase().replace(/[^a-z0-9]/g, '');
    if (enc === 'hex') {
      var b = []; for (var i = 0; i < str.length; i += 2) b.push(parseInt(str.substr(i,2),16)||0);
      return new Uint8Array(b);
    }
    if (enc === 'base64' || enc === 'base64url') {
      var s = str.replace(/-/g,'+').replace(/_/g,'/');
      while (s.length % 4) s += '=';
      var bin = atob(s), b = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) b[i] = bin.charCodeAt(i);
      return b;
    }
    // utf8 / ascii / latin1
    if (typeof TextEncoder !== 'undefined') return new TextEncoder().encode(str);
    var b = new Uint8Array(str.length);
    for (var i = 0; i < str.length; i++) b[i] = str.charCodeAt(i) & 0xFF;
    return b;
  }

  function _decodeBytes(bytes, enc, start, end) {
    var b = bytes.subarray(start||0, end!==undefined ? end : bytes.length);
    enc = (enc || 'utf8').toLowerCase().replace(/[^a-z0-9]/g, '');
    if (enc === 'hex') {
      var h = ''; for (var i=0;i<b.length;i++) h += ('0'+b[i].toString(16)).slice(-2); return h;
    }
    if (enc === 'base64') {
      var s = ''; for (var i=0;i<b.length;i++) s += String.fromCharCode(b[i]); return btoa(s);
    }
    if (typeof TextDecoder !== 'undefined') return new TextDecoder(enc === 'latin1' ? 'latin1' : 'utf-8').decode(b);
    var s = ''; for (var i=0;i<b.length;i++) s += String.fromCharCode(b[i]); return s;
  }

  function Buffer(arg, enc) {
    if (!(this instanceof Buffer)) return new Buffer(arg, enc);
    var bytes;
    if (typeof arg === 'number') {
      bytes = new Uint8Array(arg < 0 ? 0 : arg);
    } else if (arg instanceof Uint8Array) {
      bytes = arg;
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
    this._b = bytes;
    this.length = bytes.length;
  }

  Buffer.prototype[Symbol.iterator] = function() { return this._b[Symbol.iterator](); };

  Buffer.from = function(data, enc, len) {
    if (typeof data === 'string') return new Buffer(_encodeString(data, enc||'utf8'));
    if (data instanceof Uint8Array) return new Buffer(new Uint8Array(data));
    if (data instanceof ArrayBuffer) return new Buffer(new Uint8Array(data));
    if (Array.isArray(data)) return new Buffer(new Uint8Array(data));
    if (data && typeof data === 'object' && data.type === 'Buffer') return new Buffer(new Uint8Array(data.data));
    throw new TypeError('Buffer.from: unsupported argument type');
  };

  Buffer.alloc = function(size, fill, enc) {
    var b = new Buffer(size < 0 ? 0 : size);
    if (fill !== undefined && size > 0) {
      if (typeof fill === 'number') {
        b._b.fill(fill & 0xFF);
      } else {
        var fb = _encodeString(String(fill), enc||'utf8');
        for (var i = 0; i < size; i++) b._b[i] = fb[i % fb.length];
      }
    }
    return b;
  };
  Buffer.allocUnsafe = function(size) { return new Buffer(size < 0 ? 0 : size); };
  Buffer.allocUnsafeSlow = Buffer.allocUnsafe;

  Buffer.isBuffer = function(obj) { return obj instanceof Buffer; };
  Buffer.isEncoding = function(enc) {
    return ['utf8','utf-8','hex','base64','base64url','ascii','latin1','binary','ucs2','ucs-2','utf16le','utf-16le'].indexOf((enc||'').toLowerCase()) !== -1;
  };
  Buffer.byteLength = function(str, enc) { return _encodeString(String(str), enc||'utf8').length; };

  Buffer.concat = function(list, totalLen) {
    if (!Array.isArray(list)) throw new TypeError('list must be an Array');
    var len = 0;
    for (var i = 0; i < list.length; i++) len += list[i].length;
    if (totalLen !== undefined) len = Math.min(len, totalLen);
    var result = new Uint8Array(len), offset = 0;
    for (var i = 0; i < list.length; i++) {
      var src = list[i]._b || list[i];
      var chunk = Math.min(src.length, len - offset);
      result.set(src.subarray(0, chunk), offset);
      offset += chunk; if (offset >= len) break;
    }
    return new Buffer(result);
  };

  Buffer.compare = function(a, b) { return a.compare(b); };

  Buffer.prototype.toString = function(enc, start, end) {
    return _decodeBytes(this._b, enc||'utf8', start, end);
  };
  Buffer.prototype.slice   = function(s, e) { return new Buffer(this._b.slice(s||0, e!==undefined?e:this._b.length)); };
  Buffer.prototype.subarray = Buffer.prototype.slice;
  Buffer.prototype.fill = function(val, s, e) {
    s = s||0; e = e!==undefined?e:this._b.length;
    var v = typeof val === 'number' ? val & 0xFF : (val.charCodeAt ? val.charCodeAt(0) & 0xFF : 0);
    for (var i=s;i<e;i++) this._b[i]=v; return this;
  };
  Buffer.prototype.copy = function(tgt, tStart, sStart, sEnd) {
    tStart=tStart||0; sStart=sStart||0; sEnd=sEnd!==undefined?sEnd:this._b.length;
    var src = this._b.subarray(sStart, sEnd);
    (tgt._b||tgt).set(src.subarray(0, Math.min(src.length, (tgt._b||tgt).length - tStart)), tStart);
    return src.length;
  };
  Buffer.prototype.equals  = function(o) { if(this._b.length!==o._b.length) return false; for(var i=0;i<this._b.length;i++) if(this._b[i]!==o._b[i]) return false; return true; };
  Buffer.prototype.compare = function(o, ts, te, ss, se) {
    var a=this._b.subarray(ss||0,se!==undefined?se:this._b.length),
        b=(o._b||o).subarray(ts||0,te!==undefined?te:o._b.length);
    for(var i=0;i<Math.min(a.length,b.length);i++){if(a[i]<b[i])return -1;if(a[i]>b[i])return 1;} return a.length-b.length;
  };
  Buffer.prototype.indexOf = function(val, off, enc) {
    off=off||0;
    var s = typeof val==='string'?_encodeString(val,enc||'utf8'):val instanceof Buffer?val._b:new Uint8Array([val&0xFF]);
    for(var i=off;i<=this._b.length-s.length;i++){var ok=true;for(var j=0;j<s.length;j++){if(this._b[i+j]!==s[j]){ok=false;break;}}if(ok)return i;} return -1;
  };
  Buffer.prototype.lastIndexOf = function(val, off) {
    var s = typeof val==='string'?_encodeString(val,'utf8'):val instanceof Buffer?val._b:new Uint8Array([val&0xFF]);
    var start = off!==undefined ? Math.min(off, this._b.length-s.length) : this._b.length-s.length;
    for(var i=start;i>=0;i--){var ok=true;for(var j=0;j<s.length;j++){if(this._b[i+j]!==s[j]){ok=false;break;}}if(ok)return i;} return -1;
  };
  Buffer.prototype.includes = function(val, off) { return this.indexOf(val, off) !== -1; };
  Buffer.prototype.keys     = function() { return this._b.keys(); };
  Buffer.prototype.values   = function() { return this._b.values(); };
  Buffer.prototype.entries  = function() { return this._b.entries(); };
  Buffer.prototype.toJSON   = function() { return { type:'Buffer', data: Array.from(this._b) }; };
  Buffer.prototype.swap16   = function() { for(var i=0;i<this._b.length;i+=2){var t=this._b[i];this._b[i]=this._b[i+1];this._b[i+1]=t;} return this; };
  Buffer.prototype.swap32   = function() { for(var i=0;i<this._b.length;i+=4){var a=this._b,t=[a[i],a[i+1],a[i+2],a[i+3]];a[i]=t[3];a[i+1]=t[2];a[i+2]=t[1];a[i+3]=t[0];} return this; };

  // Integer reads
  Buffer.prototype.readUInt8 = function(o){return this._b[o]>>>0;};
  Buffer.prototype.readInt8  = function(o){var v=this._b[o];return v>=0x80?v-0x100:v;};
  Buffer.prototype.readUInt16LE = function(o){return(this._b[o]|(this._b[o+1]<<8))>>>0;};
  Buffer.prototype.readUInt16BE = function(o){return((this._b[o]<<8)|this._b[o+1])>>>0;};
  Buffer.prototype.readInt16LE  = function(o){var v=this.readUInt16LE(o);return v>=0x8000?v-0x10000:v;};
  Buffer.prototype.readInt16BE  = function(o){var v=this.readUInt16BE(o);return v>=0x8000?v-0x10000:v;};
  Buffer.prototype.readUInt32LE = function(o){return((this._b[o]|(this._b[o+1]<<8)|(this._b[o+2]<<16))+this._b[o+3]*0x1000000)>>>0;};
  Buffer.prototype.readUInt32BE = function(o){return(this._b[o]*0x1000000+((this._b[o+1]<<16)|(this._b[o+2]<<8)|this._b[o+3]))>>>0;};
  Buffer.prototype.readInt32LE  = function(o){return this._b[o]|(this._b[o+1]<<8)|(this._b[o+2]<<16)|(this._b[o+3]<<24);};
  Buffer.prototype.readInt32BE  = function(o){return(this._b[o]<<24)|(this._b[o+1]<<16)|(this._b[o+2]<<8)|this._b[o+3];};
  Buffer.prototype.readFloatLE  = function(o){var dv=new DataView(this._b.buffer,this._b.byteOffset);return dv.getFloat32(o,true);};
  Buffer.prototype.readFloatBE  = function(o){var dv=new DataView(this._b.buffer,this._b.byteOffset);return dv.getFloat32(o,false);};
  Buffer.prototype.readDoubleLE = function(o){var dv=new DataView(this._b.buffer,this._b.byteOffset);return dv.getFloat64(o,true);};
  Buffer.prototype.readDoubleBE = function(o){var dv=new DataView(this._b.buffer,this._b.byteOffset);return dv.getFloat64(o,false);};

  // Integer writes
  Buffer.prototype.writeUInt8    = function(v,o){this._b[o]=v&0xFF;return o+1;};
  Buffer.prototype.writeInt8     = function(v,o){this._b[o]=(v<0?v+0x100:v)&0xFF;return o+1;};
  Buffer.prototype.writeUInt16LE = function(v,o){this._b[o]=v&0xFF;this._b[o+1]=(v>>8)&0xFF;return o+2;};
  Buffer.prototype.writeUInt16BE = function(v,o){this._b[o]=(v>>8)&0xFF;this._b[o+1]=v&0xFF;return o+2;};
  Buffer.prototype.writeUInt32LE = function(v,o){this._b[o]=v&0xFF;this._b[o+1]=(v>>8)&0xFF;this._b[o+2]=(v>>16)&0xFF;this._b[o+3]=(v>>>24)&0xFF;return o+4;};
  Buffer.prototype.writeUInt32BE = function(v,o){this._b[o]=(v>>>24)&0xFF;this._b[o+1]=(v>>16)&0xFF;this._b[o+2]=(v>>8)&0xFF;this._b[o+3]=v&0xFF;return o+4;};
  Buffer.prototype.writeInt32LE  = Buffer.prototype.writeUInt32LE;
  Buffer.prototype.writeInt32BE  = Buffer.prototype.writeUInt32BE;
  Buffer.prototype.writeFloatLE  = function(v,o){var dv=new DataView(this._b.buffer,this._b.byteOffset);dv.setFloat32(o,v,true);return o+4;};
  Buffer.prototype.writeFloatBE  = function(v,o){var dv=new DataView(this._b.buffer,this._b.byteOffset);dv.setFloat32(o,v,false);return o+4;};
  Buffer.prototype.writeDoubleLE = function(v,o){var dv=new DataView(this._b.buffer,this._b.byteOffset);dv.setFloat64(o,v,true);return o+8;};
  Buffer.prototype.writeDoubleBE = function(v,o){var dv=new DataView(this._b.buffer,this._b.byteOffset);dv.setFloat64(o,v,false);return o+8;};

  Object.defineProperty(Buffer.prototype,'buffer',{get:function(){return this._b.buffer;}});
  Object.defineProperty(Buffer.prototype,'byteOffset',{get:function(){return this._b.byteOffset;}});
  Object.defineProperty(Buffer.prototype,'BYTES_PER_ELEMENT',{get:function(){return 1;}});

  globalThis.Buffer = Buffer;
})();
    "#)?;

    Ok(())
}
