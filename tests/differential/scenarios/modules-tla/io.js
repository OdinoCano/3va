// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
import { readFile } from 'fs/promises';

const txt = await readFile('scenarios/modules-tla/data.txt', 'utf8');
export const ioValue = txt.trim();
