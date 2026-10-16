// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const crypto = require('crypto');
const port = Number(process.argv[2]);
const base = 'http://127.0.0.1:' + port;
const out = [];

const sha256 = (b) => crypto.createHash('sha256').update(Buffer.from(b)).digest('hex');
const rec = (label, t) => out.push(label + '\t' + t);
const recRes = async (label, res, kind) => {
  if (kind === 'hash') rec(label, res.status + '\tsha256=' + sha256(await res.arrayBuffer()));
  else rec(label, res.status + '\t' + (await res.text()));
};

(async () => {
  await recRes('string-body', await fetch(base + '/echo', { method: 'POST', body: 'hi' }), 'text');
  await recRes('buffer-body', await fetch(base + '/echo', { method: 'POST', body: Buffer.from([0, 255, 104, 105, 200]) }), 'text');
  await recRes('uint8array-body', await fetch(base + '/echo', { method: 'POST', body: new Uint8Array([0, 255, 104, 105, 200]) }), 'text');
  await recRes('arraybuffer-body', await fetch(base + '/echo', { method: 'POST', body: new Uint8Array([9, 8, 7]).buffer }), 'text');
  await recRes('urlsearchparams-body', await fetch(base + '/echo', { method: 'POST', body: new URLSearchParams({ a: '1', b: 'x' }) }), 'text');
  await recRes('binary-response', await fetch(base + '/binary'), 'hash');
  await recRes('chunked-response', await fetch(base + '/chunked'), 'text');
  await recRes('no-content', await fetch(base + '/nocontent'), 'text');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
