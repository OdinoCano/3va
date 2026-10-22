// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const https = require('https');
const port = Number(process.argv[2]);

const req = https.request(
  { host: '127.0.0.1', port, path: '/', method: 'GET', servername: 'localhost' },
  (res) => {
    let d = '';
    res.on('data', (c) => (d += c));
    res.on('end', () => process.stdout.write('tls\t' + res.statusCode + '\t' + d + '\n'));
  }
);
req.on('error', (e) => {
  process.stdout.write('error\t' + e.message + '\n');
  process.exit(1);
});
req.end();
