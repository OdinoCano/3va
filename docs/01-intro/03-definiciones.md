# 03 - DEFINICIONES Y ABREVIATURAS

## 3.1 Términos y Definiciones

### 3.1.1 Términos del Dominio

**Capability (Capacidad)**: Un permiso explícito concedido a un proceso para realizar una operación específica, como leer un archivo o conectarse a una red.

**Sandboxing**: Técnica de aislamiento que ejecuta código en un entorno restringido con acceso limitado a recursos del sistema.

**Deny-by-default (Denegar por defecto)**: Principio de seguridad donde todos los permisos están prohibidos a menos que se concedan explícitamente.

**Tree shaking**: Proceso de eliminación de código muerto durante el empaquetado.

**Code splitting**: Técnica de división del código en fragmentos que se cargan bajo demanda.

**Lockfile**: Archivo que registra las versiones exactas de todas las dependencias para garantizar instalaciones reproducibles.

**Polyfill**: Implementación de una API de JavaScript en entornos que no la soportan nativamente.

**Fuzzing**: Técnica de pruebas que introduce datos aleatorios o semialeatorios para detectar vulnerabilidades.

**Supply chain (Cadena de suministro)**: Conjunto de todos los componentes, dependencias y procesos involucrados en el desarrollo y distribución de software.

**Post-cuantum**: Conjunto de algoritmos criptográficos resistentes a ataques de computación cuántica.

### 3.1.2 Términos Técnicos

**WASM (WebAssembly)**: Formato de instrucciones binarias diseñado para ejecución en navegadores y servidores.

**ESM (ECMAScript Modules)**: Sistema de módulos nativo de JavaScript basado en la especificación ES6+.

**CommonJS**: Sistema de módulos tradicional de Node.js con require() y module.exports.

**JSX**: Extensión de sintaxis para JavaScript que permite escribir HTML en archivos JavaScript.

**QuickJS**: Implementación de JavaScript escrita en C con licencia MIT.

**Tokio**: Runtime asíncrono para Rust.

## 3.2 Abreviaturas

| Abreviatura | Significado |
|-------------|--------------|
| API | Application Programming Interface |
| CLI | Command Line Interface |
| CJS | CommonJS |
| ESM | ECMAScript Modules |
| GDPR | General Data Protection Regulation |
| HIPAA | Health Insurance Portability and Accountability Act |
| ISO | International Organization for Standardization |
| IEC | International Electrotechnical Commission |
| IEEE | Institute of Electrical and Electronics Engineers |
| LTS | Long Term Support |
| MIT | Massachusetts Institute of Technology |
| npm | Node Package Manager |
| PM | Package Manager |
| QubesOS | QUestion Block Environment Operating System |
| RFC | Request for Comments |
| Rust | Rust Programming Language |
| WASM | WebAssembly |
| WASI | WebAssembly System Interface |

## 3.3 Convenciones

### 3.3.1 Convenciones de Nomenclatura

- **Crates**: Prefijo `vvva_` seguido del nombre del componente (e.g., `vvva_core`, `vvva_permissions`)
- **Comandos CLI**: kebab-case (e.g., `3va run`, `3va install`)
- **Flags**: double-dash con kebab-case (e.g., `--allow-net`, `--deny-env`)
- **Enumeraciones**: PascalCase (e.g., `Capability`, `PermissionState`)

### 3.3.2 Convenciones de Versiónado

El versionado sigue SemVer (Semantic Versioning) con el formato:
```
MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]
```

- **MAJOR**: Cambios incompatibles en la API
- **MINOR**: Funcionalidad nueva compatible
- **PATCH**: Correcciones compatibles

## 3.4 Referencias Normativas

### 3.4.1 Estándares ISO/IEC

- ISO/IEC 27001:2022 - Sistemas de gestión de seguridad de la información
- ISO/IEC 25010:2011 - Calidad de productos de software
- ISO/IEC 29148:2018 - Ingeniería de requisitos
- ISO/IEC 12207:2008 - Procesos del ciclo de vida del software

### 3.4.2 Estándares IEEE

- IEEE 829:2008 - Documentación de pruebas de software
- IEEE 1012:2016 - Verificación y validación de software
- IEEE 1063:2002 - Documentación de requisitos de software

### 3.4.3 Estándares Europeos

- EN 55024:2010 - Equipos de tecnología de información - Características de inmunidad
- GDPR (Reglamento (UE) 2016/679) - Protección de datos personales
- eIDAS (Reglamento (UE) 910/2014) - Identificación electrónica y servicios de confianza

### 3.4.4 Estándares de la Industria

- ECMAScript 2024 - Especificación del lenguaje JavaScript
- CommonJS Modules 1.1.1 - Especificación de módulos
- Node.js API Documentation - APIs compatibles

