// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const http = require('http');
const codes = [200, 201, 204, 301, 302, 304, 400, 401, 403, 404, 405, 409, 413, 415, 422, 429, 500, 502, 503];
const out = [];
for (const c of codes) out.push(c + '\t' + String(http.STATUS_CODES[c]));
process.stdout.write(out.join('\n') + '\n');
