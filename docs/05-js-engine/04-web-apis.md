# 04 - APIs WEB COMPATIBLES

## 4.1 Visión General

3va implementa APIs web estándar compatibles con navegadores modernos y el estándar WHATWG.

## 4.2 APIs Implementadas por Estado

### 4.2.1 APIs Completas

| API | Especificación | Estado |
|-----|----------------|--------|
| AbortController | WHATWG | Implementado |
| AbortSignal | WHATWG | Implementado |
| BroadcastChannel | WHATWG | Parcial |
| Crypto | Web Crypto API | Implementado |
| CryptoKey | Web Crypto API | Implementado |
| EventTarget | WHATWG | Implementado |
| Event | WHATWG | Implementado |
| Headers | WHATWG | Implementado |
| Request | WHATWG | Implementado |
| Response | WHATWG | Implementado |
| URL | WHATWG | Implementado |
| URLSearchParams | WHATWG | Implementado |
| TextEncoder | Encoding | Implementado |
| TextDecoder | Encoding | Implementado |
| Performance | Performance API | Implementado |
| PerformanceEntry | Performance API | Implementado |
| PerformanceMark | Performance API | Implementado |
| PerformanceMeasure | Performance API | Implementado |
| FormData | WHATWG | Implementado |
| Blob | File API | Implementado |
| File | File API | Implementado |

### 4.2.2 APIs Parciales/Por Implementar

| API | Estado | Notas |
|-----|--------|-------|
| ReadableStream | Parcial | Solo básica |
| WritableStream | Parcial | Solo básica |
| TransformStream | Por implementar | Roadmap |
| WebSocket | Por implementar | Roadmap |
| Worker | Por implementar | Roadmap |
| MessageChannel | Implementado | Basic |
| SharedArrayBuffer | Por implementar | Requiere headers |

## 4.3 fetch API

### 4.3.1 Uso Básico

```javascript
// GET simple
const response = await fetch('https://api.example.com/data');
const data = await response.json();

// POST con body
const response = await fetch('https://api.example.com/submit', {
    method: 'POST',
    headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer token123'
    },
    body: JSON.stringify({ key: 'value' }),
    // Credentials
    credentials: 'same-origin',  // 'omit', 'same-origin', 'include'
    // Mode
    mode: 'cors',  // 'no-cors', 'cors', 'same-origin'
});

// Manejo de respuesta
if (response.ok) {
    const data = await response.json();
} else {
    console.error('Error:', response.status);
}
```

### 4.3.2 Request

```javascript
// Constructor de Request
const request = new Request('/api/data', {
    method: 'GET',
    headers: new Headers({
        'Content-Type': 'application/json'
    }),
    body: JSON.stringify({ data: 'test' }),
    cache: 'default',
    credentials: 'same-origin',
    mode: 'cors',
    redirect: 'follow',
    referrer: 'no-referrer',
});

// Métodos
request.url
request.method
request.headers
request.body
request.clone()
```

### 4.3.3 Response

```javascript
// Crear Response
const response = new Response(JSON.stringify(data), {
    status: 200,
    statusText: 'OK',
    headers: new Headers({ 'Content-Type': 'application/json' })
});

// Propiedades
response.ok
response.status
response.statusText
response.headers
response.url
response.type  // 'basic', 'cors', 'opaque', 'error'

// Métodos
response.text()
response.json()
response.blob()
response.formData()
response.clone()
response.redirect(url, status)
```

## 4.4 Headers

### 4.4.1 Uso de Headers

```javascript
// Crear
const headers = new Headers({
    'Content-Type': 'application/json',
    'Authorization': 'Bearer token'
});

// Leer
headers.get('content-type')
headers.get('authorization')

// Escribir
headers.set('Content-Type', 'text/plain')
headers.append('Accept', 'application/json')
headers.delete('Authorization')

// Iterar
for (const [key, value] of headers) {
    console.log(`${key}: ${value}`);
}

// Verificar
headers.has('Content-Type')
```

## 4.5 Streams API

### 4.5.1 ReadableStream (Parcial)

```javascript
// Obtener stream de response
const response = await fetch('https://api.example.com/data');
const reader = response.body.getReader();

// Leer chunks
while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    console.log('Chunk:', value);
}

// Cancelar
reader.cancel();

// Propiedades
reader.closed
reader.desiredSize
```

### 4.5.2 WritableStream (Parcial)

```javascript
// Crear writable stream
const stream = new WritableStream({
    start(controller) {
        console.log('Stream started');
    },
    write(chunk, controller) {
        console.log('Writing:', chunk);
    },
    close() {
        console.log('Stream closed');
    },
    abort(reason) {
        console.error('Stream aborted:', reason);
    }
});

// Escribir
const writer = stream.getWriter();
await writer.write('Hello');
await writer.close();
```

### 4.5.3 TransformStream (No disponible)

```javascript
// Próximamente:
// const transform = new TransformStream(transformer);
```

## 4.6 WebSocket (Por implementar)

```javascript
// Roadmap: Implementación Planned
// ws = new WebSocket('ws://example.com/socket');

// Eventos
// ws.onopen = () => { };
// ws.onmessage = (event) => { };
// ws.onerror = (error) => { };
// ws.onclose = (event) => { };

// Métodos
// ws.send(data);
// ws.close(code, reason);
```

## 4.7 Performance API

### 4.7.1 Medición

```javascript
// Marcas
performance.mark('startOperation');
// ... operación ...
performance.mark('endOperation');

// Medidas
performance.measure('operation', 'startOperation', 'endOperation');

// Entries
const entries = performance.getEntries();
const entriesByType = performance.getEntriesByType('measure');
const entriesByName = performance.getEntriesByName('operation');

// Clear
performance.clearMarks();
performance.clearMeasures();
performance.clearResources();
```

### 4.7.2 Navigation Timing

```javascript
// Información de navegación
const navigation = performance.getEntriesByType('navigation')[0];

console.log(navigation.domainLookupEnd - navigation.domainLookupStart);  // DNS
console.log(navigation.connectEnd - navigation.connectStart);              // TCP
console.log(navigation.responseStart - navigation.requestStart);            // TTFB
console.log(navigation.loadEventEnd - navigation.navigationStart);           // Load
```

## 4.8 Blob y File

### 4.8.1 Blob

```javascript
// Crear blob
const blob = new Blob(['Hello World'], { type: 'text/plain' });

// Propiedades
blob.size
blob.type

// Métodos
blob.slice(start, end, contentType)
blob.stream()
blob.text()
blob.arrayBuffer()
```

### 4.8.2 File

```javascript
// Crear file (generalmente de input)
const file = new File(['content'], 'example.txt', {
    type: 'text/plain',
    lastModified: Date.now()
});

// Propiedades
file.name
file.size
file.type
file.lastModified
```

### 4.8.3 FileReader

```javascript
const reader = new FileReader();

reader.onload = (event) => {
    console.log('Content:', event.target.result);
};

reader.onerror = (error) => {
    console.error('Error:', error);
};

reader.readAsText(file);
reader.readAsDataURL(file);
reader.readAsArrayBuffer(file);
```

---

*APIs web conformes a WHATWG y estándares W3C.*