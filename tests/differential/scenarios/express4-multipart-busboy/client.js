// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const { send } = require('../../lib/multipart.js');
const port = Number(process.argv[2]);

(async () => {
  const r = await send(port, '/upload', [
    { file: true, name: 'file', value: Buffer.from([0, 255, 1, 2, 3]), filename: 'blob.bin', contentType: 'application/octet-stream' },
    { name: 'note', value: 'x' },
  ]);
  process.stdout.write('busboy\t' + r.status + '\t' + r.body + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
