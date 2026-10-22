// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const out = [];

function fresh() {
  return new http.ServerResponse(new http.IncomingMessage({}));
}

// A CR/LF in the reason phrase would put a second header on the status line.
{
  const res = fresh();
  try {
    res.writeHead(200, 'OK\r\nSet-Cookie: pwned=1');
    out.push('writeHead-injection: no-throw');
  } catch (err) {
    out.push(
      'writeHead-injection: ' +
        [err.code, err instanceof TypeError, err.message, res.headersSent].join('|'),
    );
  }
}

// The same value assigned directly: the error comes when the head is written.
{
  const res = fresh();
  res.statusMessage = 'OK\r\nX-Evil: 1';
  try {
    res.end('x');
    out.push('direct-injection: no-throw');
  } catch (err) {
    out.push('direct-injection: ' + [err.code, err.message].join('|'));
  }
}

// Node records the status line before validating the header list.
{
  const res = fresh();
  try {
    res.writeHead(207, { 'X-Foo': 'a\r\nb' });
    out.push('writeHead-bad-header: no-throw');
  } catch (err) {
    out.push(
      'writeHead-bad-header: ' +
        [err.code, res.statusCode, res.statusMessage, res.headersSent].join('|'),
    );
  }
}

// Tab is allowed; a non-string second argument is the headers object.
{
  const res = fresh();
  res.writeHead(200, 'OK\tthere');
  out.push('tab: ' + JSON.stringify(res.statusMessage));
}
{
  const res = fresh();
  res.writeHead(201, 42);
  out.push('headers-arg: ' + res.statusCode + '|' + JSON.stringify(res.statusMessage));
}

process.stdout.write(out.join('\n') + '\n');
