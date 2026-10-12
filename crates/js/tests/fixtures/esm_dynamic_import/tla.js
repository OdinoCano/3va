globalThis.__depStarted = true;
globalThis.__resolved = new Promise((resolve) => {
  globalThis.__finishDep = resolve;
});
await globalThis.__resolved;
globalThis.__depEnded = true;
export const late = 'late';
