# 13 — Tauri

3va como toolchain de **frontend** para una app Tauri v2.

Tauri compila el shell de escritorio en Rust (`cargo tauri build`), pero el
frontend es TypeScript y el JS toolchain vive fuera. 3va cubre ese lado:
dev server Vite-style, bundler e test runner — lo que en un proyecto Tauri
normal haría Vite + jest, sin instalar nada más.

## Cómo encaja en un proyecto Tauri v2

```
mi-app/
├─ src-tauri/          ← Rust (shell, #[tauri::command])
│   └─ tauri.conf.json ← beforeDevCommand / beforeBuildCommand apuntan a 3va
├─ index.html
└─ src/
    └─ main.ts         ← frontend TS que 3va sirve y bundlea
```

En `src-tauri/tauri.conf.json`:

```json
{
  "build": {
    "beforeDevCommand": "3va dev --port 3000",
    "beforeBuildCommand": "3va bundle src/main.ts -o dist/bundle.js --minify",
    "devUrl": "http://localhost:3000",
    "frontendDist": "../dist"
  }
}
```

`cargo tauri dev` arranca 3va, apunta la webview al dev server, y HMR se encarga
del resto.

## Probar los comandos de estado sin Rust

El frontend despacha comandos tipados por IPC (`#[tauri::command]`). Aquí los
mismos tipos corren en TS puro, así 3va los testea sin compilar Rust:

```bash
3va test
```

## Sirve el frontend y bundlea

```bash
3va dev --port 3000     # transpila ./src/*.ts por request, HMR
3va bundle src/main.ts -o dist/bundle.js --minify   # bundle de producción
```

## Ejecutar la demo de servidor

```bash
3va dev --port 3000 --no-csp
# abre http://localhost:3000 — slider de volumen + toggle de tema
```

## Qué demuestra

- 3va sirve un frontend TS literalmente Vite-style (ESM on-demand, `window`,
  DOM APIs, HMR) — el mismo contrato que el `beforeDevCommand` de Tauri
- `3va bundle` produce el `dist/bundle.js` que el shell Tauri cargará
- `3va test` cubre la lógica de comandos sin emular IPC ni compilar Rust
- Toolchain completo (dev/server/bundle/test) en un solo binario