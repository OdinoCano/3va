# 04 - CODE SPLITTING

## 4.1 División de Código

Code splitting divide el bundle en múltiples chunks que se cargan bajo demanda.

## 4.2 Estrategias

| Estrategia | Descripcion |
|------------|-------------|
| entry | Un chunk por punto de entrada |
| async | Chunks para dynamic imports |
| manual | División manual con comentarios |

## 4.3 Dynamic Imports

```javascript
// Se crea un chunk separado
const module = await import("./heavy-module.js");

// En build
// main.js
// heavy-module.js (lazy loaded)
```

## 4.4 Configuración

```javascript
// 3va.config.js
module.exports = {
  splitting: true,
  chunks: {
    "vendor": ["react", "react-dom"],
    "utils": ["./utils/*.js"]
  }
};
```

## 4.5 Output

```
dist/
├── main.js
├── vendor.js
├── utils.js
└── chunk-abc123.js
```

---

*Code splitting conforme a webpack y esbuild.*