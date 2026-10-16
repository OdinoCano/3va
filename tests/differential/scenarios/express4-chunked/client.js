// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const port = Number(process.argv[2]);
const out = [];

function send(parts) {
  return new Promise((resolve, reject) => {
    const r = http.request(
      { host: '127.0.0.1', port, method: 'POST', path: '/plain/echo', headers: { 'content-type': 'application/json' } },
      (res) => {
        const cs = [];
        res.on('data', (c) => cs.push(Buffer.from(c)));
        res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(cs).toString('utf8') }));
      }
    );
    r.on('error', reject);
    let i = 0;
    const next = () => {
      if (i < parts.length - 1) {
        r.write(parts[i++]);
        setTimeout(next, 15);
      } else {
        r.end(parts[i]);
      }
    };
    next();
  });
}

(async () => {
  out.push('split-json\t' + JSON.stringify(await send(['{"a":1,', '"b":"x"}'])));
  out.push('split-unicode\t' + JSON.stringify(await send(['{"s":"caf', 'é"}'])));
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
