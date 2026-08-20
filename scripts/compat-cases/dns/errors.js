// Error semantics: per-record-type syscall names, ENOTFOUND on bad hosts,
// synchronous ERR_INVALID_ARG_VALUE for unknown rrtype. SRV may legitimately
// time out (ETIMEOUT) instead of ENOTFOUND depending on the live resolver, so
// ETIMEOUT is normalized to ENOTFOUND for the SRV case only.
var dns = require('dns');
var out = [];
var pending = 0;
function done() { if (--pending <= 0) console.log(out.join('\n')); }
function errCase(label, rrtype) {
    pending++;
    dns['resolve' + rrtype]('nonexistent-3va-test.invalid', function(err) {
        var code = err.code;
        if (rrtype === 'Srv' && code === 'ETIMEOUT') code = 'ENOTFOUND';
        out.push(label + ' code=' + code + ' syscall=' + err.syscall + ' hostname=' + err.hostname);
        done();
    });
}
errCase('MX', 'Mx');
errCase('TXT', 'Txt');
errCase('NS', 'Ns');
errCase('CNAME', 'Cname');
errCase('SRV', 'Srv');
errCase('NAPTR', 'Naptr');
errCase('SOA', 'Soa');
errCase('PTR', 'Ptr');
errCase('A4', '4');
errCase('A6', '6');

try {
    dns.resolve('example.com', 'BOGUS', function() {});
    out.push('bogus=no-throw');
} catch (e) {
    out.push('bogus=' + e.code + ' msg=' + e.message);
}