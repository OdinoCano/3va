const name = "t3";
const m = await import(`./${name}.js`);
export const r = m.default();
