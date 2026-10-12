import { eol, query } from "./db.js";
import { ok } from "./t1.js";
import v from "./t2.js";
import { r } from "./t6.js";
import { out } from "./t8.js";
import { res } from "./t9.js";
import k from "./t11.js";
import { again } from "./t12.js";
import { q, x } from "./nofn.js";
import { a } from "./t7.js";
const dyn = await import("./t2.js");
globalThis.__out = JSON.stringify({
  eol, sep: await query(), ok, v, r, a, out, res, k, again, x, q: await q(), dynW: dyn.w,
});
