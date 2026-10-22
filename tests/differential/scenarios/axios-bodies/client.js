// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const axios = require('axios');
const FormData = require('form-data');
const crypto = require('crypto');
const port = Number(process.argv[2]);
const base = 'http://127.0.0.1:' + port;
const out = [];

const sha256 = (b) => crypto.createHash('sha256').update(Buffer.from(b)).digest('hex');

(async () => {
  out.push('json\t' + JSON.stringify((await axios.post(base + '/json', { a: 1, b: 'x' })).data));

  const raw = await axios.post(base + '/raw', Buffer.from([0, 255, 104, 105, 200]), {
    headers: { 'content-type': 'application/octet-stream' },
  });
  out.push('raw\t' + JSON.stringify(raw.data));

  const fd = new FormData();
  fd.append('file', Buffer.from([0, 255, 1, 2, 3]), { filename: 'blob.bin' });
  fd.append('note', 'x');
  const up = await axios.post(base + '/upload', fd, { headers: fd.getHeaders() });
  out.push('upload\t' + JSON.stringify(up.data));

  const bin = await axios.get(base + '/binary', { responseType: 'arraybuffer' });
  out.push('binary\tsha256=' + sha256(bin.data));

  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
