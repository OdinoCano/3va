// 04 - TypeScript
// 3va runs TypeScript natively — no tsc, no tsconfig needed.

interface Calculator {
  add(a: number, b: number): number;
  subtract(a: number, b: number): number;
  multiply(a: number, b: number): number;
  divide(a: number, b: number): number;
}

class SimpleCalculator implements Calculator {
  private history: string[] = [];

  add(a: number, b: number): number {
    const result = a + b;
    this.history.push(`${a} + ${b} = ${result}`);
    return result;
  }

  subtract(a: number, b: number): number {
    const result = a - b;
    this.history.push(`${a} - ${b} = ${result}`);
    return result;
  }

  multiply(a: number, b: number): number {
    const result = a * b;
    this.history.push(`${a} * ${b} = ${result}`);
    return result;
  }

  divide(a: number, b: number): number {
    if (b === 0) throw new Error("Division by zero");
    const result = a / b;
    this.history.push(`${a} / ${b} = ${result}`);
    return result;
  }

  getHistory(): readonly string[] {
    return this.history;
  }
}

// Generic utility
function first<T>(arr: T[]): T {
  if (arr.length === 0) throw new Error("Empty array");
  return arr[0];
}

// Usage
const calc = new SimpleCalculator();

console.log("=== TypeScript in 3va ===\n");

console.log(`  10 + 5 = ${calc.add(10, 5)}`);
console.log(`  10 - 3 = ${calc.subtract(10, 3)}`);
console.log(`   4 * 7 = ${calc.multiply(4, 7)}`);
console.log(`  20 / 6 = ${calc.divide(20, 6).toFixed(4)}`);

console.log("\nHistory:");
calc.getHistory().forEach((entry) => console.log(`  ${entry}`));

console.log(`\nFirst result: ${first(calc.getHistory())}`);
