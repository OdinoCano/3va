// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const port = Number(process.argv[2]);
const out = [];

function post(path, body, headers) {
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, method: 'POST', path, headers: headers || {} }, (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(cs).toString('utf8') }));
    });
    r.on('error', reject);
    r.end(body);
  });
}

const rec = (label, r) => out.push(label + '\t' + r.status + '\t' + r.body);

(async () => {
  rec('small-binary', await post('/raw', Buffer.from([0, 255, 104, 105, 200])));
  const big = Buffer.alloc(65536);
  for (let i = 0; i < big.length; i++) big[i] = i & 0xff;
  rec('large-binary', await post('/raw', big, { 'content-type': 'application/octet-stream' }));
  rec('empty-binary', await post('/raw', Buffer.alloc(0)));
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
