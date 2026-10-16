// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const port = Number(process.argv[2]);
const out = [];

function post(path, chunks) {
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, method: 'POST', path }, (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve(res.statusCode + '\t' + Buffer.concat(cs).toString('utf8')));
    });
    r.on('error', reject);
    let i = 0;
    const next = () => {
      if (i < chunks.length - 1) {
        r.write(chunks[i++]);
        setTimeout(next, 15);
      } else {
        r.end(chunks[i]);
      }
    };
    next();
  });
}

(async () => {
  out.push('forawait-binary\t' + (await post('/forawait', [Buffer.from([0, 255, 104, 105, 200])])));
  out.push('forawait-chunked\t' + (await post('/forawait', [Buffer.from('ab'), Buffer.from([0, 255]), Buffer.from('cd')])));
  const big = Buffer.alloc(65536);
  for (let i = 0; i < big.length; i++) big[i] = (i * 7) & 0xff;
  out.push('pipeline-big\t' + (await post('/pipeline', [big.slice(0, 20000), big.slice(20000)])));
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
