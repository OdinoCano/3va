// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// The mock upstream the app calls with axios. Runs on plain node.
'use strict';

const http = require('http');

http
  .createServer((req, res) => {
    if (req.url === '/data') {
      res.writeHead(200, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ mock: 'ok', n: 42 }));
    }
    res.writeHead(404).end();
  })
  .listen(1234, '0.0.0.0', () => console.log('MOCK READY'));
