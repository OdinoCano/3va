// Error mapping: EADDRINUSE on double bind, ENOTFOUND on unresolvable host,
// connected send without destination.
var dgram = require('dgram');
var out = [];
var advanced = false;
function advance() { if (advanced) return; advanced = true; next(); }

var s = dgram.createSocket('udp4');
s.bind(0, '127.0.0.1', function() {
    var port = s.address().port;
    var s2 = dgram.createSocket('udp4');
    var flagged = false;
    function once(code) {
        if (flagged) return;
        flagged = true;
        out.push('eaddr-in-use=' + code);
        advance();
    }
    s2.on('error', function(e) { once(e.code); });
    s2.bind(port, '127.0.0.1', function(e) { once(e ? e.code : 'no-error'); });
    setTimeout(function() { once('timeout'); }, 500);
});

function next() {
    s.send('x', 9, 'nonexistent-3va-test.invalid', function(err) {
        out.push('badhost=' + (err && err.code) + ' syscall=' + (err && err.syscall));
        var c = dgram.createSocket('udp4');
        c.bind(0, '127.0.0.1', function() {
            var r = dgram.createSocket('udp4');
            r.on('message', function(m) { out.push('connected-recv=' + m.toString()); finish(); });
            r.bind(0, '127.0.0.1', function() {
                var rp = r.address().port;
                c.connect(rp, '127.0.0.1', function(err) {
                    out.push('connect-err=' + (err ? err.code : 'null'));
                    c.send('conn', function(err, bytes) {
                        out.push('connected-send err=' + (err ? err.code : 'null') + ' bytes=' + bytes);
                    });
                });
            });
        });
    });
}
function finish() {
    s.close();
    console.log(out.join('\n'));
    process.exit(0);
}