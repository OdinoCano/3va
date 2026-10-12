const v = await Promise.resolve(42);
export default v;
export const w = await new Promise(r => setTimeout(() => r("late"), 20));
