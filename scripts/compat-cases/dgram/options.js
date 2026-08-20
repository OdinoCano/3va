// Return-value contracts for socket option setters/getters. Buffer sizes are
// kernel-dependent, so they are printed as P (only the fact that a number is
// returned is compared).
var dgram = require('dgram');
var s = dgram.createSocket('udp4');
s.bind(0, '127.0.0.1', function() {
    var out = [];
    out.push('setTTL-ret=' + s.setTTL(64));
    out.push('setBroadcast-ret=' + s.setBroadcast(true));
    out.push('setMulticastTTL-ret=' + s.setMulticastTTL(8));
    out.push('setMulticastLoopback-ret=' + s.setMulticastLoopback(true));
    out.push('setRecvBufferSize-ret=' + s.setRecvBufferSize(32768));
    out.push('addMembership-ret=' + s.addMembership('224.0.0.1'));
    out.push('getRecvBufferSize=' + (typeof s.getRecvBufferSize() === 'number' ? 'P' : 'NaN'));
    out.push('getSendBufferSize=' + (typeof s.getSendBufferSize() === 'number' ? 'P' : 'NaN'));
    out.push('ref-ret=this ' + (s.ref() === s));
    out.push('unref-ret=this ' + (s.unref() === s));
    out.push('setMulticastInterface-ret=' + s.setMulticastInterface('127.0.0.1'));
    out.push('dropMembership-ret=' + s.dropMembership('224.0.0.1'));
    console.log(out.join('\n'));
    s.close();
});