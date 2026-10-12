const o = []; async function* g() { yield 1; yield 2; }
for await (const v of g()) o.push(v);
export const res = o;
