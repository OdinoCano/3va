// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

const fs = require('fs');
const crypto = require('crypto');
const zlib = require('zlib');
const out = [];

const sha = (b) => crypto.createHash('sha256').update(b).digest('hex');
const buf = Buffer.from([0, 255, 1, 2, 3, 250]);

fs.writeFileSync('binary-modules.bin', buf);
const back = fs.readFileSync('binary-modules.bin');
out.push('fs\t' + sha(back) + '\t' + Buffer.isBuffer(back) + '\t' + back.length);
out.push('fs-Uint8Array\t' + sha(new Uint8Array(back)));
out.push('hash-u8\t' + sha(new Uint8Array([9, 8, 7, 6])));

const def = zlib.deflateSync(buf);
out.push('zlib-roundtrip\t' + sha(zlib.inflateSync(def)) + '\t' + (Buffer.compare(zlib.inflateSync(def), buf) === 0));
const gz = zlib.gzipSync(buf);
out.push('gzip-roundtrip\t' + sha(zlib.gunzipSync(gz)));
out.push('enc\t' + buf.toString('hex') + '\t' + buf.toString('base64'));

process.stdout.write(out.join('\n') + '\n');
