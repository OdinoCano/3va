const os = await import("node:os");
export const eol = os.EOL;
export async function query() { const { default: p } = await import("node:path"); return p.sep; }
