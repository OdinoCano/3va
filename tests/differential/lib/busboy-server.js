// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// A raw-http multipart server that parses with busboy directly (no express).
'use strict';

const http = require('http');
const busboy = require('busboy');
const { sha256 } = require('./common.js');

function start(port) {
  const srv = http.createServer((req, res) => {
    const bb = busboy({ headers: req.headers });
    const files = [];
    const fields = {};
    bb.on('file', (name, stream, info) => {
      const cs = [];
      stream.on('data', (c) => cs.push(Buffer.from(c)));
      stream.on('end', () => {
        const buf = Buffer.concat(cs);
        files.push({ name, filename: info.filename, mimeType: info.mimeType, size: buf.length, sha: sha256(buf) });
      });
    });
    bb.on('field', (name, val) => { fields[name] = val; });
    bb.on('close', () => {
      const payload = JSON.stringify({ files, fields });
      res.writeHead(200, { 'content-type': 'application/json', 'content-length': String(Buffer.byteLength(payload)) });
      res.end(payload);
    });
    req.pipe(bb);
  });
  srv.listen(port, '127.0.0.1', () => process.stdout.write('READY\n'));
  return srv;
}

module.exports = { start };
