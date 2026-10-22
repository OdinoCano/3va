// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const port = Number(process.argv[2]);
const out = [];

function send(opts, fn) {
  return new Promise((resolve, reject) => {
    const r = http.request(Object.assign({ host: '127.0.0.1', port }, opts), (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(cs) }));
    });
    r.on('error', reject);
    fn(r);
  });
}

const rec = (label, r) => out.push(label + '\t' + r.status + '\t' + r.body.toString('utf8'));

(async () => {
  rec(
    'multi-write',
    await send({ method: 'POST', path: '/echo' }, (r) => {
      r.write('ab');
      setTimeout(() => {
        r.write(Buffer.from([0, 255]));
        r.end('cd');
      }, 20);
    })
  );
  rec('flush-then-write', await send({ method: 'POST', path: '/echo' }, (r) => { r.flushHeaders(); r.write('xy'); r.end('z'); }));
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
