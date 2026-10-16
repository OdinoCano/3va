// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const net = require('net');
const port = Number(process.argv[2]);
const out = [];

function once(payload, delayed) {
  return new Promise((resolve, reject) => {
    const c = net.connect({ port, host: '127.0.0.1' }, () => {
      if (delayed) {
        c.write(payload);
        setTimeout(() => c.end(), 30);
      } else {
        c.end(payload);
      }
    });
    const cs = [];
    c.on('data', (d) => cs.push(Buffer.from(d)));
    c.on('end', () => resolve(Buffer.concat(cs).toString('hex')));
    c.on('error', reject);
  });
}

(async () => {
  out.push('end-data\t' + (await once(Buffer.from([0, 255, 104, 105, 200]), false)));
  out.push('delayed-end\t' + (await once(Buffer.from('hi'), true)));
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
