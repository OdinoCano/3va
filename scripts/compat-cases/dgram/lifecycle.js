// Socket lifecycle and state transitions. Uncaught "expected" throws are
// caught and their error codes compared (stack traces are never compared).
var dgram = require('dgram');
var out = [];
var s = dgram.createSocket('udp4');

function tryCode(fn) {
    try { fn(); return 'no-throw'; } catch (e) { return e.code; }
}

// Node's own key order for address()/remoteAddress() objects is not stable
// across calls, so serialize with sorted keys for a byte-stable comparison.
function sjson(obj) {
    return JSON.stringify(obj, Object.keys(obj).sort());
}

out.push('pre-bind address=' + tryCode(function() { s.address(); }));
out.push('pre-bind remoteAddress=' + tryCode(function() { s.remoteAddress(); }));
out.push('bind-ret=this ' + (s.bind(0, '127.0.0.1', function() {
    out.push('bound address=' + sjson(s.address()).replace(/"port":\d+/, '"port":P'));
    out.push('double-bind=' + tryCode(function() { s.bind(0, '127.0.0.1'); }));
    out.push('close-ret=this ' + (s.close() === s));
    out.push('post-close address=' + tryCode(function() { s.address(); }));
    out.push('post-close send=' + tryCode(function() { s.send('x', 9, '127.0.0.1'); }));

    var c = dgram.createSocket('udp4');
    c.bind(0, '127.0.0.1', function() {
        var ret = c.connect(1, '127.0.0.1', function() {
            out.push('connect-ret=this ' + (ret === c));
            out.push('connected remoteAddress=' + sjson(c.remoteAddress()).replace(/"port":\d+/, '"port":P'));
            c.disconnect();
            out.push('after-disconnect remoteAddress=' + tryCode(function() { c.remoteAddress(); }));
            c.close();
            console.log(out.join('\n'));
        });
    });
}) === s));