# 03 - OBJETOS GLOBALES
3.1 Globals del Entorno JavaScript
3va expone un conjunto de objetos globales compatible con Node.js y navegadores, siguiendo la especificación ECMAScript y APIs web estándar.
3.2 Globals Estándar ECMAScript
3.2.1 Objetos Fundamentales
Global	Descripción
Object	Constructor de objetos
Function	Constructor de funciones
Array	Constructor de arrays
Boolean	Constructor booleano
Number	Constructor numérico
BigInt	Enteros grandes
String	Constructor de strings
Symbol	Símbolos únicos
Date	Fechas
RegExp	Expresiones regulares
Error	Clase de errores
Map	Colección clave-valor
Set	Colección de valores únicos
WeakMap	Map debil
WeakSet	Set debil
ArrayBuffer	Buffer binario
Promise	Promesas
Proxy	Metaprogramación
Reflect	Reflexión
3.2.2 Funciones Globales
Función
eval()
isFinite()
isNaN()
parseFloat()
parseInt()
decodeURI()
encodeURI()
decodeURIComponent()
encodeURIComponent()
3.2.3 Constructores de Tipos
// Typed Arrays
Int8Array, Uint8Array, Uint8ClampedArray
Int16Array, Uint16Array
Int32Array, Uint32Array
Float32Array, Float64Array
BigInt64Array, BigUint64Array
// Estructured Clone
SharedArrayBuffer
Atomics
3.3 Globals de Node.js
3.3.1 Objetos Node
// Proceso
process          // global process object
global           // global namespace
// Console
console          // console object
// Timers (funciones)
setTimeout       // ejecutar después de delay
setInterval      // ejecutar repetidamente
setImmediate     // ejecutar en siguiente fase
clearTimeout     // cancelar timeout
clearInterval    // cancelar intervalo
clearImmediate   // cancelar inmediato
// Módulos
module           // current module
exports          // module exports
require          // require function
__dirname        // directory of current module
__filename       // filename of current module
3.3.2 Buffer Global
// Buffer global
Buffer           // Clase Buffer disponible globalmente
// Crear buffer
Buffer.from('hello')      // desde string
Buffer.alloc(8)            // allocation
Buffer.allocUnsafe(8)      // sin inicializar
3.3.3 URL y Utils
// URL
URL              // Constructor de URLs
URLSearchParams // Parámetros de query
// Utilidades
setTimeout.setTimeout
setTimeout.clearTimeout
3.4 APIs Web Compatibles
4.4.1 fetch API
// fetch polyfill (implementado en QuickJS)
const response = await fetch('https://api.example.com/data');
const data = await response.json();
// Opciones
await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: 'value' }),
});
4.4.2 Web APIs
API	Descripción
AbortController	Control de abortos
AbortSignal	Señal de abortado
BroadcastChannel	Comunicación entre tabs
Crypto	Operaciones criptográficas
CryptoKey	Claves criptográficas
Performance	Medición de rendimiento
PerformanceEntry	Entrada de rendimiento
PerformanceMark	Marca de rendimiento
PerformanceMeasure	Medida de rendimiento
TextEncoder	Codificador de texto
TextDecoder	Decodificador de texto
TransformStream	Streams de transformación
ReadableStream	Streams legibles
WritableStream	Streams escribibles
Headers	Cabeceras HTTP
Request	Solicitud HTTP
Response	Respuesta HTTP
FormData	Datos de formulario
URLSearchParams	Parámetros URL
WebSocket	WebSockets
3.5 Polyfills de Seguridad
3.5.1 Fetch con Verificación
// El fetch de 3va incluye verificación de permisos
pub async fn secure_fetch(url: &str, options: RequestInit) -> Result<Response> {
    // 1. Verificar permiso de red
    if !permissions.check(&Capability::Network(url)) {
        return Err(Error::PermissionDenied);
    }
    // 2. Validar URL
    let parsed = Url::parse(url)?;
    validate_no_malicious_redirect(&parsed)?;
    // 3. Ejecutar fetch
    let response = native_fetch(url, options).await?;
    // 4. Verificar respuesta
    validate_response(&response)?;
    Ok(response)
}
3.5.2 Console con Logging de Auditoría
// Console escribe a log de auditoría
pub fn log(&self, level: Level, args: Vec<Value>) {
    // Output normal
    self.output.write(args);
    // Log de auditoría
    audit::log(AuditEvent {
        event_type: "console".to_string(),
        level: level.to_string(),
        timestamp: now(),
        data: args.clone(),
    });
}
Globals conforme a Node.js API y WHATWG standards.