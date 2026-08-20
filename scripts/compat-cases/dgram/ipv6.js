// IPv6 (udp6) round-trip: family reported as IPv6, ::1 loopback, rinfo.size.
var dgram = require('dgram');
var out = [];
var s = dgram.createSocket('udp6');
var r = dgram.createSocket('udp6');
r.on('message', function(msg, rinfo) {
    out.push('recv=' + msg.toString() + ' rinfo.family=' + rinfo.family +
        ' rinfo.size=' + rinfo.size + ' rinfo.address=' + rinfo.address);
    finish();
});
r.bind(0, '::1', function() {
    var rp = r.address().port;
    s.bind(0, '::1', function() {
        var addr = s.address();
        out.push('addr.address=' + addr.address + ' addr.family=' + addr.family +
            ' addr.port>0=' + (addr.port > 0));
        s.send('ipv6-ping', 0, 9, rp, '::1', function(err, bytes) {
            out.push('send err=' + (err ? err.code : 'null') + ' bytes=' + bytes);
        });
    });
});
function finish() {
    s.close();
    r.close();
    setTimeout(function() { console.log(out.join('\n')); }, 0);
}