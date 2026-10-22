// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// A net server that reads to EOF (the client's half-close) and only then
// answers, then ends. This is the shape that 3va got wrong in 2.12.5.
'use strict';

const net = require('net');

function serve(port) {
  const srv = net.createServer({ allowHalfOpen: true }, (sock) => {
    const cs = [];
    sock.on('data', (c) => cs.push(Buffer.from(c)));
    sock.on('end', () => {
      const body = Buffer.concat(cs);
      sock.end(Buffer.concat([Buffer.from('echo:'), body, Buffer.from([0, 255])]));
    });
    sock.on('error', () => {});
  });
  srv.listen(port, '127.0.0.1', () => process.stdout.write('READY\n'));
  return srv;
}

module.exports = { serve };
