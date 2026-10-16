// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// Send a multipart/form-data request over raw http using the `form-data`
// package. The random boundary never reaches the printed output: the servers
// reply with the parsed filename/size/sha and the fields.
'use strict';

const http = require('http');
const FormData = require('form-data');

function send(port, path, parts) {
  const fd = new FormData();
  for (const p of parts) {
    if (p.file) fd.append(p.name, p.value, { filename: p.filename, contentType: p.contentType });
    else fd.append(p.name, p.value);
  }
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, method: 'POST', path, headers: fd.getHeaders() }, (res) => {
      const cs = [];
      res.on('data', (c) => cs.push(Buffer.from(c)));
      res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(cs).toString('utf8') }));
    });
    r.on('error', reject);
    fd.pipe(r);
  });
}

module.exports = { send };
