# 01 - ESPECIFICACIÓN DE TESTS

## 1.1 Test Runner

El test runner de 3va ejecuta pruebas escritas en JavaScript o TypeScript, inyectando una API global compatible con la convención Jest en cada archivo de test. Cada archivo se ejecuta en su propia instancia aislada del motor JS con permisos de lectura y escritura limitados al directorio del archivo.

## 1.2 Uso

```bash
# Descubrir y ejecutar todos los tests en el directorio actual
3va test

# Ejecutar tests en rutas específicas
3va test tests/ src/lib/

# Modo watch: re-ejecuta al detectar cambios
3va test --watch

# Reporte de cobertura de líneas y ramas
3va test --coverage

# Actualizar snapshots existentes en disco
3va test --update-snapshots
```

No existe soporte para `--bail`, `--test-name-pattern` ni ningún archivo de configuración `jest.config.js`.

## 1.3 Descubrimiento de Archivos

El runner busca recursivamente archivos con los siguientes patrones de nombre:

| Patrón | Descripción |
|--------|-------------|
| `*.test.js` | Test JavaScript |
| `*.test.ts` | Test TypeScript |
| `*.spec.js` | Spec JavaScript |
| `*.spec.ts` | Spec TypeScript |

## 1.4 API Global Inyectada

Las siguientes funciones están disponibles como globales dentro de cada archivo de test; no es necesario importarlas.

```javascript
// Agrupa tests relacionados; es anidable
describe("nombre del suite", () => {
  // Registra un test individual
  test("nombre del test", () => {
    expect(1 + 1).toBe(2);
  });

  // Alias exacto de test
  it("también registra un test", () => {
    expect("hola").toHaveLength(4);
  });
});
```

## 1.5 expect y Matchers

`expect(valor)` crea una cadena de aserciones. Todos los matchers admiten negación mediante `.not`:

```javascript
expect(valor).toBe(esperado);
expect(valor).not.toBe(noEsperado);
```

La lista completa de matchers implementados se documenta en `02-matchers.md`.

## 1.6 Snapshots

La primera vez que un test llama a `.toMatchSnapshot()`, el valor serializado se guarda en disco. Las ejecuciones posteriores comparan contra ese valor guardado.

- Ubicación del archivo: `__snapshots__/<nombre-del-test>.snap` junto al archivo de test.
- Formato: JSON plano con la estructura `{ "nombre del test": <valor serializado>, ... }`.
- Para actualizar snapshots desactualizados: `3va test --update-snapshots`.

```javascript
test("el objeto tiene la forma esperada", () => {
  const resultado = { id: 1, activo: true };
  expect(resultado).toMatchSnapshot();
});
```

## 1.7 Comportamiento del Runner

- Cada archivo de test se ejecuta en su propia instancia de `JsEngine` con `PermissionState` aislado.
- Se conceden permisos de `FileRead` y `FileWrite` al directorio del archivo (necesario para leer y escribir snapshots).
- La salida reporta `PASS` / `FAIL`, el nombre del suite, el mensaje de aserción y el tiempo transcurrido.
- Los errores de sintaxis en el archivo de test se capturan y se reportan como un test fallido.
- `run_directory(path)` descubre y ejecuta recursivamente todos los archivos de test en un directorio.

## 1.8 Cobertura

El flag `--coverage` genera un reporte de cobertura de **líneas** y **ramas**. Ver `03-coverage.md` para detalles.

---

*Implementado en `crates/test/src/` (`runner.rs`, `framework.rs`, `matchers.rs`, `coverage.rs`).*
