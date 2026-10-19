// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

// The request must go through the agent's `createConnection()`.
//
// The proxy stands in for the proxy of a corporate network: the origin the
// request names does not exist, so only the proxy can answer it. A client
// that ignores the factory (3va did, until 2.13.0) would try to reach
// `origin.invalid` directly and fail.

const http = require('http');
const net = require('net');

const proxyPort = Number(process.argv[2]);
const out = [];

class ProxyAgent extends http.Agent {
  createConnection(options, cb) {
    const socket = net.connect(proxyPort, '127.0.0.1', () => {
      socket.write(
        'CONNECT ' + options.host + ':' + options.port + ' HTTP/1.1\r\n' +
        'Host: ' + options.host + ':' + options.port + '\r\n\r\n',
      );
    });
    // The socket is only handed over once the proxy has established the
    // tunnel, as the real proxy agents do.
    let head = '';
    const onData = (chunk) => {
      head += chunk.toString('latin1');
      const idx = head.indexOf('\r\n\r\n');
      if (idx < 0) return;
      socket.removeListener('data', onData);
      const status = (head.split('\r\n')[0] || '').split(' ')[1];
      if (status !== '200') {
        socket.destroy();
        cb(new Error('the proxy refused the tunnel: ' + status));
        return;
      }
      cb(null, socket);
    };
    socket.on('data', onData);
    socket.on('error', (err) => cb(err));
  }
}

const req = http.request({
  agent: new ProxyAgent(),
  host: 'origin.invalid',
  port: 443,
  path: '/through-the-proxy',
}, (res) => {
  const chunks = [];
  res.on('data', (c) => chunks.push(c));
  res.on('end', () => {
    out.push('status\t' + res.statusCode);
    out.push('body\t' + Buffer.concat(chunks).toString('utf8').trim());
    process.stdout.write(out.join('\n') + '\n');
  });
});
req.on('error', (err) => {
  process.stdout.write('fatal\t' + (err && err.message) + '\n');
  process.exit(1);
});
req.end();
