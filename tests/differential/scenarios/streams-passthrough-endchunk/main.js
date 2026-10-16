// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const stream = require('stream');
const out = [];

function collectTransform(t, label) {
  return new Promise((resolve) => {
    const got = [];
    t.on('data', (d) => got.push(Buffer.from(d).toString('hex')));
    t.on('end', () => {
      out.push(label + '\t' + got.join(''));
      resolve();
    });
    t.end(Buffer.from([0, 255, 104, 105, 200]));
  });
}

(async () => {
  await collectTransform(new stream.PassThrough(), 'passthrough');
  await collectTransform(new stream.Transform({ transform(c, e, cb) { cb(null, Buffer.from(c).reverse()); } }), 'transform');
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
