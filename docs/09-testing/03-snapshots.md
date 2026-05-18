# 03 - SNAPSHOTS

## 3.1 Snapshots

Los snapshots guardan serializaciones de valores para comparar.

## 3.2 Uso

```javascript
// Crear snapshot
test("renders correctly", () => {
  const tree = render(<App />);
  expect(tree).toMatchSnapshot();
});

// Actualizar snapshots
3va test --update-snapshots
```

## 3.3 Ubicación

```
__snapshots__/
├── App.test.js.snap
└── utils.test.js.snap
```

## 3.4 Formato

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
// Snapshot inline en el test
expect(value).toMatchInlineSnapshot(`
  <div>value</div>
`);
```

---

*Snapshots conforme a Jest snapshot.*