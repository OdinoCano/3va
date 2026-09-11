# 11 — React Native

Ejecuta y testea la **lógica compartida** de tu app React Native con 3va.

React Native renderiza en un bridge nativo (no ejecutable por 3va), pero el
grueso del código de una app RN — reducers, selectores, validaciones,
formateo, lógica de negocio, API client — es TypeScript puro que SÍ corre en
3va. Ese mismo módulo se comparte entre la app y el backend sin tocar nada.

## Estructura

```
src/
  todos.ts      reducer + selectores de una todo list (TS puro)
  validate.ts   validaciones de formulario (compartidas con el backend)
  index.ts      demo ejecutable
  todos.test.ts tests Jest-compatibles
```

## Ejecutar la demo

```bash
3va run src/index.ts --allow-read=./src
```

## Testear la lógica

```bash
3va test
```

## En una app React Native real

```bash
# 1. Coopera con el ciclo normal de RN:
#    npx react-native run-android | run-ios   (el puente nativo)
3va install                                     # 2. instala tus dependencias pnpm-style
3va run scripts/generate-types.ts               # 3. código auxiliar (codegen, configs) en 3va
3va test                                        # 4. la suite Jest de tu lógica en 3va
```

## Qué demuestra

- 3va transpila `.ts` sobre la marcha y corre TypeScript puro sin config
- El test runner de 3va es Jest-compatible: `describe`/`test`/`expect`
- La lógica RN (reducer, selectores, validación) es 100 % comprobable sin emulador
- Los mismos módulos sirven para el backend — una sola lógica, dos runtimes