// resolveAny shape: every record must carry a `type`, TXT records an `entries`
// array. The exact record SET for an ANY query is resolver-dependent, so only
// the sorted set of record types + shape invariants are compared.
var dns = require('dns');
dns.resolveAny('google.com', function(err, records) {
    if (err) { console.log('resolveAny ERR ' + err.code); return; }
    var types = {};
    var shape = true;
    (records || []).forEach(function(r) {
        types[r.type] = (types[r.type] || 0) + 1;
        if (!r.type) shape = false;
        if (r.type === 'TXT' && !Array.isArray(r.entries)) shape = false;
        if ((r.type === 'A' || r.type === 'AAAA') && !r.address) shape = false;
        if (r.type === 'NS' && !r.value) shape = false;
        if (r.type === 'MX' && !r.exchange) shape = false;
        if (r.type === 'SOA' && !r.nsname) shape = false;
        if (r.type === 'SRV' && !r.name) shape = false;
        if (r.type === 'CNAME' && !r.value) shape = false;
        if (r.type === 'PTR' && !r.value) shape = false;
    });
    console.log('types=' + Object.keys(types).sort().join(','));
    console.log('shape=' + shape);
});