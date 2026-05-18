# 04 - MODO WATCH

## 4.1 Watch Mode

El modo watch ejecuta tests automáticamente cuando cambian archivos.

## 4.2 Uso

```bash
# Activar watch
3va test --watch

# Watch con coverage
3va test --watch --coverage
```

## 4.3 Controles Interactivos

| Tecla | Accion |
|-------|--------|
| a | Ejecutar todos los tests |
| f | Solo tests fallidos |
| p | Filtrar por path |
| t | Filtrar por nombre |
| q | Salir |

## 4.4 Archivos Monitoreados

| Patron | Descripcion |
|--------|-------------|
| *.test.js | Archivos de test |
| *.spec.js | Specs |
| src/**/* | Archivos fuente |

## 4.5 Configuración

```javascript
module.exports = {
  watchPathIgnorePatterns: [
    "node_modules",
    "\\.git"
  ],
  watchman: true
};
```

---

*Watch mode conforme a Jest watch.*