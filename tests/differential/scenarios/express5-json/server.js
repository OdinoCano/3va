// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

require('../../lib/express-json.js').serve(require('express5'), Number(process.argv[2]));
