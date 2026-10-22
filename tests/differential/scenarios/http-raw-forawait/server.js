// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const fs = require('fs');
const crypto = require('crypto');
const { pipeline } = require('stream/promises');

const sha256 = (b) => crypto.createHash('sha256').update(Buffer.from(b)).digest('hex');
const OUT = 'pipeline-body.bin';

const srv = http.createServer(async (req, res) => {
  try {
    if (req.url === '/forawait') {
      const cs = [];
      for await (const c of req) cs.push(Buffer.from(c));
      res.end('forawait\t' + sha256(Buffer.concat(cs)) + '\t' + Buffer.concat(cs).length);
    } else if (req.url === '/pipeline') {
      await pipeline(req, fs.createWriteStream(OUT));
      const b = fs.readFileSync(OUT);
      res.end('pipeline\t' + sha256(b) + '\t' + b.length);
    } else {
      res.statusCode = 404;
      res.end('nope');
    }
  } catch (e) {
    res.statusCode = 500;
    res.end('err:' + e.message);
  }
});
srv.listen(Number(process.argv[2]), '127.0.0.1', () => process.stdout.write('READY\n'));
