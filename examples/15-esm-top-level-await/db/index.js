// A "db" module that needs top-level await: load the driver lazily, then export
// a ready-to-use handle. Same shape as a typical app's db/index.js.
const { default: path } = await import("node:path");

export const driver = path.sep === "/" ? "posix" : "win32";

export async function query(sql) {
  const { default: os } = await import("node:os");
  return `[${driver}] ${sql}${os.EOL}`.trimEnd();
}
