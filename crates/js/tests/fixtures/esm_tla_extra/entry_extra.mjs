import { a, c, d, x, rest } from './destructure.js';
import D, { Base } from './anondefault.js';

globalThis.__out = {
  a,
  c,
  d,
  x,
  rest,
  anonBase: new D() instanceof Base,
};
