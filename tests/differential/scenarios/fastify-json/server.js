// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const crypto = require('crypto');

const fastify = require('fastify')({ logger: false });
const sha256 = (b) => crypto.createHash('sha256').update(Buffer.from(b)).digest('hex');
const BINARY = Buffer.alloc(256);
for (let i = 0; i < 256; i++) BINARY[i] = i;

fastify.addContentTypeParser('application/octet-stream', { parseAs: 'buffer' }, (req, body, done) => done(null, body));

fastify.post('/echo', async (req) => ({ got: req.body === undefined ? '<undefined>' : req.body }));
fastify.post('/raw', async (req) => {
  const b = Buffer.isBuffer(req.body) ? req.body : Buffer.alloc(0);
  return { len: b.length, sha: sha256(b) };
});
fastify.get('/binary', async (req, reply) => {
  reply.header('content-type', 'application/octet-stream');
  return BINARY;
});

fastify.listen({ port: Number(process.argv[2]), host: '127.0.0.1' }).then(() => process.stdout.write('READY\n'));
