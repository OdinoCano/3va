let cfg; try { cfg = await import("node:os"); } catch { cfg = null; }
export const ok = typeof cfg?.EOL;
