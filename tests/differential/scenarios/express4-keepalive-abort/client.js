// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const port = Number(process.argv[2]);

function post(path, body) {
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, method: 'POST', path, headers: { 'content-type': 'application/json' } }, (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve(res.statusCode + '\t' + Buffer.concat(cs).toString('utf8')));
    });
    r.on('error', reject);
    r.end(body);
  });
}

(async () => {
  // Abort an upload half way through.
  await new Promise((resolve) => {
    const a = http.request(
      { host: '127.0.0.1', port, method: 'POST', path: '/plain/echo', headers: { 'content-type': 'application/json', 'content-length': '100000' } },
      () => {}
    );
    a.on('error', () => {});
    a.write('{"a":');
    setTimeout(() => {
      a.destroy();
      resolve();
    }, 30);
  });

  await new Promise((r) => setTimeout(r, 80));
  process.stdout.write('after-abort\t' + (await post('/plain/echo', '{"ok":true}')) + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
