# 12 — Expo

Utilidades de Expo ejecutadas y testeadas con 3va, sin necesidad de Metro.

3va no compila una app Expo completa (ni Expo Router, ni el bridge nativo), pero
hace tres cosas muy útiles **sin emulador**:

1. Ejecuta y testea helpers de theme, texto, fecha, API client — el TS puro.
2. Corre scripts auxiliares (validación de `app.json`, codegen, migration).
3. Servir el frontend Expo-web con `3va dev` si tu app exporta un bundle web.

## Generar una app Expo real con 3va

```bash
3va create expo-app mi-app          # usa create-expo-app bajo el capó
cd mi-app && 3va dev                # 3va sirve el frontend web (Vite-style)
```

> Esto es equivalente a `npx create-expo-app mi-app` pero intégra el runtime,
> test runner, bundler y process manager en un solo binario.

## Ejecutar esta demo local

```bash
3va run src/index.ts --allow-read=./src
3va run src/validate-app.ts --allow-read=./app.json
3va test
```

## Qué demuestra

- Tokens de tema (colores oscuro/claro, spacing, font size) — mismos que
  una app real RN comparte entre la app y tools.
- Helpers de texto: `truncate`, `pluralize`, `formatCount`, `slug`.
- Formateo de fecha relativa para UI.
- Script de validación de `app.json` — corre sin Metro, sin emulador.
- Todos los tests pasan con el runner Jest-compat内置 de 3va.