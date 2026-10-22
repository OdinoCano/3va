// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const crypto = require('crypto');
const port = Number(process.argv[2]);
const out = [];

const sha256 = (b) => crypto.createHash('sha256').update(b).digest('hex');

function send(opts, fn) {
  return new Promise((resolve, reject) => {
    const r = http.request(Object.assign({ host: '127.0.0.1', port }, opts), (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(cs) }));
    });
    r.on('error', reject);
    fn(r);
  });
}

const rec = (label, r) => out.push(label + '\t' + r.status + '\t' + r.body.toString('utf8'));
const recHash = (label, r) => out.push(label + '\t' + r.status + '\tsha256=' + sha256(r.body));

(async () => {
  rec('string-end', await send({ method: 'POST', path: '/echo' }, (r) => r.end('hi')));
  rec('buffer-end', await send({ method: 'POST', path: '/echo' }, (r) => r.end(Buffer.from([0, 255, 104, 105, 200]))));
  rec('uint8array-end', await send({ method: 'POST', path: '/echo' }, (r) => r.end(new Uint8Array([0, 255, 104, 105, 200]))));
  rec('explicit-length', await send({ method: 'POST', path: '/echo', headers: { 'content-length': '5' } }, (r) => { r.write('hello'); r.end(); }));
  rec('text-body', await send({ method: 'POST', path: '/echo', headers: { 'content-type': 'text/plain' } }, (r) => r.end('hello')));

  recHash('binary-response', await send({ method: 'GET', path: '/binary' }, (r) => r.end()));
  rec('chunked-response', await send({ method: 'GET', path: '/chunked' }, (r) => r.end()));
  rec('no-content', await send({ method: 'GET', path: '/nocontent' }, (r) => r.end()));

  const agent = new http.Agent({ keepAlive: true });
  rec('keepalive-1', await send({ method: 'POST', path: '/echo', agent }, (r) => r.end('one')));
  rec('keepalive-2', await send({ method: 'POST', path: '/echo', agent }, (r) => r.end('two')));
  agent.destroy();

  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