---

*Las definiciones en esta sección son vinculantes para la implementación del proyecto.*

## 4.1 Estándares Internacionales
### 4.1.1 Estándares ISO/IEC
Referencia	Título	Aplicación
ISO/IEC 27001:2022	Sistemas de gestión de seguridad de la información	Requisitos de seguridad
ISO/IEC 27002:2022	Controles de seguridad de la información	Implementación de controles
ISO/IEC 27005:2022	Gestión de riesgos de seguridad de la información	Análisis de riesgos
ISO/IEC 25010:2011	Calidad de productos de software	Métricas de calidad
ISO/IEC 25001:2014	Planificación y gestión de calidad	Gestión de calidad
ISO/IEC 29148:2018	Ingeniería de requisitos	Documentación de requisitos
ISO/IEC 12207:2008	Procesos del ciclo de vida del software	Metodología de desarrollo
ISO/IEC 15289:2019	Documentación de software	Estructura documental
ISO/IEC 15939:2007	Proceso de medición de software	Métricas
### 4.1.2 Estándares IEEE
Referencia	Título	Aplicación
IEEE 829	Estándar para documentación de pruebas de software	Documentación de testing
IEEE 1012	Estándar para verificación y validación de software	V&V
IEEE 1063	Estándar para documentación de requisitos de software	Especificación de requisitos
IEEE 1008	Estándar para pruebas de unidad de software	Testing unitario
IEEE 1028	Estándar para inspecciones de software	Revisiones de código
IEEE 1044	Clasificación de anomalías de software	Gestión de defectos
IEEE 1059	Guía para técnicas de verificación de software	Técnicas de validación
IEEE 1074	Estándar para procesos de desarrollo de software	Metodología
### 4.1.3 Estándares Europeos
Referencia	Título	Aplicación
EN 55024:2010	Equipos de TI - Características de inmunidad	Compatibilidad electromagnética
EN 301 549	Requisitos de accesibilidad para productos ICT	Accesibilidad
GDPR	Reglamento (UE) 2016/679	Protección de datos
eIDAS	Reglamento (UE) 910/2014	Firmas electrónicas
NIS2	Directiva (UE) 2022/2555	Seguridad de redes
## 4.2 Especificaciones Técnicas
### 4.2.1 JavaScript y Node.js
Referencia	Título	Aplicación
ECMAScript 2024	ECMA-262 15th edition	Especificación del lenguaje
CommonJS Modules 1.1.1	CommonJS Module Specification	Sistema de módulos
Node.js API	Node.js Documentation	APIs compatibles
WHATWG Fetch	Fetch Standard	API fetch
WHATWG Streams	Streams Standard	Streams API
W3C WebSocket	WebSocket API	API WebSocket
### 4.2.2 WebAssembly
Referencia	Título	Aplicación
WASI	WebAssembly System Interface	Interface del sistema
WASM Core	WebAssembly Core Specification	Especificación del núcleo
WASM JS API	JavaScript API for WebAssembly	Integración JS
## 4.3 Documentos de Proyecto
Identificador	Título	Descripción
3VA-SPEC-2026-001	Especificación técnica (este documento)	Especificación completa
3VA-DESIGN-2026-001	Documento de diseño arquitectónico	Arquitectura detallada
3VA-TEST-2026-001	Plan de pruebas	Estrategia de testing
3VA-SEC-2026-001	Análisis de seguridad	Modelo de amenazas
3VA-REQ-2026-001	Especificación de requisitos	Requisitos funcionales
## 4.4 Recursos Externos
### 4.4.1 Herramientas y Bibliotecas
Recurso	URL	Propósito
Rust	https://rustup.rs/ (https://rustup.rs/)	Lenguaje de implementación
Tokio	https://tokio.rs/ (https://tokio.rs/)	Runtime asíncrono
QuickJS	https://bellard.org/quickjs/ (https://bellard.org/quickjs/)	Motor JavaScript
rquickjs	https://github.com/Delakroa/rquickjs (https://github.com/Delakroa/rquickjs)	Bindings de QuickJS
Clap	https://clap.rs/ (https://clap.rs/)	CLI parsing
tracing	https://tokio.rs/recipes/tracing (https://tokio.rs/recipes/tracing)	Logging
### 4.4.2 Referencias de Seguridad
Recurso	URL	Propósito
OWASP Top 10	https://owasp.org/www-project-top-ten/ (https://owasp.org/www-project-top-ten/)	Vulnerabilidades web
CVE Database	https://cve.mitre.org/ (https://cve.mitre.org/)	Base de vulnerabilidades
NIST SP 800-53	https://csrc.nist.gov/publications/sp800 (https://csrc.nist.gov/publications/sp800)	Controles de seguridad