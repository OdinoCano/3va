# 02 - MATCHERS AND ASSERTIONS

## 2.1 Introduction

Matchers are invoked on the object returned by `expect(value)`. All support negation by prepending `.not`:

```javascript
expect(value).toBe(expected);
expect(value).not.toBe(unexpected);
```

## 2.2 Equality

| Matcher | Description |
|---------|-------------|
| `.toBe(expected)` | Strict equality (`===`) |
| `.toEqual(expected)` | Deep equality (via JSON serialization) |
| `.toStrictEqual(expected)` | Alias of `.toEqual` (same JSON-based comparison) |

```javascript
expect(2 + 2).toBe(4);
expect({ a: 1, b: [2] }).toEqual({ a: 1, b: [2] });
```

## 2.3 Nullity and Definition

| Matcher | Description |
|---------|-------------|
| `.toBeNull()` | Value is `null` |
| `.toBeUndefined()` | Value is `undefined` |
| `.toBeDefined()` | Value is not `undefined` |

```javascript
expect(null).toBeNull();
expect(undefined).toBeUndefined();
expect(42).toBeDefined();
```

## 2.4 Truthiness

| Matcher | Description |
|---------|-------------|
| `.toBeTruthy()` | `Boolean(value) === true` |
| `.toBeFalsy()` | `Boolean(value) === false` |

```javascript
expect(1).toBeTruthy();
expect("").toBeFalsy();
expect(0).toBeFalsy();
```

## 2.5 Numeric Comparison

| Matcher | Description |
|---------|-------------|
| `.toBeGreaterThan(n)` | `value > n` |
| `.toBeLessThan(n)` | `value < n` |
| `.toBeGreaterThanOrEqual(n)` | `value >= n` |
| `.toBeLessThanOrEqual(n)` | `value <= n` |

```javascript
expect(10).toBeGreaterThan(5);
expect(3).toBeLessThan(10);
expect(5).toBeGreaterThanOrEqual(5);
expect(4).toBeLessThanOrEqual(4);
```

## 2.6 Collections and Strings

| Matcher | Description |
|---------|-------------|
| `.toContain(item)` | Array includes the item, or string includes the substring |
| `.toHaveLength(n)` | `value.length === n` |
| `.toMatch(pattern)` | String matches a `RegExp` (or a string compiled to one) |

```javascript
expect([1, 2, 3]).toContain(2);
expect("hello world").toContain("world");
expect([1, 2, 3]).toHaveLength(3);
expect("abc").toHaveLength(3);
expect("user-42").toMatch(/user-\d+/);
```

## 2.7 Exceptions

| Matcher | Description |
|---------|-------------|
| `.toThrow()` | Function throws any error when invoked |
| `.toThrow(msg)` | Function throws an error whose message contains `msg` |

The value passed to `expect` must be a function; the matcher invokes it internally.

```javascript
expect(() => {
  throw new Error("fail");
}).toThrow();

expect(() => {
  throw new Error("invalid input");
}).toThrow("invalid");

expect(() => {
  return 42;
}).not.toThrow();
```

## 2.8 Snapshots

| Matcher | Description |
|---------|-------------|
| `.toMatchSnapshot(hint?)` | Creates or compares against a snapshot on disk. The snapshot key is the suite prefix + test name (+ `hint`); pass distinct hints for multiple snapshots inside the same test. |
| `.toMatchInlineSnapshot(expected?)` | Compares against an inline string. With no argument, prints the serialized value to the console (first-run helper). |

The first run saves the serialized value. Subsequent runs fail if the value differs. Use `3va test --update-snapshots` to overwrite outdated snapshots.

```javascript
test("user structure", () => {
  const user = { id: 1, name: "Ana", active: true };
  expect(user).toMatchSnapshot("user-structure");
});

test("inline", () => {
  expect("<div>value</div>").toMatchInlineSnapshot(`<div>value</div>`);
});
```

## 2.9 Lifecycle Hooks

Not matchers, but part of the injected framework — scoped to the `describe`
block where they are registered (root-level hooks apply to all tests):

| Hook | When it runs |
|------|--------------|
| `beforeAll(fn)` | Once, before the first test of the scope |
| `afterAll(fn)` | Once, after the last test of the scope |
| `beforeEach(fn)` | Before every test of the scope |
| `afterEach(fn)` | After every test of the scope |

---

*Matchers implemented as inline JavaScript in `crates/test/src/runner.rs` (`TEST_FRAMEWORK_JS`).*
