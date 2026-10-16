// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// An express app used by the axios scenario: a JSON route, a raw-bytes route
// (hash only, so nothing binary is printed), a multer upload route and a fixed
// binary download.
'use strict';

const multer = require('multer');
const { sha256 } = require('./common.js');

function serve(express, port) {
  const app = express();
  const upload = multer({ storage: multer.memoryStorage() });

  app.post('/json', express.json(), (req, res) => {
    res.json({ got: req.body === undefined ? '<undefined>' : req.body });
  });

  app.post('/raw', express.raw({ type: '*/*' }), (req, res) => {
    const b = Buffer.isBuffer(req.body) ? req.body : Buffer.alloc(0);
    res.json({ len: b.length, sha: sha256(b), hex: b.length <= 16 ? b.toString('hex') : null });
  });

  app.post('/upload', upload.single('file'), (req, res) => {
    const f = req.file;
    res.json({
      hasFile: !!f,
      field: f ? f.fieldname : null,
      filename: f ? f.originalname : null,
      size: f ? f.size : null,
      sha: f ? sha256(f.buffer) : null,
      fields: req.body,
    });
  });

  app.get('/binary', (req, res) => {
    const b = Buffer.alloc(256);
    for (let i = 0; i < 256; i++) b[i] = i;
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': '256' });
    res.end(b);
  });

  app.use((err, req, res, next) => {
    res.status(err.status || 500).json({ error: err.type || err.name || 'error' });
  });

  app.listen(port, '127.0.0.1', () => process.stdout.write('READY\n'));
}

module.exports = { serve };
