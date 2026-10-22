// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
'use strict';

require('../../lib/axios-server.js').serve(require('express4'), Number(process.argv[2]));
