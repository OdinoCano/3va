// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// Shared server + client for the express request-body matrix (express 4 and 5).
'use strict';

const { request, line } = require('./common.js');

function serve(express, port) {
  const app = express();

  // An async middleware *before* the body parser: in a single-process test this
  // path is usually never exercised, but it is what 2.12.x got wrong.
  const slow = async (req, res, next) => {
    await new Promise((r) => setTimeout(r, 5));
    next();
  };

  function echo(req, res) {
    const b = req.body;
    res.json({
      has: b !== undefined && b !== null,
      type: typeof b,
      keys: b && typeof b === 'object' && !Array.isArray(b) ? Object.keys(b) : null,
      value: b === undefined ? '<undefined>' : JSON.stringify(b),
    });
  }

  app.post('/plain/echo', express.json(), echo);
  app.post('/async/echo', slow, express.json(), echo);
  app.post('/url/echo', express.urlencoded({ extended: false }), echo);
  app.post('/text/echo', express.text({ type: '*/*' }), echo);
  app.post('/plain/count', express.json(), (req, res) => {
    res.json({ n: req.body && Array.isArray(req.body.items) ? req.body.items.length : -1 });
  });

  app.use((err, req, res, next) => {
    res.status(err.status || 500).json({ error: err.type || err.name || 'error' });
  });

  app.listen(port, '127.0.0.1', () => process.stdout.write('READY\n'));
}

async function run(port) {
  const J = { 'content-type': 'application/json' };
  const cases = [
    ['plain-json', { method: 'POST', path: '/plain/echo', headers: J, body: '{"a":1,"b":"x"}' }],
    ['async-json', { method: 'POST', path: '/async/echo', headers: J, body: '{"a":1,"b":"x"}' }],
    ['json-charset', { method: 'POST', path: '/plain/echo', headers: { 'content-type': 'application/json; charset=utf-8' }, body: '{"n":2}' }],
    ['json-array', { method: 'POST', path: '/plain/echo', headers: J, body: '[1,2,3]' }],
    ['empty-body', { method: 'POST', path: '/plain/echo', headers: J, body: '' }],
    ['no-body', { method: 'POST', path: '/plain/echo', headers: {} }],
    ['urlencoded', { method: 'POST', path: '/url/echo', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: 'a=1&b=x' }],
    ['text', { method: 'POST', path: '/text/echo', headers: { 'content-type': 'text/plain' }, body: 'hello' }],
    ['malformed-json', { method: 'POST', path: '/plain/echo', headers: J, body: '{bad' }],
    ['json-null', { method: 'POST', path: '/plain/echo', headers: J, body: 'null' }],
    ['json-number', { method: 'POST', path: '/plain/echo', headers: J, body: '42' }],
    ['large-json', { method: 'POST', path: '/plain/count', headers: J, body: JSON.stringify({ items: Array.from({ length: 4000 }, (_, i) => i) }) }],
  ];
  const out = [];
  for (const [label, opts] of cases) {
    out.push(line(label, await request(port, opts)));
  }
  process.stdout.write(out.join('\n') + '\n');
}

module.exports = { serve, run };
