# 02 - TS/JSX TRANSPILATION

## 2.1 Transpiler

The transpiler converts TypeScript and JSX to runtime-compatible JavaScript.

## 2.2 TypeScript

### 2.2.1 Support

| Feature | Support |
|---------|---------|
| Types | Stripped at compile time |
| Interfaces | Stripped |
| Enums | Converted to objects |
| Generics | Stripped with verification |
| Decorators | Partial support |
| Namespace | Converted to IIFE |
| Async/await | Supported |
| Nullish coalescing | Supported |
| Optional chaining | Supported |

### 2.2.2 Example

```typescript
// Input
interface User {
  name: string;
  age?: number;
}

const user: User = { name: "John" };
const age = user.age ?? 0;

// Output
const user = { name: "John" };
const age = user.age !== null && user.age !== void 0 ? user.age : 0;
```

## 2.3 JSX

### 2.3.1 Transformations

| JSX | Output |
|-----|--------|
| <div /> | React.createElement("div", null) |
| <div className="x" /> | React.createElement("div", { className: "x" }) |
| <div>{text}</div> | React.createElement("div", null, text) |

### 2.3.2 Configuration

```javascript
// 3va.config.js
module.exports = {
  jsx: "react",           // react, react-jsx, preserve
  jsxImportSource: "react", // package for jsx
};
```

## 2.4 Plugins

| Plugin | Description |
|--------|-------------|
| @3va/plugin-react | React support |
| @3va/plugin-node | Node.js polyfills |
| @3va/plugin-paths | Resolve paths |

---

*Transpilation conforming to TypeScript compiler API.*
