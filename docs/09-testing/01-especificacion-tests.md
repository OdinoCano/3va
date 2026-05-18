# 01 - ESPECIFICACIÓN DE TESTS

## 1.1 Test Runner

El test runner de 3va es compatible con Jest, con capacidades adicionales de seguridad.

## 1.2 Uso

```bash
# Ejecutar todos los tests
3va test

# Archivos específicos
3va test tests/

# Modo watch
3va test --watch

# Coverage
3va test --coverage

# Bail en primer fallo
3va test --bail

# Filtrar por nombre
3va test --test-name-pattern=auth
```

## 1.3 Archivos de Test

| Extension | Descripcion |
|-----------|-------------|
| .test.js | Test JavaScript |
| .test.ts | Test TypeScript |
| .spec.js | Spec JavaScript |
| .spec.ts | Spec TypeScript |
| .test.jsx | Test JSX |
| .test.tsx | Test TSX |

## 1.4 API

```javascript
// describe
describe("suite name", () => { ... });

// test / it
test("test name", () => { ... });
it("it name", () => { ... });

// expect
expect(value).toBe(expected);
expect(value).toEqual(expected);
```

## 1.5 Configuración

```javascript
// jest.config.js o package.json
module.exports = {
  testEnvironment: "node",
  testMatch: ["**/*.test.js"],
  collectCoverage: true,
  coverageDirectory: "coverage",
};
```

---

*Test runner conforme a Jest API.*