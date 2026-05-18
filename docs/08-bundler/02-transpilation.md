# 02 - TRANSPILACIÓN TS/JSX

## 2.1 Transpilador

El transpilador convierte TypeScript y JSX a JavaScript compatible con el runtime.

## 2.2 TypeScript

### 2.2.1 Soporte

| Feature | Soporte |
|---------|---------|
| Tipos | Elimina en tiempo de compilación |
| Interfaces | Elimina |
| Enums | Convierte a objetos |
| Generics | Elimina con verificación |
| Decorators | Soporte parcial |
| Namespace | Convierte a IIFE |
| Async/await | Soportado |
| Nullish coalescing | Soportado |
| Optional chaining | Soportado |

### 2.2.2 Ejemplo

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

### 2.3.1 Transformaciones

| JSX | Output |
|-----|--------|
| <div /> | React.createElement("div", null) |
| <div className="x" /> | React.createElement("div", { className: "x" }) |
| <div>{text}</div> | React.createElement("div", null, text) |

### 2.3.2 Configuración

```javascript
// 3va.config.js
module.exports = {
  jsx: "react",           // react, react-jsx, preserve
  jsxImportSource: "react", // package para jsx
};
```

## 2.4 Plugins

| Plugin | Descripcion |
|--------|-------------|
| @3va/plugin-react | Soporte React |
| @3va/plugin-node | Polyfills Node.js |
| @3va/plugin-paths | Resolve paths |

---

*Transpilación conforme a TypeScript compiler API.*