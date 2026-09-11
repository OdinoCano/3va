// 01 - Hello World
// The simplest possible 3va script. No permissions needed.

console.log("Hello from 3va!");
console.log(`Runtime: ${typeof Deno !== "undefined" ? "Deno" : "3va"}`);
console.log(`Node compat: ${typeof process !== "undefined"}`);
