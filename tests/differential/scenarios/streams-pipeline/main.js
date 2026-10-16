// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const stream = require('stream');
const { pipeline } = require('stream/promises');
const out = [];

async function collect(src) {
  const chunks = [];
  await pipeline(src, new stream.Writable({ write(c, e, cb) { chunks.push(Buffer.from(c).toString()); cb(); } }));
  return chunks.join('');
}

(async () => {
  async function* gen() { yield 'a'; yield 'b'; yield 'c'; }
  out.push('generators\t' + (await collect(gen())));
  out.push('from-list\t' + (await collect(stream.Readable.from(['x', 'y']))));
  let err = 'none';
  try {
    const bad = new stream.Transform({ transform(c, e, cb) { cb(new Error('boom')); } });
    await pipeline(stream.Readable.from(['a']), bad, new stream.Transform({ transform(c, e, cb) { cb(null, c); } }));
  } catch (e) {
    err = e.message;
  }
  out.push('mid-error\t' + err);
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
