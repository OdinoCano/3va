// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const crypto = require('crypto');
const port = Number(process.argv[2]);
const out = [];

const sha256 = (b) => crypto.createHash('sha256').update(Buffer.from(b)).digest('hex');

function send(opts, body) {
  return new Promise((resolve, reject) => {
    const r = http.request(Object.assign({ host: '127.0.0.1', port }, opts), (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(cs) }));
    });
    r.on('error', reject);
    r.end(body);
  });
}

const rec = (label, r) => out.push(label + '\t' + r.status + '\t' + r.body.toString('utf8'));
const recHash = (label, r) => out.push(label + '\t' + r.status + '\tsha256=' + sha256(r.body));

(async () => {
  rec('json', await send({ method: 'POST', path: '/echo', headers: { 'content-type': 'application/json' } }, '{"a":1,"b":"x"}'));
  rec('binary', await send({ method: 'POST', path: '/raw', headers: { 'content-type': 'application/octet-stream' } }, Buffer.from([0, 255, 104, 105, 200])));
  recHash('binary-response', await send({ method: 'GET', path: '/binary' }));
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
