# 03 - SNAPSHOTS

## 3.1 Snapshots

Snapshots store serializations of values for comparison.

## 3.2 Usage

```javascript
// Create snapshot
test("renders correctly", () => {
  const tree = render(<App />);
  expect(tree).toMatchSnapshot();
});

// Update snapshots
3va test --update-snapshots
```

## 3.3 Location

```
__snapshots__/
├── App.test.js.snap
└── utils.test.js.snap
```

## 3.4 Format

```javascript
// __snapshots__/test.js.snap
exports[`test name 1`] = `
<div>
  <h1>Hello</h1>
</div>
`;
```

## 3.5 Inline Snapshots

```javascript
// Inline snapshot in the test
expect(value).toMatchInlineSnapshot(`
  <div>value</div>
`);
```

---

*Snapshots conforming to Jest snapshot.*
