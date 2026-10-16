// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// The scenario's test client: drives the whole flow through nginx and exits
// non-zero if any check fails. Runs on node 24 in its own container.
'use strict';

const crypto = require('crypto');

const BASE = process.env.BASE_URL || 'http://nginx';
const results = [];

function check(name, ok, detail) {
  results.push({ name, ok, detail: detail === undefined ? '' : String(detail) });
}

async function waitReady(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let last = '';
  while (Date.now() < deadline) {
    try {
      const r = await fetch(BASE + '/healthz');
      if (r.status === 200) return true;
    } catch (e) {
      last = e.message;
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error('app never became ready: ' + last);
}

async function main() {
  await waitReady(90000);

  const login = await fetch(BASE + '/login', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ user: 'demo', password: 'secret' }),
  });
  check('login-status', login.status === 200, login.status);
  const setCookie = login.headers.get('set-cookie') || '';
  check('login-sets-cookie', setCookie.includes('connect.sid'), setCookie.slice(0, 40));
  const cookie = setCookie.split(';')[0];

  const me = await fetch(BASE + '/me', { headers: { cookie } });
  const meBody = await me.json();
  check('me-status', me.status === 200, me.status);
  check('me-user', meBody.user === 'demo', JSON.stringify(meBody));

  const meAnon = await fetch(BASE + '/me');
  check('me-without-cookie', meAnon.status === 401, meAnon.status);

  const payload = Buffer.from(Array.from({ length: 8192 }, (_, i) => i & 0xff));
  const sha = crypto.createHash('sha256').update(payload).digest('hex');
  const fd = new FormData();
  fd.append('file', new Blob([payload], { type: 'application/octet-stream' }), 'blob.bin');
  const upload = await fetch(BASE + '/upload', { method: 'POST', headers: { cookie }, body: fd });
  const uploadBody = await upload.json();
  check('upload-status', upload.status === 200, upload.status);
  check('upload-size', uploadBody.size === payload.length, uploadBody.size);
  check('upload-sha', uploadBody.sha === sha, uploadBody.sha);

  const dl = await fetch(BASE + '/download/' + uploadBody.id);
  const dlBytes = Buffer.from(await dl.arrayBuffer());
  const dlSha = crypto.createHash('sha256').update(dlBytes).digest('hex');
  check('download-status', dl.status === 200, dl.status);
  check('download-sha', dlSha === sha, dlSha);

  const upstream = await fetch(BASE + '/upstream');
  const upstreamBody = await upstream.json();
  check('upstream-status', upstream.status === 200, upstream.status);
  check(
    'upstream-data',
    upstreamBody.ok === true && upstreamBody.upstream && upstreamBody.upstream.mock === 'ok' && upstreamBody.upstream.n === 42,
    JSON.stringify(upstreamBody)
  );

  const ip = await fetch(BASE + '/ip');
  const ipBody = await ip.json();
  check('proxy-remote-address', ipBody.remoteAddress === '203.0.113.7', JSON.stringify(ipBody));

  const db = await fetch(BASE + '/db');
  const dbBody = await db.json();
  check('db-status', db.status === 200, db.status);
  check('db-value', dbBody.ok === true && dbBody.row && dbBody.row.value === 'seed-ok', JSON.stringify(dbBody));

  let failed = 0;
  for (const r of results) {
    if (!r.ok) failed++;
    console.log((r.ok ? 'ok   ' : 'FAIL ') + r.name + (r.detail ? '  (' + r.detail + ')' : ''));
  }
  console.log('---');
  console.log(results.length - failed + '/' + results.length + ' checks passed');
  process.exit(failed === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error('scenario error: ' + (e && e.stack ? e.stack : e));
  process.exit(1);
});
