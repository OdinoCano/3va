use v8::{ContextScope, HandleScope, Script};

pub fn inject_web_globals(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    let web_globals_code = r#"
    (function() {
        'use strict';

        // ── self and navigator ──────────────────────────────────────────────────
        globalThis.self = globalThis;
        globalThis.navigator = {
            userAgent: '3va/2.4.0',
            onLine: true,
        };

        // ── Blob ────────────────────────────────────────────────────────────────
        var _blobId = 0;
        function Blob(parts, options) {
            if (!(this instanceof Blob)) {
                return new Blob(parts, options);
            }
            parts = parts || [];
            this._id = ++_blobId;
            this._type = (options && options.type) || '';
            this._parts = [];
            var totalSize = 0;
            for (var i = 0; i < parts.length; i++) {
                var part = parts[i];
                if (part instanceof Blob) {
                    this._parts.push(part._parts);
                    totalSize += part.size;
                } else if (typeof part === 'string') {
                    this._parts.push(part);
                    totalSize += part.length;
                } else if (part instanceof Uint8Array) {
                    this._parts.push(part);
                    totalSize += part.length;
                } else if (part instanceof ArrayBuffer) {
                    this._parts.push(new Uint8Array(part));
                    totalSize += part.byteLength;
                } else if (ArrayBuffer.isView(part)) {
                    var arr = new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
                    this._parts.push(arr);
                    totalSize += part.byteLength;
                } else {
                    var str = String(part);
                    this._parts.push(str);
                    totalSize += str.length;
                }
            }
            this._size = totalSize;
            Object.defineProperty(this, 'size', { get: function() { return this._size; } });
            Object.defineProperty(this, 'type', { get: function() { return this._type; } });
        }
        Blob.prototype.text = function() {
            var blob = this;
            return new Promise(function(resolve) {
                var result = '';
                var parts = blob._parts;
                for (var i = 0; i < parts.length; i++) {
                    var part = parts[i];
                    if (part instanceof Uint8Array) {
                        result += String.fromCharCode.apply(null, part);
                    } else if (ArrayBuffer.isView(part)) {
                        var arr = new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
                        result += String.fromCharCode.apply(null, arr);
                    } else if (part instanceof ArrayBuffer) {
                        var arr = new Uint8Array(part);
                        result += String.fromCharCode.apply(null, arr);
                    } else {
                        result += part;
                    }
                }
                resolve(result);
            });
        };
        Blob.prototype.arrayBuffer = function() {
            var blob = this;
            return new Promise(function(resolve) {
                var result = new Uint8Array(blob.size);
                var offset = 0;
                for (var i = 0; i < blob._parts.length; i++) {
                    var part = blob._parts[i];
                    if (part instanceof Uint8Array) {
                        result.set(part, offset);
                        offset += part.length;
                    } else if (typeof part === 'string') {
                        for (var j = 0; j < part.length; j++) {
                            result[offset++] = part.charCodeAt(j) & 0xFF;
                        }
                    }
                }
                resolve(result.buffer);
            });
        };
        Blob.prototype.bytes = function() {
            return this.arrayBuffer().then(function(buffer) {
                return new Uint8Array(buffer);
            });
        };
        Blob.prototype.slice = function(start, end, type) {
            var s = start || 0;
            var e = end !== undefined ? end : this.size;
            if (s < 0) s = Math.max(this.size + s, 0);
            if (e < 0) e = Math.max(this.size + e, 0);
            s = Math.min(s, this.size);
            e = Math.min(e, this.size);
            if (s >= e) return new Blob([], { type: type !== undefined ? type : this.type });
            var content = '';
            var consumed = 0;
            var remaining = e - s;
            for (var i = 0; i < this._parts.length && remaining > 0; i++) {
                var part = this._parts[i];
                var partLen = part instanceof Uint8Array ? part.length : part.length;
                var skip = Math.max(0, s - consumed);
                var take = Math.min(partLen - skip, remaining);
                if (take <= 0) {
                    consumed += partLen;
                    continue;
                }
                if (part instanceof Uint8Array) {
                    for (var j = skip; j < skip + take; j++) {
                        content += String.fromCharCode(part[j]);
                    }
                } else if (typeof part === 'string') {
                    content += part.substr(skip, take);
                }
                consumed += partLen;
                remaining -= take;
            }
            return new Blob([content], { type: type !== undefined ? type : this.type });
        };
        Blob.prototype.stream = function() {
            var blob = this;
            return new ReadableStream({
                start: function(controller) {
                    blob.text().then(function(text) {
                        controller.enqueue(text);
                        controller.close();
                    });
                }
            });
        };
        globalThis.Blob = Blob;

        // ── File ────────────────────────────────────────────────────────────────
        function File(parts, name, options) {
            if (!(this instanceof File)) {
                return new File(parts, name, options);
            }
            Blob.call(this, parts, options);
            this._name = name;
            this._lastModified = (options && options.lastModified) || Date.now();
            Object.defineProperty(this, 'name', { get: function() { return this._name; } });
            Object.defineProperty(this, 'lastModified', { get: function() { return this._lastModified; } });
        }
        File.prototype = Object.create(Blob.prototype);
        File.prototype.constructor = File;
        globalThis.File = File;

        // ── URL ────────────────────────────────────────────────────────────────
        function URL(url, base) {
            if (!(this instanceof URL)) {
                return new URL(url, base);
            }
            var parsed = URL._parse(url, base);
            if (!parsed.protocol) {
                throw new Error('Invalid URL');
            }
            this._url = url;
            this._href = parsed.href;
            this._origin = parsed.origin;
            this._protocol = parsed.protocol;
            this._host = parsed.host;
            this._hostname = parsed.hostname;
            this._port = parsed.port;
            this._pathname = parsed.pathname;
            this._search = parsed.search;
            this._hash = parsed.hash;
            this._searchParams = null;
        }
        Object.defineProperty(URL.prototype, 'searchParams', {
            get: function() {
                if (this._searchParams === null) {
                    this._searchParams = new URLSearchParams(this._search);
                }
                return this._searchParams;
            }
        });
        URL._parse = function(url, base) {
            var href = url;
            var hash = '';
            var query = '';
            var pathname = '/';
            var host = '';
            var hostname = '';
            var port = '';
            var protocol = '';
            var origin = '';
            var search = '';
            if (base && !href.match(/^[a-z]+:\/\//)) {
                href = URL._resolve(base, href);
            }
            var match = href.match(/^([a-z]+):\/\/([^\/:]*)(?::(\d+))?(\/.*)?$/);
            if (match) {
                protocol = match[1] + ':';
                hostname = match[2];
                port = match[3] || '';
                var isDefaultPort = (protocol === 'https:' && port === '443') ||
                                    (protocol === 'http:' && port === '80');
                host = hostname + (port && !isDefaultPort ? ':' + port : '');
                pathname = match[4] || '/';
                var hashIdx = pathname.indexOf('#');
                if (hashIdx !== -1) {
                    hash = pathname.substring(hashIdx);
                    pathname = pathname.substring(0, hashIdx);
                }
                var queryIdx = pathname.indexOf('?');
                if (queryIdx !== -1) {
                    search = pathname.substring(queryIdx);
                    pathname = pathname.substring(0, queryIdx);
                }
                origin = protocol + '//' + hostname + (port && !isDefaultPort ? ':' + port : '');
            } else if (href.indexOf('#') !== -1) {
                var parts = href.split('#');
                pathname = parts[0];
                hash = '#' + parts[1];
            } else {
                pathname = href;
            }
            return {
                href: href,
                origin: origin,
                protocol: protocol,
                host: host,
                hostname: hostname,
                port: port,
                pathname: pathname,
                search: search,
                hash: hash
            };
        };
        URL._resolve = function(base, href) {
            if (href.indexOf('://') !== -1) return href;
            if (href.startsWith('//')) {
                var baseMatch = base.match(/^([a-z]+):\/\//);
                if (baseMatch) return baseMatch[1] + ':' + href;
            }
            if (href.startsWith('/')) {
                var baseMatch = base.match(/^[a-z]+:\/\/[^\/]+/);
                if (baseMatch) return baseMatch[0] + href;
            }
            var basePath = base.split('/');
            basePath.pop();
            var relPath = href.split('/');
            for (var i = 0; i < relPath.length; i++) {
                if (relPath[i] === '..') basePath.pop();
                else if (relPath[i] !== '.') basePath.push(relPath[i]);
            }
            return basePath.join('/');
        };
        URL.prototype.toString = function() { return this.href; };
        URL.prototype.toJSON = function() { return this.href; };
        Object.defineProperty(URL.prototype, 'href', { get: function() { return this._href; } });
        Object.defineProperty(URL.prototype, 'origin', { get: function() { return this._origin; } });
        Object.defineProperty(URL.prototype, 'protocol', { get: function() { return this._protocol; } });
        Object.defineProperty(URL.prototype, 'host', { get: function() { return this._host; } });
        Object.defineProperty(URL.prototype, 'hostname', { get: function() { return this._hostname; } });
        Object.defineProperty(URL.prototype, 'port', { get: function() { return this._port; } });
        Object.defineProperty(URL.prototype, 'pathname', { get: function() { return this._pathname; } });
        Object.defineProperty(URL.prototype, 'search', { get: function() { return this._search; } });
        Object.defineProperty(URL.prototype, 'hash', { get: function() { return this._hash; } });
        URL.createObjectURL = function(blob) {
            return 'blob:' + Math.random().toString(36).substr(2);
        };
        URL.revokeObjectURL = function(url) {};
        URL.canParse = function(url, base) {
            try { new URL(url, base); return true; } catch(e) { return false; }
        };
        globalThis.URL = URL;

        // ── URLSearchParams ────────────────────────────────────────────────────
        function URLSearchParams(init) {
            if (!(this instanceof URLSearchParams)) {
                return new URLSearchParams(init);
            }
            this._params = [];
            if (typeof init === 'string') {
                if (init.indexOf('?') === 0) init = init.substr(1);
                var pairs = init.split('&');
                for (var i = 0; i < pairs.length; i++) {
                    var pair = pairs[i].split('=');
                    this._params.push({
                        key: decodeURIComponent(pair[0] || ''),
                        value: decodeURIComponent(pair[1] || '')
                    });
                }
            } else if (Array.isArray(init)) {
                for (var i = 0; i < init.length; i++) {
                    this._params.push({
                        key: String(init[i][0]),
                        value: String(init[i][1])
                    });
                }
            } else if (init && typeof init === 'object') {
                for (var key in init) {
                    this._params.push({ key: key, value: String(init[key]) });
                }
            }
        }
        URLSearchParams.prototype.append = function(key, value) {
            this._params.push({ key: key, value: String(value) });
        };
        URLSearchParams.prototype.delete = function(key) {
            this._params = this._params.filter(function(p) { return p.key !== key; });
        };
        URLSearchParams.prototype.get = function(key) {
            for (var i = 0; i < this._params.length; i++) {
                if (this._params[i].key === key) return this._params[i].value;
            }
            return null;
        };
        URLSearchParams.prototype.getAll = function(key) {
            var result = [];
            for (var i = 0; i < this._params.length; i++) {
                if (this._params[i].key === key) result.push(this._params[i].value);
            }
            return result;
        };
        URLSearchParams.prototype.has = function(key) {
            for (var i = 0; i < this._params.length; i++) {
                if (this._params[i].key === key) return true;
            }
            return false;
        };
        URLSearchParams.prototype.set = function(key, value) {
            var found = false;
            this._params = this._params.map(function(p) {
                if (p.key === key) {
                    if (!found) { found = true; return { key: key, value: String(value) }; }
                    return null;
                }
                return p;
            }).filter(function(p) { return p !== null; });
            if (!found) this._params.push({ key: key, value: String(value) });
        };
        URLSearchParams.prototype.forEach = function(fn) {
            for (var i = 0; i < this._params.length; i++) {
                fn(this._params[i].value, this._params[i].key, this);
            }
        };
        URLSearchParams.prototype[Symbol.iterator] = function() {
            var params = this._params;
            var i = 0;
            return {
                next: function() {
                    if (i < params.length) {
                        var value = [params[i].key, params[i].value];
                        i++;
                        return { value: value, done: false };
                    }
                    return { value: undefined, done: true };
                }
            };
        };
        URLSearchParams.prototype.toString = function() {
            return this._params.map(function(p) {
                return encodeURIComponent(p.key) + '=' + encodeURIComponent(p.value);
            }).join('&');
        };
        Object.defineProperty(URLSearchParams.prototype, 'size', { get: function() { return this._params.length; } });
        globalThis.URLSearchParams = URLSearchParams;

        // ── Headers ────────────────────────────────────────────────────────────
        function Headers(init) {
            if (!(this instanceof Headers)) {
                return new Headers(init);
            }
            this._map = {};
            if (init) {
                if (init instanceof Headers) {
                    init.forEach(function(v, k) { this._map[k.toLowerCase()] = v; }, this);
                } else if (typeof init === 'object') {
                    for (var key in init) {
                        this._map[key.toLowerCase()] = String(init[key]);
                    }
                } else if (Array.isArray(init)) {
                    for (var i = 0; i < init.length; i++) {
                        this._map[init[i][0].toLowerCase()] = String(init[i][1]);
                    }
                }
            }
        }
        Headers.prototype.append = function(name, value) {
            var key = name.toLowerCase();
            if (this._map[key]) {
                this._map[key] += ', ' + value;
            } else {
                this._map[key] = value;
            }
        };
        Headers.prototype.delete = function(name) {
            delete this._map[name.toLowerCase()];
        };
        Headers.prototype.get = function(name) {
            return this._map[name.toLowerCase()] || null;
        };
        Headers.prototype.has = function(name) {
            return name.toLowerCase() in this._map;
        };
        Headers.prototype.set = function(name, value) {
            this._map[name.toLowerCase()] = String(value);
        };
        Headers.prototype.forEach = function(fn, thisArg) {
            for (var key in this._map) {
                fn.call(thisArg, this._map[key], key, this);
            }
        };
        Headers.prototype.entries = function() {
            var map = this._map;
            var keys = Object.keys(map);
            var i = 0;
            return {
                next: function() {
                    if (i < keys.length) {
                        var key = keys[i++];
                        return { value: [key, map[key]], done: false };
                    }
                    return { value: undefined, done: true };
                }
            };
        };
        Headers.prototype.keys = function() {
            var map = this._map;
            var keys = Object.keys(map);
            var i = 0;
            return {
                next: function() {
                    if (i < keys.length) {
                        return { value: keys[i++], done: false };
                    }
                    return { value: undefined, done: true };
                }
            };
        };
        Headers.prototype.values = function() {
            var map = this._map;
            var keys = Object.keys(map);
            var i = 0;
            return {
                next: function() {
                    if (i < keys.length) {
                        return { value: map[keys[i++]], done: false };
                    }
                    return { value: undefined, done: true };
                }
            };
        };
        Headers.prototype[Symbol.iterator] = function() { return this.entries(); };
        Headers.prototype.getSetCookie = function() {
            var result = [];
            for (var key in this._map) {
                if (key === 'set-cookie') {
                    result = this._map[key].split(', ').map(function(s) { return s.trim(); });
                }
            }
            return result;
        };
        globalThis.Headers = Headers;

        // ── AbortController / AbortSignal ──────────────────────────────────────
        function AbortSignal() {
            if (!(this instanceof AbortSignal)) return new AbortSignal();
            this.aborted = false;
            this.reason = undefined;
            this._listeners = [];
        }
        AbortSignal.prototype.addEventListener = function(type, listener) {
            if (type === 'abort') {
                this._listeners.push(listener);
            }
        };
        AbortSignal.prototype.removeEventListener = function(type, listener) {
            if (type === 'abort') {
                this._listeners = this._listeners.filter(function(l) { return l !== listener; });
            }
        };
        AbortSignal.abort = function(reason) {
            var signal = new AbortSignal();
            signal.aborted = true;
            signal.reason = reason !== undefined ? reason : new Error('AbortError');
            return signal;
        };
        AbortSignal.timeout = function(ms) {
            var signal = new AbortSignal();
            setTimeout(function() {
                signal.aborted = true;
                signal.reason = new Error('TimeoutError');
                signal._listeners.forEach(function(l) { l.call(signal); });
            }, ms);
            return signal;
        };

        function AbortController() {
            if (!(this instanceof AbortController)) return new AbortController();
            this.signal = new AbortSignal();
        }
        AbortController.prototype.abort = function(reason) {
            this.signal.aborted = true;
            this.signal.reason = reason !== undefined ? reason : new Error('AbortError');
            this.signal._listeners.forEach(function(l) { l.call(this.signal); }, this);
        };
        globalThis.AbortController = AbortController;
        globalThis.AbortSignal = AbortSignal;

        // ── Request ────────────────────────────────────────────────────────────
        function Request(input, init) {
            if (!(this instanceof Request)) return new Request(input, init);
            if (typeof input === 'string') {
                this.url = input;
            } else if (input && input.url) {
                this.url = input.url;
            }
            this.method = 'GET';
            this.headers = new Headers();
            this.body = null;
            this.signal = new AbortSignal();
            if (init) {
                if (init.method) this.method = init.method.toUpperCase();
                if (init.headers) this.headers = new Headers(init.headers);
                if (init.body) this.body = init.body;
                if (init.signal) this.signal = init.signal;
            }
        }
        Request.prototype.clone = function() {
            var r = new Request(this.url, { method: this.method });
            r.headers = new Headers(this.headers);
            r.body = this.body;
            r.signal = this.signal;
            return r;
        };
        globalThis.Request = Request;

        // ── Response ──────────────────────────────────────────────────────────
        function Response(body, init) {
            if (!(this instanceof Response)) return new Response(body, init);
            this._body = body;
            this.status = 200;
            this.statusText = 'OK';
            this.ok = this.status >= 200 && this.status < 300;
            this.headers = new Headers();
            this.url = '';
            this.type = 'default';
            if (init) {
                if (init.status !== undefined) this.status = init.status;
                if (init.statusText !== undefined) this.statusText = init.statusText;
                if (init.headers) this.headers = new Headers(init.headers);
                if (init.url) this.url = init.url;
                if (init.type !== undefined) this.type = init.type;
                this.ok = this.status >= 200 && this.status < 300;
            }
        }
        Response.prototype.clone = function() {
            var r = new Response(this._body, {
                status: this.status,
                statusText: this.statusText,
                headers: new Headers(this.headers),
                url: this.url,
                type: this.type
            });
            return r;
        };
        Response.prototype.text = function() {
            var self = this;
            return Promise.resolve(String(self._body || ''));
        };
        Response.prototype.json = function() {
            var self = this;
            return Promise.resolve(JSON.parse(self._body || '{}'));
        };
        Response.error = function() {
            return new Response(null, { status: 0, statusText: '', type: 'error' });
        };
        Response.redirect = function(url, status) {
            if (status === undefined) status = 302;
            if (status >= 200 && status < 300) {
                throw new Error('Invalid status code for redirect');
            }
            return new Response(null, { status: status, headers: { Location: url } });
        };
        Response.json = function(data, init) {
            var headers = new Headers(init && init.headers || {});
            if (!headers.has('content-type')) {
                headers.set('content-type', 'application/json');
            }
            return new Response(JSON.stringify(data), Object.assign({}, init, { headers: headers }));
        };
        globalThis.Response = Response;

        // ── FormData ──────────────────────────────────────────────────────────
        function FormData() {
            if (!(this instanceof FormData)) return new FormData();
            this._entries = [];
        }
        FormData.prototype.append = function(name, value, filename) {
            this._entries.push({ name: name, value: value, filename: filename });
        };
        FormData.prototype.delete = function(name) {
            this._entries = this._entries.filter(function(e) { return e.name !== name; });
        };
        FormData.prototype.get = function(name) {
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i].name === name) return this._entries[i].value;
            }
            return null;
        };
        FormData.prototype.getAll = function(name) {
            var result = [];
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i].name === name) result.push(this._entries[i].value);
            }
            return result;
        };
        FormData.prototype.has = function(name) {
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i].name === name) return true;
            }
            return false;
        };
        FormData.prototype.set = function(name, value, filename) {
            var found = false;
            this._entries = this._entries.map(function(e) {
                if (e.name === name) {
                    if (!found) { found = true; return { name: name, value: value, filename: filename }; }
                    return null;
                }
                return e;
            }).filter(function(e) { return e !== null; });
            if (!found) this._entries.push({ name: name, value: value, filename: filename });
        };
        FormData.prototype.forEach = function(fn, thisArg) {
            for (var i = 0; i < this._entries.length; i++) {
                fn.call(thisArg, this._entries[i].value, this._entries[i].name, this);
            }
        };
        FormData.prototype[Symbol.iterator] = function() {
            var entries = this._entries;
            var i = 0;
            return {
                next: function() {
                    if (i < entries.length) {
                        var value = [entries[i].name, entries[i].value];
                        i++;
                        return { value: value, done: false };
                    }
                    return { value: undefined, done: true };
                }
            };
        };
        globalThis.FormData = FormData;

        // ── ReadableStream / WritableStream / TransformStream ──────────────────
        // Simplified but functionally real WHATWG Streams: getReader()/
        // getWriter() actually dispatch to the underlying source/sink
        // (previously WritableStream.write()/close() never called
        // sink.write()/sink.close() at all — chunks just vanished into an
        // internal array — and pipeTo()/getWriter() didn't exist).
        function ReadableStream(init) {
            if (!(this instanceof ReadableStream)) return new ReadableStream(init);
            init = init || {};
            this._started = false;
            this._closed = false;
            this._errored = undefined;
            this._chunks = [];
            this._pending = [];
            this.locked = false;
            var self = this;
            this._controller = {
                enqueue: function(chunk) {
                    if (!self._closed) {
                        self._chunks.push(chunk);
                        self._flushPending();
                    }
                },
                close: function() {
                    self._closed = true;
                    self._flushPending();
                },
                error: function(e) {
                    self._errored = e;
                    self._closed = true;
                    while (self._pending.length > 0) self._pending.shift().reject(e);
                }
            };
            if (init.start) {
                init.start(this._controller);
                this._started = true;
            }
        }
        ReadableStream.prototype._flushPending = function() {
            while (this._pending.length > 0 && this._chunks.length > 0) {
                this._pending.shift().resolve({ done: false, value: this._chunks.shift() });
            }
            if (this._closed) {
                while (this._pending.length > 0) {
                    this._pending.shift().resolve({ done: true, value: undefined });
                }
            }
        };
        ReadableStream.prototype.getReader = function() {
            var stream = this;
            stream.locked = true;
            return {
                read: function() {
                    return new Promise(function(resolve, reject) {
                        if (stream._chunks.length > 0) {
                            resolve({ done: false, value: stream._chunks.shift() });
                        } else if (stream._closed) {
                            resolve({ done: true, value: undefined });
                        } else {
                            stream._pending.push({ resolve: resolve, reject: reject });
                        }
                    });
                },
                releaseLock: function() { stream.locked = false; },
                closed: Promise.resolve(),
            };
        };
        ReadableStream.prototype.pipeTo = function(dest) {
            var reader = this.getReader();
            var writer = typeof dest.getWriter === 'function' ? dest.getWriter() : dest;
            function pump() {
                return reader.read().then(function(res) {
                    if (res.done) return writer.close ? writer.close() : undefined;
                    return Promise.resolve(writer.write(res.value)).then(pump);
                });
            }
            return pump();
        };
        ReadableStream.prototype.cancel = function() { this._closed = true; return Promise.resolve(); };
        ReadableStream.prototype[Symbol.asyncIterator] = function() {
            var reader = this.getReader();
            return {
                next: function() {
                    return reader.read().then(function(res) {
                        return { value: res.value, done: res.done };
                    });
                },
                return: function() { reader.releaseLock(); return Promise.resolve({ done: true }); },
            };
        };

        function WritableStream(sink) {
            if (!(this instanceof WritableStream)) return new WritableStream(sink);
            this._sink = sink || {};
            this._closed = false;
            this.locked = false;
            var self = this;
            this._controller = { error: function(e) { self._errored = e; } };
            if (this._sink.start) { this._sink.start(this._controller); }
        }
        WritableStream.prototype._doWrite = function(chunk) {
            var self = this;
            return Promise.resolve().then(function() {
                return self._sink.write ? self._sink.write(chunk, self._controller) : undefined;
            });
        };
        WritableStream.prototype._doClose = function() {
            var self = this;
            this._closed = true;
            return Promise.resolve().then(function() {
                return self._sink.close ? self._sink.close() : undefined;
            });
        };
        WritableStream.prototype._doAbort = function(reason) {
            var self = this;
            return Promise.resolve().then(function() {
                return self._sink.abort ? self._sink.abort(reason) : undefined;
            });
        };
        WritableStream.prototype.write = function(chunk) { return this._doWrite(chunk); };
        WritableStream.prototype.close = function() { return this._doClose(); };
        WritableStream.prototype.abort = function(reason) { return this._doAbort(reason); };
        WritableStream.prototype.getWriter = function() {
            var stream = this;
            stream.locked = true;
            return {
                write: function(chunk) { return stream._doWrite(chunk); },
                close: function() { return stream._doClose(); },
                abort: function(reason) { return stream._doAbort(reason); },
                releaseLock: function() { stream.locked = false; },
                closed: Promise.resolve(),
                ready: Promise.resolve(),
            };
        };

        function TransformStream(init) {
            if (!(this instanceof TransformStream)) return new TransformStream(init);
            init = init || {};
            this.readable = new ReadableStream();
            var readableController = this.readable._controller;
            this.writable = new WritableStream({
                write: function(chunk) {
                    var controller = {
                        enqueue: function(c) { readableController.enqueue(c); },
                        error: function(e) { readableController.error(e); },
                    };
                    if (init.transform) return init.transform(chunk, controller);
                    controller.enqueue(chunk);
                },
                close: function() {
                    var controller = {
                        enqueue: function(c) { readableController.enqueue(c); },
                        error: function(e) { readableController.error(e); },
                    };
                    return Promise.resolve(init.flush ? init.flush(controller) : undefined)
                        .then(function() { readableController.close(); });
                },
            });
        }

        globalThis.ReadableStream = ReadableStream;
        globalThis.WritableStream = WritableStream;
        globalThis.TransformStream = TransformStream;

        // ── structuredClone ───────────────────────────────────────────────────
        globalThis.structuredClone = function(value, options) {
            if (value === undefined || value === null) {
                return value;
            }
            if (typeof value === 'function') {
                var err = new Error('function cannot be cloned');
                err.name = 'DataCloneError';
                throw err;
            }
            if (typeof value !== 'object') {
                return value;
            }
            if (value instanceof Date) {
                return new Date(value.getTime());
            }
            if (value instanceof RegExp) {
                return new RegExp(value.source, value.flags);
            }
            if (value instanceof ArrayBuffer) {
                return value.slice(0);
            }
            if (ArrayBuffer.isView(value)) {
                return new Uint8Array(value);
            }
            if (value instanceof Uint8Array) {
                return new Uint8Array(value);
            }
            if (value instanceof Map) {
                var m = new Map();
                value.forEach(function(v, k) {
                    m.set(structuredClone(k), structuredClone(v));
                });
                return m;
            }
            if (value instanceof Set) {
                var s = new Set();
                value.forEach(function(v) {
                    s.add(structuredClone(v));
                });
                return s;
            }
            if (typeof value === 'object') {
                var copy = Array.isArray(value) ? [] : {};
                for (var key in value) {
                    if (value.hasOwnProperty(key)) {
                        copy[key] = structuredClone(value[key]);
                    }
                }
                return copy;
            }
            return value;
        };

        // ── FileReader ─────────────────────────────────────────────────────────
        function FileReader() {
            if (!(this instanceof FileReader)) return new FileReader();
            this.readyState = 0;
            this.result = null;
            this.error = null;
            this.onload = null;
            this.onerror = null;
            this.onloadend = null;
        }
        FileReader.prototype.readAsText = function(blob, encoding) {
            var reader = this;
            blob.arrayBuffer().then(function(buffer) {
                var bytes = new Uint8Array(buffer);
                var result = '';
                for (var i = 0; i < bytes.length; i++) {
                    result += String.fromCharCode(bytes[i]);
                }
                reader.result = result;
                reader.readyState = 2;
                if (reader.onload) reader.onload({ target: reader });
                if (reader.onloadend) reader.onloadend({ target: reader });
            }).catch(function(e) {
                reader.error = e;
                reader.readyState = 2;
                if (reader.onerror) reader.onerror({ target: reader });
                if (reader.onloadend) reader.onloadend({ target: reader });
            });
        };
        FileReader.prototype.readAsDataURL = function(blob) {
            var reader = this;
            blob.arrayBuffer().then(function(buffer) {
                var bytes = new Uint8Array(buffer);
                var binStr = '';
                for (var i = 0; i < bytes.length; i++) {
                    binStr += String.fromCharCode(bytes[i]);
                }
                reader.result = 'data:' + (blob.type || 'application/octet-stream') + ';base64,' + btoa(binStr);
                reader.readyState = 2;
                if (reader.onload) reader.onload({ target: reader });
                if (reader.onloadend) reader.onloadend({ target: reader });
            }).catch(function(e) {
                reader.error = e;
                reader.readyState = 2;
                if (reader.onerror) reader.onerror({ target: reader });
                if (reader.onloadend) reader.onloadend({ target: reader });
            });
        };
        FileReader.prototype.readAsArrayBuffer = function(blob) {
            var reader = this;
            blob.arrayBuffer().then(function(buffer) {
                reader.result = buffer;
                reader.readyState = 2;
                if (reader.onload) reader.onload({ target: reader });
                if (reader.onloadend) reader.onloadend({ target: reader });
            }).catch(function(e) {
                reader.error = e;
                reader.readyState = 2;
                if (reader.onerror) reader.onerror({ target: reader });
                if (reader.onloadend) reader.onloadend({ target: reader });
            });
        };
        FileReader.prototype.abort = function() {
            this.readyState = 2;
            this.result = null;
            if (this.onabort) this.onabort({ target: this });
            if (this.onloadend) this.onloadend({ target: this });
        };
        FileReader.EMPTY = 0;
        FileReader.LOADING = 1;
        FileReader.DONE = 2;
        globalThis.FileReader = FileReader;

        // ── Storage (localStorage / sessionStorage) ────────────────────────
        // ponytail: in-memory only, not persisted across process restarts —
        // matches the in-process-only guarantee Node/browsers give test code
        // anyway within a single run; add disk persistence if a script needs
        // storage to outlive the process.
        function Storage() {
            Object.defineProperty(this, '_data', { value: new Map(), enumerable: false });
        }
        Storage.prototype.getItem = function(key) {
            key = String(key);
            return this._data.has(key) ? this._data.get(key) : null;
        };
        Storage.prototype.setItem = function(key, value) {
            this._data.set(String(key), String(value));
        };
        Storage.prototype.removeItem = function(key) {
            this._data.delete(String(key));
        };
        Storage.prototype.clear = function() {
            this._data.clear();
        };
        Storage.prototype.key = function(index) {
            var keys = Array.from(this._data.keys());
            return index >= 0 && index < keys.length ? keys[index] : null;
        };
        Object.defineProperty(Storage.prototype, 'length', {
            get: function() { return this._data.size; }
        });
        globalThis.Storage = Storage;
        globalThis.localStorage = new Storage();
        globalThis.sessionStorage = new Storage();

        // ── URLPattern ───────────────────────────────────────────────────
        function _urlPatternCompile(pattern, sep) {
            var names = [];
            var parts = String(pattern).split(sep).map(function(seg) {
                if (seg === '*') { return '.*'; }
                if (seg.charAt(0) === ':') { names.push(seg.slice(1)); return '([^' + (sep === '/' ? '/' : '.') + ']+)'; }
                return seg.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
            });
            var joinChar = sep === '.' ? '\\.' : sep;
            return { regex: new RegExp('^' + parts.join(joinChar) + '$'), names: names };
        }
        function _urlPatternParts(input) {
            if (typeof input === 'string') {
                try {
                    var u = new URL(input);
                    return { pathname: u.pathname, hostname: u.hostname };
                } catch (e) {
                    return { pathname: input, hostname: '' };
                }
            }
            if (input && typeof input === 'object') {
                return { pathname: input.pathname || '', hostname: input.hostname || '' };
            }
            return { pathname: '', hostname: '' };
        }
        function URLPattern(init, baseURL) {
            var pathnamePattern, hostnamePattern;
            if (typeof init === 'string') {
                if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(init)) {
                    var u = new URL(init);
                    hostnamePattern = u.hostname;
                    pathnamePattern = u.pathname;
                } else {
                    pathnamePattern = init;
                }
            } else if (init && typeof init === 'object') {
                pathnamePattern = init.pathname;
                hostnamePattern = init.hostname;
            }
            this.pathname = pathnamePattern || '*';
            this.hostname = hostnamePattern || '*';
            this._pathnameCompiled = _urlPatternCompile(this.pathname, '/');
            this._hostnameCompiled = _urlPatternCompile(this.hostname, '.');
        }
        URLPattern.prototype.exec = function(input, baseURL) {
            var parts = _urlPatternParts(input);
            var pathMatch = this._pathnameCompiled.regex.exec(parts.pathname);
            if (!pathMatch) return null;
            var hostMatch = this._hostnameCompiled.regex.exec(parts.hostname);
            if (!hostMatch) return null;
            var groups = {};
            this._pathnameCompiled.names.forEach(function(name, i) {
                if (name) groups[name] = pathMatch[i + 1];
            });
            this._hostnameCompiled.names.forEach(function(name, i) {
                if (name) groups[name] = hostMatch[i + 1];
            });
            return {
                pathname: { input: parts.pathname, groups: groups },
                hostname: { input: parts.hostname, groups: groups },
                groups: groups,
            };
        };
        URLPattern.prototype.test = function(input, baseURL) {
            return this.exec(input, baseURL) !== null;
        };
        globalThis.URLPattern = URLPattern;

        // ── EventSource ──────────────────────────────────────────────────
        function EventSource(url) {
            this.url = url;
            this.readyState = EventSource.CONNECTING;
            this.withCredentials = false;
            this._listeners = {};
            this.onopen = null;
            this.onmessage = null;
            this.onerror = null;
            var self = this;
            this._id = __eventSourceOpen(url);
            this.readyState = EventSource.OPEN;
            this._timer = setInterval(function() {
                if (self.readyState === EventSource.CLOSED) return;
                var raw = __eventSourcePoll(self._id);
                if (!raw) return;
                var events = JSON.parse(raw);
                events.forEach(function(ev) {
                    var evt = { type: ev.type, data: ev.data, lastEventId: ev.lastEventId, target: self };
                    if (ev.type === 'error') {
                        if (self.onerror) self.onerror(evt);
                    } else if (ev.type === 'message' || ev.type === '') {
                        if (self.onmessage) self.onmessage(evt);
                    }
                    (self._listeners[ev.type] || []).forEach(function(fn) { fn(evt); });
                });
            }, 20);
        }
        EventSource.CONNECTING = 0;
        EventSource.OPEN = 1;
        EventSource.CLOSED = 2;
        EventSource.prototype.addEventListener = function(type, fn) {
            (this._listeners[type] = this._listeners[type] || []).push(fn);
        };
        EventSource.prototype.removeEventListener = function(type, fn) {
            var list = this._listeners[type];
            if (!list) return;
            var idx = list.indexOf(fn);
            if (idx !== -1) list.splice(idx, 1);
        };
        EventSource.prototype.close = function() {
            this.readyState = EventSource.CLOSED;
            clearInterval(this._timer);
            __eventSourceClose(this._id);
        };
        globalThis.EventSource = EventSource;

    })();
    "#;

    let source = v8::String::new(scope, web_globals_code).unwrap();
    let script =
        Script::compile(scope, source, None).ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    Ok(())
}
