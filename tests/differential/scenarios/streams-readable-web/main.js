// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const stream = require('stream');
const out = [];

function collect(r) {
  return new Promise((resolve, reject) => {
    const cs = [];
    r.on('data', (c) => cs.push(Buffer.from(c)));
    r.on('end', () => resolve(Buffer.concat(cs).toString()));
    r.on('error', reject);
  });
}

(async () => {
  out.push('from\t' + (await collect(stream.Readable.from(['a', 'b']))));
  const rs = new ReadableStream({
    start(c) {
      c.enqueue(new TextEncoder().encode('xy'));
      c.close();
    },
  });
  out.push('fromWeb\t' + (await collect(stream.Readable.fromWeb(rs))));
  const web = stream.Readable.toWeb(stream.Readable.from(['pq']));
  const reader = web.getReader();
  const parts = [];
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    parts.push(Buffer.from(value));
  }
  out.push('toWeb\t' + Buffer.concat(parts).toString());
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
