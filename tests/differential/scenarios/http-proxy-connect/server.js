// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

// A minimal HTTP proxy that answers a `CONNECT` and then serves the tunnelled
// request itself. It stands in for the proxy of a corporate network: the
// request must reach it through the agent's `createConnection()`, and the
// origin it names (`origin.invalid`) does not exist, so a client that
// bypassed the proxy could not get an answer at all.
//
// The port comes from the command line so it never appears in the output.

const net = require('net');

const port = Number(process.argv[2]);
let connects = 0;

const server = net.createServer((client) => {
  let buf = Buffer.alloc(0);
  let tunnelled = false;
  let target = null;

  client.on('data', (chunk) => {
    if (tunnelled) {
      // The tunnelled request: read its head and answer it.
      if (target === null) {
        buf = Buffer.concat([buf, chunk]);
        if (buf.indexOf('\r\n\r\n') < 0) return;
        const body = 'served-by-the-proxy\n';
        client.write(
          'HTTP/1.1 200 OK\r\n' +
          'Content-Type: text/plain\r\n' +
          'Content-Length: ' + Buffer.byteLength(body) + '\r\n' +
          '\r\n' + body,
        );
        target = 'done';
      }
      return;
    }

    buf = Buffer.concat([buf, chunk]);
    const idx = buf.indexOf('\r\n\r\n');
    if (idx < 0) return;
    const head = buf.slice(0, idx).toString('latin1');
    const line = head.split('\r\n')[0] || '';
    const m = /^CONNECT\s+(\S+)\s+HTTP\/1\.[01]$/.exec(line);
    if (!m) {
      client.end('HTTP/1.1 400 Bad Request\r\n\r\n');
      return;
    }
    connects++;
    tunnelled = true;
    buf = Buffer.alloc(0);
    client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
  });

  client.on('error', () => {});
});

server.listen(port, '127.0.0.1', () => {
  process.stdout.write('READY\n');
});

process.on('exit', () => {
  process.stderr.write('connects=' + connects + '\n');
});
