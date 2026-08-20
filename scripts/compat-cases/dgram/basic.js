// Round-trip over loopback: bind, send with offset/length, recv with rinfo.
// Ephemeral ports are printed as P so node and 3va can be compared verbatim.
var dgram = require('dgram');
var out = [];
var s = dgram.createSocket('udp4');
var r = dgram.createSocket('udp4');
r.on('message', function(msg, rinfo) {
    out.push('recv=' + msg.toString() + ' rinfo.family=' + rinfo.family +
        ' rinfo.size=' + rinfo.size + ' rinfo.port>0=' + (rinfo.port > 0));
    finish();
});
r.bind(0, '127.0.0.1', function() {
    var rp = r.address().port;
    s.bind(0, '127.0.0.1', function() {
        var addr = s.address();
        out.push('addr.address=' + addr.address + ' addr.family=' + addr.family +
            ' addr.port>0=' + (addr.port > 0));
        s.send('hello-udp', 0, 9, rp, '127.0.0.1', function(err, bytes) {
            out.push('send err=' + (err ? err.code : 'null') + ' bytes=' + bytes);
        });
    });
});
function finish() {
    s.close();
    r.close();
    setTimeout(function() { console.log(out.join('\n')); }, 0);
}