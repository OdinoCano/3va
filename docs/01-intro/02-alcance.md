# 02 - ALCANCE Y OBJETIVOS DEL PROYECTO

## 2.1 Alcance Funcional

### 2.1.1 Componentes Incluidos

El alcance de 3va comprende los siguientes componentes principales:

#### 2.1.1.1 Runtime (vvva_core)
- Event loop asíncrono basado en Tokio
- Integración con motor JavaScript (QuickJS vía rquickjs)
- Soporte para ESM y CommonJS
- Implementación de APIs web estándar (fetch, WebSocket, ReadableStream)
- Gestión de módulos y require/import

#### 2.1.1.2 CLI (vvva_cli)
- Interfaz de línea de comandos con clap
- Subcomandos: run, install, test, build, eval
- Flags de permisos: --allow-read, --allow-write, --allow-net, --allow-env, --allow-child-process
- Flags de denegación: --deny-* (para revocar permisos específicos)

#### 2.1.1.3 Sistema de Permisos (vvva_permissions)
- Modelo de capacidades basado en deny-by-default
- Matching de patrones glob para hostnames y paths
- Enforcement de políticas en tiempo de ejecución
- Auditoría de sensibilidad operativa

#### 2.1.1.4 Package Manager (vvva_pm)
- Resolución de dependencias compatible con npm
- Verificación de firmas criptográficas
- Ejecución de post-install scripts deshabilitada por defecto
- Sandboxing de paquetes instalados
- Formato de lockfile propietario

#### 2.1.1.5 Motor JavaScript (vvva_js)
- Integración con QuickJS
- Polyfills para APIs de Node.js
- Soporte para TypeScript y JSX (transpilación en tiempo de ejecución)
- Carga de módulos ESM y CommonJS

#### 2.1.1.6 Bundler (vvva_bundler) [POR IMPLEMENTAR]
- Transpilación de TypeScript y JSX
- Tree shaking basado en análisis estático
- Code splitting automático
- Generación de source maps

#### 2.1.1.7 Test Runner (vvva_test) [POR IMPLEMENTAR]
- Compatibilidad con Jest
- Matchers estándar y personalizados
- Soporte para snapshots
- Modo watch
- Integración con análisis de seguridad

### 2.1.2 Componentes Excluidos

Los siguientes componentes están fuera del alcance inicial:
- IDE plugins y extensiones
- Depurador gráfico (solo CLI debugger)
- Package registry hosting
- Servicios de CI/CD

## 2.2 Objetivos del Proyecto

### 2.2.1 Objetivos de Seguridad

| Objetivo | Métrica | Target |
|----------|---------|--------|
| Cobertura de análisis estático | Porcentaje de vulnerabilidades detectadas | ≥95% |
| Tiempo de escaneo de paquetes | Por paquete | <500ms |
| Falsos positivos | Tasa | <2% |
| Cumplimiento normativo | Estándares aplicados | ISO 27001, GDPR |
| Auditoría | Eventos registrados | 100% |

### 2.2.2 Objetivos de Rendimiento

| Objetivo | Métrica | Target |
|----------|---------|--------|
| Tiempo de inicio | Cold start | <100ms |
| Ejecución de scripts | Benchmarks | Comparable a Bun |
| Instalación de paquetes | Paquetes/segundo | ≥30x npm |
| Uso de memoria | MB por proceso | <50MB base |

### 2.2.3 Objetivos de Compatibilidad

| Objetivo | Métrica | Target |
|----------|---------|--------|
| Compatibilidad Node.js | APIs implementadas | 99.9% |
| Compatibilidad npm | Paquetes funcionando | 95% |
| Compatibilidad ESM/CJS | Módulos cargados | 100% |

## 2.3 Fases de Desarrollo

### Fase 1: Foundation (Mes 1-3)
- Core runtime funcional
- CLI básico con permisos
- Integración QuickJS operativa

### Fase 2: Package Manager (Mes 4-6)
- Instalación de paquetes
- Lockfile
- Sandbox básico

### Fase 3: Herramientas (Mes 7-9)
- Bundler
- Test Runner
- Análisis de seguridad

### Fase 4: LTS (Mes 10-12)
- Estabilización
- Compatibilidad completa
- Documentación final

## 2.4 Entregables

1. Runtime binario ejecutable
2. Documentación técnica (este documento)
3. Documentación de API
4. Suite de pruebas de regresión
5. Informe de seguridad
6. Guía de contribución

---

*Documento conforme a IEEE 829 y estándares de gestión de proyectos.*