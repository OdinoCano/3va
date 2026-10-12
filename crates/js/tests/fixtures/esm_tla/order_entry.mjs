import { gate } from "./order_setup.js";
const logs = [];
const b = import("./order_b.js").finally(() => logs.push("B"));
const a = import("./order_a.js").finally(() => logs.push("A"));
setTimeout(() => gate.resolve(), 20);
await Promise.all([a, b]);
let dup = "no error";
try { await import("./dupattr.js"); } catch (e) { dup = e.name; }
globalThis.__out = JSON.stringify({ logs, dup });
