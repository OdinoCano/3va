# 04 - MODO WATCH

## 4.1 Descripción

El modo watch observa el sistema de archivos y re-ejecuta automáticamente todos los tests cuando detecta cambios en archivos de código fuente. No requiere ninguna interacción del usuario durante la ejecución.

## 4.2 Uso

```bash
# Activar watch mode
3va test --watch

# Watch con reporte de cobertura en cada re-ejecución
3va test --watch --coverage
```

## 4.3 Comportamiento

- Al iniciar, ejecuta todos los tests descubiertos una primera vez.
- Monitorea cambios en archivos con extensión `.js`, `.ts`, `.jsx` y `.tsx`.
- Aplica un **debounce de 500 ms**: si se detectan múltiples cambios en ráfaga, espera a que la actividad se detenga antes de lanzar la siguiente ejecución.
- Al detectar un cambio, re-ejecuta la suite completa desde cero.
- El proceso se mantiene en ejecución hasta que se interrumpe con `Ctrl+C`.

## 4.4 Archivos Monitoreados

| Extensión | Descripción |
|-----------|-------------|
| `.js` | JavaScript |
| `.ts` | TypeScript |
| `.jsx` | JavaScript + JSX |
| `.tsx` | TypeScript + JSX |

## 4.5 Limitaciones

- No existen controles de teclado interactivos. Las teclas `a`, `f`, `p`, `t` y `q` que aparecen en la interfaz de Jest **no están implementadas**.
- No es posible filtrar tests por nombre o ruta desde el modo watch.
- No hay integración con Watchman; el watcher usa la crate `notify` de Rust directamente.
- No se leen patrones de exclusión desde archivos de configuración.

---

*Watch mode implementado en `crates/test/src/runner.rs` usando la crate `notify`.*
