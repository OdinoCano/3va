// 07 - Test Runner
// Tests for math.js — run with `3va test` (Jest-compatible, no config).

import {
  add,
  subtract,
  multiply,
  divide,
  fibonacci,
  isPalindrome,
  toSlug,
} from "./math.js";

describe("basic arithmetic", () => {
  test("add returns the sum", () => {
    expect(add(2, 3)).toBe(5);
    expect(add(-1, 1)).toBe(0);
  });

  test("subtract returns the difference", () => {
    expect(subtract(10, 4)).toBe(6);
  });

  test("multiply returns the product", () => {
    expect(multiply(3, 7)).toBe(21);
    expect(multiply(0, 100)).toBe(0);
  });

  test("divide returns the quotient", () => {
    expect(divide(20, 5)).toBe(4);
  });

  test("divide by zero throws", () => {
    expect(() => divide(1, 0)).toThrow("Cannot divide by zero");
  });
});

describe("fibonacci", () => {
  test("known values", () => {
    expect(fibonacci(0)).toBe(0);
    expect(fibonacci(1)).toBe(1);
    expect(fibonacci(2)).toBe(1);
    expect(fibonacci(5)).toBe(5);
    expect(fibonacci(10)).toBe(55);
    expect(fibonacci(20)).toBe(6765);
  });

  test("negative input throws", () => {
    expect(() => fibonacci(-1)).toThrow("Input must be non-negative");
  });
});

describe("isPalindrome", () => {
  test("detects palindromes", () => {
    expect(isPalindrome("racecar")).toBe(true);
    expect(isPalindrome("A man, a plan, a canal: Panama")).toBe(true);
    expect(isPalindrome("hello world")).toBe(false);
  });
});

describe("toSlug", () => {
  test("converts strings to URL-friendly slugs", () => {
    expect(toSlug("Hello World!")).toBe("hello-world");
    expect(toSlug("  Multiple   Spaces  Here  ")).toBe("multiple-spaces-here");
    expect(toSlug("Hello & Goodbye")).toBe("hello-goodbye");
  });
});

describe("snapshot testing", () => {
  test("math result snapshot", () => {
    const results = {
      add: add(2, 3),
      multiply: multiply(4, 5),
      fibonacci: fibonacci(8),
    };
    expect(results).toMatchSnapshot();
  });
});