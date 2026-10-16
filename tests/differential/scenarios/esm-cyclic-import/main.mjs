// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
import { aValue } from './cycle-a.js';
import { seenA } from './cycle-b.js';

process.stdout.write('cycle\t' + aValue + '\t' + seenA() + '\n');
