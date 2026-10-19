// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

// The proxy of the scenario: a minimal HTTP proxy that answers a `CONNECT`
// and then tunnels the bytes to the target. It stands in for the proxy of a
// corporate network: the app's outgoing request has to go through it, and
// the proxy records that it did.
//
// The port comes from the environment so it never appears in the output.

const http = require('http');
const net = require('net');

const port = Number(process.env.PROXY_PORT || 1235);
let connects = 0;

const server = net.createServer((client) => {
  let buf = Buffer.alloc(0);
  let tunnelled = false;
  let target = null;

  client.on('data', (chunk) => {
    if (tunnelled) {
      if (target) target.write(chunk);
      return;
    }
    buf = Buffer.concat([buf, chunk]);
    const idx = buf.indexOf('\r\n\r\n');
    if (idx < 0) return;
    const line = (buf.slice(0, idx).toString('latin1').split('\r\n')[0] || '');
    const m = /^CONNECT\s+(\S+):(\d+)\s+HTTP\/1\.[01]$/.exec(line);
    if (!m) {
      client.end('HTTP/1.1 400 Bad Request\r\n\r\n');
      return;
    }
    connects++;
    tunnelled = true;
    buf = Buffer.alloc(0);
    target = net.connect(Number(m[2]), m[1], () => {
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      client.pipe(target);
      target.pipe(client);
    });
    target.on('error', () => client.destroy());
  });

  client.on('error', () => {});
});

server.listen(port, '0.0.0.0', () => console.log('PROXY READY'));

// The count of tunnels, so the app (and the client) can check the request
// really went through the proxy.
http
  .createServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ connects }));
  })
  .listen(port + 1, '0.0.0.0', () => console.log('PROXY STATS READY'));
