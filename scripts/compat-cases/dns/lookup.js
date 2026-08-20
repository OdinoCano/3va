// lookup() with family hints, resolve4/resolve6. Loopback names are resolved
// through glibc getaddrinfo by both runtimes, so results are deterministic per
// machine and comparable.
var dns = require('dns');
var out = [];
var pending = 0;
function done() { if (--pending <= 0) console.log(out.join('\n')); }
function rec(label, cb) {
    pending++;
    cb(function(err, res) {
        out.push(label + ' ' + (err ? ('ERR ' + err.code) : res));
        done();
    });
}
rec('lookup-localhost', function(cb) { dns.lookup('localhost', function(e, a, f) { cb(e, e ? null : a + '/' + f); }); });
rec('lookup-fam6', function(cb) { dns.lookup('localhost', { family: 6 }, function(e, a, f) { cb(e, e ? null : a + '/' + f); }); });
rec('lookup-all', function(cb) { dns.lookup('localhost', { all: true }, cb); });
rec('lookup-example4', function(cb) { dns.lookup('example.com', { all: true, family: 4 }, function(e, r) { cb(e, e ? null : r.map(function(x) { return x.address; }).sort()); }); });
rec('resolve4-example', function(cb) { dns.resolve4('example.com', function(e, r) { cb(e, e ? null : r.sort()); }); });
rec('resolve6-localhost', function(cb) { dns.resolve6('localhost', function(e, r) { cb(e, e ? null : r.sort()); }); });
rec('resolve6-example', function(cb) { dns.resolve6('example.com', function(e, r) { cb(e, e ? null : r.sort()); }); });