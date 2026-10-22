// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

require('../../lib/express-json.js').run(Number(process.argv[2])).catch((e) => {
  process.stdout.write('fatal\t' + (e && e.message) + '\n');
  process.exit(1);
});
