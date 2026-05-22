# 04 - CODE SPLITTING

## 4.1 Code Division

Code splitting divides the bundle into multiple chunks that load on demand.

## 4.2 Strategies

| Strategy | Description |
|------------|-------------|
| entry | One chunk per entry point |
| async | Chunks for dynamic imports |
| manual | Manual division with comments |

## 4.3 Dynamic Imports

```javascript
// A separate chunk is created
const module = await import("./heavy-module.js");

// In build
// main.js
// heavy-module.js (lazy loaded)
```

## 4.4 Configuration

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

*Code splitting conforming to webpack and esbuild.*
