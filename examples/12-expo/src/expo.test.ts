// 12 - Expo: tests de los helpers (Jest-compatible).

import { themeColor, Color } from "./theme";
import { truncate, pluralize, capitalize, formatCount, slug } from "./text";
import { formatRelative, formatShortDate } from "./date";

describe("themeColor", () => {
  test("dark mode returns dark palette", () => {
    expect(themeColor("primary", true)).toBe(Color.dark.primary);
  });

  test("light mode returns light palette", () => {
    expect(themeColor("danger", false)).toBe(Color.light.danger);
  });
});

describe("truncate", () => {
  test("shorter strings pass through", () => {
    expect(truncate("hi", 10)).toBe("hi");
  });

  test("longer strings are truncated with ellipsis", () => {
    expect(truncate("123456789", 6)).toBe("12345…");
  });
});

describe("pluralize", () => {
  test("singular", () => expect(pluralize(1, "item", "items")).toBe("item"));
  test("plural", () => expect(pluralize(3, "item", "items")).toBe("items"));
});

describe("capitalize", () => {
  test("normal", () => expect(capitalize("hello")).toBe("Hello"));
  test("empty", () => expect(capitalize("")).toBe(""));
});

describe("formatCount", () => {
  test("numbers below 1000", () => expect(formatCount(42)).toBe("42"));
  test("K suffix", () => expect(formatCount(15200)).toBe("15.2K"));
  test("M suffix", () => expect(formatCount(2300000)).toBe("2.3M"));
});

describe("slug", () => {
  test("slugifies strings", () => expect(slug("Hello World!")).toBe("hello-world"));
  test("handles multiple spaces", () => expect(slug("a  b  c")).toBe("a-b-c"));
});

describe("formatRelative", () => {
  test("now", () => {
    const s = formatRelative(Date.now());
    expect(s).toBe("ahora");
  });

  test("future date", () => {
    const s = formatRelative(Date.now() + 30_000);
    expect(s).toMatch(/^en \d+/);
  });

  test("past date", () => {
    const s = formatRelative(Date.now() - 3600_000);
    expect(s).toBe("hace 1h");
  });
});

describe("formatShortDate", () => {
  test("formats as dd/mm/yyyy", () => {
    expect(formatShortDate(new Date(2026, 0, 5).getTime())).toBe("05/01/2026");
  });
});

describe("snapshot theme", () => {
  test("all dark mode colors", () => {
    expect(Color.dark).toMatchSnapshot();
  });
});