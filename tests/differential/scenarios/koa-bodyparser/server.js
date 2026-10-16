// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const Koa = require('koa');
const bodyParser = require('koa-bodyparser');

const app = new Koa();
app.use(bodyParser());
app.use(async (ctx) => {
  if (ctx.path === '/echo') {
    const b = ctx.request.body;
    ctx.body = { got: b === undefined ? '<undefined>' : b, type: typeof b };
  }
});

app.listen(Number(process.argv[2]), '127.0.0.1', () => process.stdout.write('READY\n'));
