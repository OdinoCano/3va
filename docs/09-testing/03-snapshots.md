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
├── App.test.js.snap.json
└── utils.test.js.snap.json
```

## 3.4 Format

```javascript
// __snapshots__/test.js.snap.json
{ "<test name 1>": "<div>\n  <h1>Hello</h1>\n</div>" }
```

Snapshots are stored as **JSON** files. Each test name maps to its serialized value.

## 3.5 Inline Snapshots

```javascript
// Inline snapshot in the test
expect(value).toMatchInlineSnapshot(`<div>value</div>`);
```

---

*Snapshots stored as JSON (`.snap.json`) for portability.*
