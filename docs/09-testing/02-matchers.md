# 02 - MATCHERS Y ASERCIONES

## 2.1 Introducción

Los matchers se invocan sobre el objeto devuelto por `expect(valor)`. Todos admiten negación anteponiendo `.not`:

```javascript
expect(valor).toBe(esperado);
expect(valor).not.toBe(noEsperado);
```

## 2.2 Igualdad

| Matcher | Descripción |
|---------|-------------|
| `.toBe(esperado)` | Igualdad estricta (`===`) |
| `.toEqual(esperado)` | Igualdad profunda (compara estructuras recursivamente) |

```javascript
expect(2 + 2).toBe(4);
expect({ a: 1, b: [2] }).toEqual({ a: 1, b: [2] });
```

## 2.3 Nulidad y Definición

| Matcher | Descripción |
|---------|-------------|
| `.toBeNull()` | El valor es `null` |
| `.toBeUndefined()` | El valor es `undefined` |
| `.toBeDefined()` | El valor no es `undefined` |

```javascript
expect(null).toBeNull();
expect(undefined).toBeUndefined();
expect(42).toBeDefined();
```

## 2.4 Veracidad

| Matcher | Descripción |
|---------|-------------|
| `.toBeTruthy()` | `Boolean(valor) === true` |
| `.toBeFalsy()` | `Boolean(valor) === false` |

```javascript
expect(1).toBeTruthy();
expect("").toBeFalsy();
expect(0).toBeFalsy();
```

## 2.5 Comparación Numérica

| Matcher | Descripción |
|---------|-------------|
| `.toBeGreaterThan(n)` | `valor > n` |
| `.toBeLessThan(n)` | `valor < n` |
| `.toBeGreaterThanOrEqual(n)` | `valor >= n` |
| `.toBeLessThanOrEqual(n)` | `valor <= n` |

```javascript
expect(10).toBeGreaterThan(5);
expect(3).toBeLessThan(10);
expect(5).toBeGreaterThanOrEqual(5);
expect(4).toBeLessThanOrEqual(4);
```

## 2.6 Colecciones y Cadenas

| Matcher | Descripción |
|---------|-------------|
| `.toContain(elemento)` | Array incluye el elemento, o la cadena incluye la subcadena |
| `.toHaveLength(n)` | `valor.length === n` |

```javascript
expect([1, 2, 3]).toContain(2);
expect("hola mundo").toContain("mundo");
expect([1, 2, 3]).toHaveLength(3);
expect("abc").toHaveLength(3);
```

## 2.7 Excepciones

| Matcher | Descripción |
|---------|-------------|
| `.toThrow()` | La función lanza cualquier error al invocarse |

El valor pasado a `expect` debe ser una función; el matcher la invoca internamente.

```javascript
expect(() => {
  throw new Error("fallo");
}).toThrow();

expect(() => {
  return 42;
}).not.toThrow();
```

## 2.8 Snapshots

| Matcher | Descripción |
|---------|-------------|
| `.toMatchSnapshot()` | Crea o compara contra un snapshot en disco |

La primera ejecución guarda el valor serializado. Las ejecuciones posteriores fallan si el valor difiere. Usar `3va test --update-snapshots` para sobrescribir snapshots desactualizados.

```javascript
test("estructura del usuario", () => {
  const usuario = { id: 1, nombre: "Ana", activo: true };
  expect(usuario).toMatchSnapshot();
});
```

---

*Matchers implementados en `crates/test/src/matchers.rs`.*
