// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
import { aValue } from './cycle-a.js';

export const bValue = 'b';

export function seenA() {
  return aValue;
}
