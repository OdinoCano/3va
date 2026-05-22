# 04 - INTEGRATED FUZZING

## 4.1 Fuzz Testing

Fuzzing generates random data to find vulnerabilities and bugs.

## 4.2 Fuzzing Types

| Type | Description |
|------|-------------|
| Dumb fuzzing | Random inputs |
| Guided fuzzing | Coverage-based mutation |
| Semantic fuzzing | Structured inputs |

## 4.3 Integration

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

# Fuzz with coverage
3va fuzz --coverage

# Fuzz with concurrency
3va fuzz --parallel
```

## 4.5 Coverage

| Metric | Description |
|---------|-------------|
| Line coverage | Percentage of lines executed |
| Branch coverage | Percentage of branches |
| Function coverage | Functions called |

## 4.6 Mutators

| Mutator | Description |
|---------|-------------|
| byteFlip | Random bit flips |
| arithmetic | Modify numbers |
| empty | Empty input |
| unicode | Unicode characters |
| trim | Extra spaces |

---

*Fuzzing based on AFL and libFuzzer.*
