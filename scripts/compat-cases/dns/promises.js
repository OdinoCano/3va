// dns.promises API: resolveMx / resolve4 / lookup / lookupService / getServers.
var dns = require('dns');
var p = dns.promises;
var out = [];
var pending = 0;
function done() { if (--pending <= 0) console.log(out.join('\n')); }
function wrap(label, prom) {
    pending++;
    prom.then(function(res) { out.push(label + ' ' + JSON.stringify(sort(res))); done(); },
              function(e) { out.push(label + ' ERR ' + e.code); done(); });
}
function sort(arr) {
    if (arr && arr.length && typeof arr[0] === 'object' && !Array.isArray(arr[0])) {
        return arr.slice().sort(function(a, b) { return JSON.stringify(a) < JSON.stringify(b) ? -1 : 1; });
    }
    if (Array.isArray(arr)) return arr.slice().sort();
    return arr;
}
wrap('pMx', p.resolveMx('gmail.com'));
wrap('pA', p.resolve4('example.com'));
wrap('pLookup', p.lookup('localhost'));
wrap('pLookupService', p.lookupService('8.8.8.8', 443).then(function(r) { return r.hostname + ' ' + r.service; }));
wrap('pReverse', p.reverse('8.8.8.8'));
wrap('pServers', Promise.resolve(dns.promises.getServers()));
wrap('pAny', p.resolveAny('google.com').then(function(recs) {
    return (recs || []).map(function(r) { return r.type; }).sort();
}));