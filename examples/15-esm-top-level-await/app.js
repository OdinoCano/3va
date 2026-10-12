import { driver, query } from "./db/index.js";

console.log("driver:", driver);
console.log(await query("SELECT 1"));

// import() also waits for a module that is still doing real async work.
const late = await import("./late.js");
console.log("late:", late.value);
