# 03 - POLYFILLS Y SHIMS

## 3.1 Sistema de Polyfills

3va implementa polyfills para APIs de Node.js y navegador que no están disponibles nativamente en QuickJS.

## 3.2 Polyfills de Node.js

### 3.2.1 Módulos Integrados

| Módulo | Tipo | Descripción |
|--------|------|-------------|
| buffer | Built-in | Buffer de datos binarios |
| console | Built-in | Consola (con auditoría) |
| crypto | Built-in | Criptografía básica |
| events | Built-in | EventEmitter |
| fs | Built-in (parcial) | Sistema de archivos |
| http | Built-in (parcial) | HTTP cliente/servidor |
| os | Built-in | Información del sistema |
| path | Built-in | Utilidades de rutas |
| process | Built-in | Proceso actual |
| stream | Built-in (parcial) | Streams |
| url | Built-in | Parseo de URLs |
| util | Built-in | Utilidades |

### 3.2.2 Buffer

```javascript
// Polyfill de Buffer disponible globalmente
const buf = Buffer.from('Hello World');
const buf = Buffer.alloc(8);
const buf = Buffer.allocUnsafe(8);

// Encodings soportados
buf.toString('utf8')
buf.toString('base64')
buf.toString('hex')
buf.toString('latin1')

// Métodos
Buffer.isBuffer(obj)
Buffer.byteLength(string, encoding)
Buffer.concat(buffers)
```

### 3.2.3 crypto

```javascript
const crypto = require('crypto');

// Hash
const hash = crypto.createHash('sha256');
hash.update('data');
console.log(hash.digest('hex'));

// HMAC
const hmac = crypto.createHmac('sha256', 'key');
hmac.update('data');
console.log(hmac.digest('hex'));

// Random
const bytes = crypto.randomBytes(16);

// Utilities
crypto.createCipheriv()
crypto.createDecipheriv()
crypto.sign()
crypto.verify()
```

### 3.2.4 Events

```javascript
const EventEmitter = require('events');

class MyEmitter extends EventEmitter {}

const emitter = new MyEmitter();

emitter.on('event', (arg) => {
    console.log('event:', arg);
});

emitter.emit('event', { data: 'value' });

// Métodos
emitter.on(event, listener)
emitter.once(event, listener)
emitter.off(event, listener)
emitter.emit(event, ...args)
emitter.removeAllListeners(event)
```

## 3.3 Polyfills de Web APIs

### 3.3.1 fetch

```javascript
// fetch disponible globalmente
const response = await fetch('https://api.example.com/data');
const json = await response.json();

// Request
const req = new Request('/api', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: 'value' })
});

// Response
new Response(JSON.stringify(data), {
    status: 200,
    headers: { 'Content-Type': 'application/json' }
});

// AbortController
const controller = new AbortController();
fetch('/api', { signal: controller.signal });
controller.abort();
```

### 3.3.2 TextEncoder/TextDecoder

```javascript
// TextEncoder
const encoder = new TextEncoder();
const encoded = encoder.encode('Hello');  // Uint8Array

// TextDecoder
const decoder = new TextDecoder('utf-8');
const decoded = decoder.decode(encoded);  // "Hello"

// Con stream
const transform = encoder.encodeStreaming(data);
```

### 3.3.3 URL y URLSearchParams

```javascript
// URL
const url = new URL('https://user:pass@example.com:8080/path?query=1#hash');

console.log(url.protocol);   // "https:"
console.log(url.host);       // "example.com:8080"
console.log(url.pathname);   // "/path"
console.log(url.search);     // "?query=1"
console.log(url.hash);       // "#hash"
console.log(url.username);   // "user"
console.log(url.password);   // "pass"

// URLSearchParams
const params = new URLSearchParams('foo=bar&baz=qux');
params.get('foo');           // "bar"
params.set('foo', 'new');
params.append('extra', 'value');
params.delete('baz');
```

### 3.3.4 Performance

```javascript
// Performance
const start = performance.now();

// ... código ...

const end = performance.now();
console.log(`Tiempo: ${end - start}ms`);

// Marcas y medidas
performance.mark('start');
// ... código ...
performance.mark('end');
performance.measure('total', 'start', 'end');

// Navigation timing (parcial)
performance.timing;
performance.navigation;
```

## 3.4 Polyfills de Seguridad

### 3.4.1 Fetch Seguro

```rust
// El fetch de 3va implementa:
// 1. Verificación de permisos
// 2. Validación de URL
// 3. Sanitización de headers
// 4. Límites de tamaño de respuesta

pub struct SecureFetch {
    permissions: PermissionState,
    max_response_size: usize,
}

impl SecureFetch {
    pub async fn fetch(&self, url: &str, init: RequestInit) -> Result<Response> {
        // Verificar permiso de red
        let url_parsed = Url::parse(url)?;
        if !self.permissions.check(&Capability::Network(url_parsed.host_str().unwrap_or(""))) {
            return Err(Error::PermissionDenied);
        }

        // Verificar URL no es maligna
        if url_parsed.username().is_some() && url_parsed.password().is_some() {
            return Err(Error::SecurityError("URL con credenciales embebidas"));
        }

        // Fetch con límites
        let response = self.raw_fetch(url, init).await?;

        // Verificar tamaño
        if let Some(len) = response.content_length() {
            if len > self.max_response_size {
                return Err(Error::ResponseTooLarge);
            }
        }

        Ok(response)
    }
}
```

## 3.5 Registro de Polyfills

### 3.5.1 PolyfillRegistry

```rust
pub struct PolyfillRegistry {
    builtins: HashMap<String, Polyfill>,
    globals: HashMap<String, Value>,
}

impl PolyfillRegistry {
    pub fn init(context: &Context) -> anyhow::Result<()> {
        // 1. Cargar módulos built-in en el contexto
        // 2. Definir globals
        // 3. Configurar resolveHook

        Ok(())
    }

    pub fn register_builtin(&mut self, name: &str, module: Polyfill) {
        self.builtins.insert(name.to_string(), module);
    }
}
```

### 3.5.2 Configuración de Polyfills

```javascript
// En JavaScript: deshabilitar polyfills específicos
3va run app.ts --no-polyfill=fetch
3va run app.ts --no-polyfill=stream
```

---

*Polyfills conformes a Node.js API y WHATWG standards.*