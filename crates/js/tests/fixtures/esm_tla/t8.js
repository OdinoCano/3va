const out = [];
for (const x of [1, 2]) { out.push(await Promise.resolve(x * 2)); }
if (out.length) { out.push(await Promise.resolve(9)); }
export { out };
