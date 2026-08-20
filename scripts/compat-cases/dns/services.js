// lookupService, reverse, getServers, constants, result-order defaults.
var dns = require('dns');
var out = [];
var pending = 0;
var printed = false;
function done() { if (printed) return; if (--pending <= 0) { printed = true; console.log(out.join('\n')); } }
function rec(label, cb) {
    pending++;
    cb(function(err, res) {
        out.push(label + ' ' + (err ? ('ERR ' + err.code) : res));
        done();
    });
}
rec('lookupService-88', function(cb) {
    dns.lookupService('8.8.8.8', 443, function(e, h, s) { cb(e, e ? null : h + ' ' + s); });
});
rec('lookupService-lo', function(cb) {
    dns.lookupService('127.0.0.1', 80, function(e, h, s) { cb(e, e ? null : h + ' ' + s); });
});
rec('reverse-88', function(cb) { dns.reverse('8.8.8.8', function(e, r) { cb(e, e ? null : r.sort()); }); });

out.push('getServers ' + JSON.stringify(dns.getServers()));
out.push('getDefaultResultOrder ' + dns.getDefaultResultOrder());
dns.setDefaultResultOrder('ipv4first');
out.push('getDefaultResultOrder2 ' + dns.getDefaultResultOrder());
dns.setDefaultResultOrder('verbatim');
out.push('consts ' + dns.ADDRCONFIG + ',' + dns.ALL + ',' + dns.V4MAPPED);
out.push('consts-codes ' + dns.NODATA + ',' + dns.NOTFOUND + ',' + dns.FORMERR + ',' + dns.SERVFAIL + ',' + dns.NOTIMP + ',' + dns.REFUSED);