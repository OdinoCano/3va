# 02 - MATCHERS Y ASERCIONES

## 2.1 Matchers

Los matchers permiten verificar valores en tests.

## 2.2 Matchers Comunes

| Matcher | Descripcion |
|---------|-------------|
| toBe(value) | Comparación exacta |
| toEqual(value) | Comparación profunda |
| toBeNull() | Es null |
| toBeUndefined() | Es undefined |
| toBeTruthy() | Es truthy |
| toBeFalsy() | Es falsy |
| toContain(item) | Contiene elemento |
| toThrow() | Lanza error |
| toHaveLength(n) | Tiene longitud n |

## 2.3 Matchers Numéricos

| Matcher | Descripcion |
|---------|-------------|
| toBeGreaterThan(n) | Mayor que n |
| toBeLessThan(n) | Menor que n |
| toBeCloseTo(n, digits) | Cercano a n |

## 2.4 Matchers de Seguridad

| Matcher | Descripcion |
|---------|-------------|
| toBeSafeHtml() | No contiene XSS |
| toNotHaveSecrets() | No contiene API keys |
| toNotHaveInjection() | No tiene SQL injection |

## 2.5 Ejemplos

```javascript
expect(5).toBe(5);
expect({a:1}).toEqual({a:1});
expect([1,2,3]).toContain(2);
expect(() => x()).toThrow();
expect(html).toBeSafeHtml();
```

---

*Matchers conforme a Jest matchers.*