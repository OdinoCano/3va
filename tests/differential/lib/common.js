// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// Shared helpers for the differential scenarios. Nothing here may introduce
// non-determinism into the printed output (no timestamps, no ports, no PIDs).

'use strict';

const http = require('http');
const crypto = require('crypto');

/** SHA-256 of a string or Buffer, lowercase hex. */
function sha256(input) {
  return crypto.createHash('sha256').update(input).digest('hex');
}

/**
 * Send one HTTP request with the raw `http` module and resolve
 * `{ status, headers, body }`, where `body` is a Buffer.
 */
function request(port, opts) {
  const options = Object.assign(
    { host: '127.0.0.1', port, method: 'GET', path: '/' },
    opts
  );
  return new Promise((resolve, reject) => {
    const req = http.request(options, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(Buffer.from(c)));
      res.on('end', () =>
        resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) })
      );
    });
    req.on('error', reject);
    if (options.body !== undefined && options.body !== null) req.write(options.body);
    req.end();
  });
}

/** Print one deterministic line: `label<TAB>status<TAB>body(utf8 lossy)`. */
function line(label, res) {
  return label + '\t' + res.status + '\t' + res.body.toString('utf8');
}

/** Print a deterministic line with the body replaced by its SHA-256. */
function lineHash(label, res) {
  return label + '\t' + res.status + '\tsha256=' + sha256(res.body);
}

function listen(app, port, onReady) {
  const srv = app.listen(port, '127.0.0.1', () => process.stdout.write('READY\n'));
  return srv;
}

module.exports = { sha256, request, line, lineHash, listen };
