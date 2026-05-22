# 04 - COMPATIBLE WEB APIs

## 4.1 Overview

3va implements standard web APIs compatible with modern browsers and the WHATWG standard.

## 4.2 APIs Implemented by Status

### 4.2.1 Complete APIs

| API | Specification | Status |
|-----|---------------|--------|
| AbortController | WHATWG | Implemented |
| AbortSignal | WHATWG | Implemented |
| BroadcastChannel | WHATWG | Partial |
| Crypto | Web Crypto API | Implemented |
| CryptoKey | Web Crypto API | Implemented |
| EventTarget | WHATWG | Implemented |
| Event | WHATWG | Implemented |
| Headers | WHATWG | Implemented |
| Request | WHATWG | Implemented |
| Response | WHATWG | Implemented |
| URL | WHATWG | Implemented |
| URLSearchParams | WHATWG | Implemented |
| TextEncoder | Encoding | Implemented |
| TextDecoder | Encoding | Implemented |
| Performance | Performance API | Implemented |
| PerformanceEntry | Performance API | Implemented |
| PerformanceMark | Performance API | Implemented |
| PerformanceMeasure | Performance API | Implemented |
| FormData | WHATWG | Implemented |
| Blob | File API | Implemented |
| File | File API | Implemented |

### 4.2.2 Partial/To Be Implemented APIs

| API | Status | Notes |
|-----|--------|-------|
| ReadableStream | Partial | Basic only |
| WritableStream | Partial | Basic only |
| TransformStream | To implement | Roadmap |
| WebSocket | To implement | Roadmap |
| Worker | To implement | Roadmap |
| MessageChannel | Implemented | Basic |
| SharedArrayBuffer | To implement | Requires headers |

## 4.3 fetch API

### 4.3.1 Basic Usage

```javascript
// Simple GET
const response = await fetch('https://api.example.com/data');
const data = await response.json();

// POST with body
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

// Response handling
if (response.ok) {
    const data = await response.json();
} else {
    console.error('Error:', response.status);
}
```

### 4.3.2 Request

```javascript
// Request constructor
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

// Methods
request.url
request.method
request.headers
request.body
request.clone()
```

### 4.3.3 Response

```javascript
// Create Response
const response = new Response(JSON.stringify(data), {
    status: 200,
    statusText: 'OK',
    headers: new Headers({ 'Content-Type': 'application/json' })
});

// Properties
response.ok
response.status
response.statusText
response.headers
response.url
response.type  // 'basic', 'cors', 'opaque', 'error'

// Methods
response.text()
response.json()
response.blob()
response.formData()
response.clone()
response.redirect(url, status)
```

## 4.4 Headers

### 4.4.1 Headers Usage

```javascript
// Create
const headers = new Headers({
    'Content-Type': 'application/json',
    'Authorization': 'Bearer token'
});

// Read
headers.get('content-type')
headers.get('authorization')

// Write
headers.set('Content-Type', 'text/plain')
headers.append('Accept', 'application/json')
headers.delete('Authorization')

// Iterate
for (const [key, value] of headers) {
    console.log(`${key}: ${value}`);
}

// Check
headers.has('Content-Type')
```

## 4.5 Streams API

### 4.5.1 ReadableStream (Partial)

```javascript
// Get stream from response
const response = await fetch('https://api.example.com/data');
const reader = response.body.getReader();

// Read chunks
while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    console.log('Chunk:', value);
}

// Cancel
reader.cancel();

// Properties
reader.closed
reader.desiredSize
```

### 4.5.2 WritableStream (Partial)

```javascript
// Create writable stream
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

// Write
const writer = stream.getWriter();
await writer.write('Hello');
await writer.close();
```

### 4.5.3 TransformStream (Not available)

```javascript
// Coming soon:
// const transform = new TransformStream(transformer);
```

## 4.6 WebSocket (To be implemented)

```javascript
// Roadmap: Implementation Planned
// ws = new WebSocket('ws://example.com/socket');

// Events
// ws.onopen = () => { };
// ws.onmessage = (event) => { };
// ws.onerror = (error) => { };
// ws.onclose = (event) => { };

// Methods
// ws.send(data);
// ws.close(code, reason);
```

## 4.7 Performance API

### 4.7.1 Measurement

```javascript
// Marks
performance.mark('startOperation');
// ... operation ...
performance.mark('endOperation');

// Measures
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
// Navigation information
const navigation = performance.getEntriesByType('navigation')[0];

console.log(navigation.domainLookupEnd - navigation.domainLookupStart);  // DNS
console.log(navigation.connectEnd - navigation.connectStart);              // TCP
console.log(navigation.responseStart - navigation.requestStart);            // TTFB
console.log(navigation.loadEventEnd - navigation.navigationStart);           // Load
```

## 4.8 Blob and File

### 4.8.1 Blob

```javascript
// Create blob
const blob = new Blob(['Hello World'], { type: 'text/plain' });

// Properties
blob.size
blob.type

// Methods
blob.slice(start, end, contentType)
blob.stream()
blob.text()
blob.arrayBuffer()
```

### 4.8.2 File

```javascript
// Create file (typically from input)
const file = new File(['content'], 'example.txt', {
    type: 'text/plain',
    lastModified: Date.now()
});

// Properties
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

*Web APIs conforming to WHATWG and W3C standards.*
