# 03 - TREE SHAKING

## 3.1 Eliminación de Código Muerto

Tree shaking elimina código no utilizado del bundle final.

## 3.2 Algoritmo

### 3.2.1 Análisis de Exportaciones

```
1. Parsear todos los módulos
2. Construir grafo de dependencias
3. Identificar exports utilizados
4. Marcar imports como "usados"
5. Recursivamente marcar dependencias
6. Eliminar código no marcado
```

### 3.2.2 Efectos Secundarios

```javascript
// Este código tiene efectos secundarios
// no puede ser eliminado
import { something } from "module";

// Marcar como no eliminable
/* @__PURE__ */ something();
```

## 3.3 Configuración

| Opcion | Descripcion |
|--------|-------------|
| treeShaking: true | Habilitar por defecto |
| sideEffects: false | Todo es puro |
| sideEffects: ["*.css"] | Solo CSS tiene efectos |

## 3.4 Ejemplo

```javascript
// module.js
export function used() { return 1; }
export function unused() { return 2; }

// main.js
import { used } from "./module.js";
console.log(used());

// Output - unused() eliminado
console.log(1);
```

---

*Tree shaking conforme a Rollup spec.*