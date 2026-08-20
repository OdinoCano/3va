// resolveMx/resolveTxt/resolveNs/resolveCname record shapes. Record ORDER is
// not a compatibility contract (DNS servers shuffle answers), so arrays are
// sorted before printing.
var dns = require('dns');
var out = [];
var pending = 0;
function done() { if (--pending <= 0) console.log(out.join('\n')); }
function rec(label, cb) {
    pending++;
    cb(function(err, res) {
        out.push(label + ' ' + (err ? ('ERR ' + err.code + ' ' + (err.syscall || '')) : JSON.stringify(sort(res))));
        done();
    });
}
function sort(arr) {
    if (arr && arr.length && typeof arr[0] === 'object') {
        return arr.slice().sort(function(a, b) { return JSON.stringify(a) < JSON.stringify(b) ? -1 : 1; });
    }
    return (arr || []).slice().sort();
}
rec('resolveMx', function(cb) { dns.resolveMx('gmail.com', cb); });
rec('resolveTxt', function(cb) { dns.resolveTxt('google.com', cb); });
rec('resolveNs', function(cb) { dns.resolveNs('google.com', cb); });
rec('resolveCname', function(cb) { dns.resolveCname('www.github.com', cb); });