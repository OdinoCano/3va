// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// A TLS server using the private CA the runner generates in the work dir
// (certs/). Clients trust it through NODE_EXTRA_CA_CERTS.
'use strict';

const https = require('https');
const fs = require('fs');

function serve(port) {
  const opts = {
    key: fs.readFileSync('certs/server-key.pem'),
    cert: fs.readFileSync('certs/server-cert.pem'),
  };
  const srv = https.createServer(opts, (req, res) => {
    res.writeHead(200, { 'content-type': 'text/plain' });
    res.end('secure-ok');
  });
  srv.listen(port, '127.0.0.1', () => process.stdout.write('READY\n'));
  return srv;
}

module.exports = { serve };
