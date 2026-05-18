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

## 1.6 Cumplimiento Normativo (IEEE 829)

El *Test Runner* ha sido implementado proveyendo trazabilidad y salida de reportes estándar según **IEEE 829** (Estándar de Documentación de Pruebas de Software).
La arquitectura captura fallos (Failures), aciertos (Passes) y métricas de desempeño de forma rigurosa, facilitando la validación del ciclo de vida del software de cualquier proyecto que corra sobre la máquina.

---

*Test runner conforme a Jest API e implementado en `crates/test/src` (`runner.rs`, `framework.rs`, `matchers.rs`).*