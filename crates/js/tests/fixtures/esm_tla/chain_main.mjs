import { getThing, VALUE } from './chain_index.js';
import { viaMid } from './chain_mid.js';
globalThis.__out = [typeof getThing, VALUE, viaMid].join(' ');
