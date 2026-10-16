// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// A minimal, deterministic HTTP server used by the client-side scenarios. It
// echoes the request in a canonical JSON form, and has a few fixed binary /
// chunked / empty responses so the client framing can be compared.
'use strict';

const http = require('http');
const { sha256 } = require('./common.js');

const BINARY = Buffer.alloc(256);
for (let i = 0; i < 256; i++) BINARY[i] = i;

function start(port) {
  const srv = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(Buffer.from(c)));
    req.on('end', () => {
      if (req.url === '/binary') {
        res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': String(BINARY.length) });
        return res.end(BINARY);
      }
      if (req.url === '/chunked') {
        res.writeHead(200, { 'content-type': 'text/plain' });
        res.write('ab');
        setTimeout(() => {
          res.write('cd');
          res.end('ef');
        }, 10);
        return;
      }
      if (req.url === '/nocontent') {
        res.writeHead(204);
        return res.end();
      }
      const body = Buffer.concat(chunks);
      const payload = JSON.stringify({
        method: req.method,
        path: req.url,
        contentType: req.headers['content-type'] || null,
        contentLength: req.headers['content-length'] || null,
        transferEncoding: req.headers['transfer-encoding'] || null,
        bodyHex: body.toString('hex'),
        bodySha: sha256(body),
      });
      res.writeHead(200, { 'content-type': 'application/json', 'content-length': String(Buffer.byteLength(payload)) });
      res.end(payload);
    });
  });
  srv.listen(port, '127.0.0.1', () => process.stdout.write('READY\n'));
  return srv;
}

module.exports = { start, BINARY };
