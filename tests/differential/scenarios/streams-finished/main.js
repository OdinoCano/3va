// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const stream = require('stream');
const out = [];

async function main() {
  await new Promise((resolve) => {
    const p = new stream.PassThrough();
    let calls = 0;
    stream.finished(p, () => {
      calls++;
    });
    p.on('data', () => {});
    p.end('x');
    setTimeout(() => {
      out.push('duplex-calls\t' + calls);
      resolve();
    }, 50);
  });

  await new Promise((resolve) => {
    const w = new stream.Writable({ write(c, e, cb) { cb(new Error('nope')); } });
    stream.finished(w, (err) => {
      out.push('error\t' + (err ? err.message : 'none'));
      resolve();
    });
    w.on('error', () => {});
    w.write('x');
  });

  process.stdout.write(out.join('\n') + '\n');
}

main().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
