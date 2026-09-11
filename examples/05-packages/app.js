// 05 - Using npm Packages
// Install packages with 3va install and use them in code.

const isOdd = require("is-odd");
const ms = require("ms");

console.log("=== Package Usage ===\n");

// is-odd: a tiny type-checking package with real dependencies
console.log("Parity check (is-odd):");
for (const n of [1, 2, 7, 10, 99]) {
  console.log(`  isOdd(${n}) = ${isOdd(n)}`);
}

// ms: convert durations between strings and milliseconds
console.log("\nDurations (ms package):");
console.log(`  ms('2 days') = ${ms("2 days")} ms`);
console.log(`  ms('1.5h')   = ${ms("1.5h")} ms`);
console.log(`  ms(120000)   = ${ms(120000)}`);
console.log(`  ms(86400000) = ${ms(86400000)}`);