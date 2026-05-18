# 04 - FUZZING INTEGRADO

## 4.1 Fuzz Testing

El fuzzing genera datos aleatorios para encontrar vulnerabilidades y bugs.

## 4.2 Tipos de Fuzzing

| Tipo | Descripcion |
|------|-------------|
| Dumb fuzzing | Entradas aleatorias |
| Guided fuzzing |Mutation basada en coverage |
| Semantic fuzzing | Entradas con estructura |

## 4.3 Integración

```javascript
// test/fuzz.test.js
import { fuzz, fuzzFunctions } from "3va/fuzz";

fuzz(myFunction, {
  maxIterations: 10000,
  timeout: 60000,
  dataTypes: ["string", "number", "array"]
});
```

## 4.4 Usage

```bash
# Fuzz function
3va fuzz test/fuzz.js

# Fuzz con coverage
3va fuzz --coverage

# Fuzz conmutidad
3va fuzz --parallel
```

## 4.5 Coverage

| Métrica | Descripcion |
|---------|-------------|
| Line coverage | Porcentaje de líneas ejecutadas |
| Branch coverage | Porcentaje de ramas |
| Function coverage | Funciones llamadas |

## 4.6 Mutators

| Mutator | Descripcion |
|---------|-------------|
| byteFlip | Flip bits aleatorios |
| arithmetic | Modificar números |
| empty | Input vacío |
| unicode | Caracteres Unicode |
| trim | Spaces extra |

---

*Fuzzing basado en AFL y libFuzzer.*