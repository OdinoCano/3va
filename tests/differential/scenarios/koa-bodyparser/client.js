// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const port = Number(process.argv[2]);
const out = [];

function post(path, body, ct) {
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, method: 'POST', path, headers: { 'content-type': ct } }, (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve(res.statusCode + '\t' + Buffer.concat(cs).toString('utf8')));
    });
    r.on('error', reject);
    r.end(body);
  });
}

(async () => {
  out.push('json\t' + (await post('/echo', '{"a":1,"b":"x"}', 'application/json')));
  out.push('urlencoded\t' + (await post('/echo', 'a=1&b=x', 'application/x-www-form-urlencoded')));
  out.push('empty\t' + (await post('/echo', '', 'application/json')));
  process.stdout.write(out.join('\n') + '\n');
})().catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
