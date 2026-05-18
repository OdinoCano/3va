# 01 - ARQUITECTURA DEL BUNDLER

## 1.1 Visión General

El empaquetador (Bundler) nativo de 3va es un sistema optimizado de generación de código capaz de resolver grafos de dependencias, ejecutar *tree-shaking* (eliminación de código muerto) de manera agresiva y generar *chunks* (fragmentos) sin dependencias externas.

## 1.2 Componentes Principales

```
┌─────────────────────────────────────────────────────────────┐
│                       3va Bundler                           │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │  Resolver   │  │ Tree Shaker │  │  Generator  │        │
│  │ (Mod Graph) │  │(Dead Code)  │  │(Code Emit)  │        │
│  └─────────────┘  └─────────────┘  └─────────────┘        │
└─────────────────────────────────────────────────────────────┘
```

### 1.2.1 El Resolutor (ModuleResolver)
Construye el grafo del módulo (`DependencyGraph`) analizando los imports ESM (`import x from 'y'`) y las llamadas CommonJS (`require()`), resolviendo rutas relativas y absolutas contra el sistema de archivos virtual (`VirtualFs`).

### 1.2.2 El Tree Shaker
Analiza el AST (Abstract Syntax Tree) para marcar los nodos que se utilizan realmente (`used_exports`) y purgar las declaraciones exportadas que jamás son llamadas por ningún archivo consumidor del proyecto.

### 1.2.3 El Generador (CodeGenerator)
Emite el código de salida unificando los módulos bajo un formato destino (IIFE, CommonJS, ESM). Soporta remoción de tipado TypeScript instantánea en el proceso de lectura (`process_module`).

## 1.3 Formatos de Salida Soportados

| Formato | Descripción | Uso principal |
|---------|-------------|---------------|
| `IIFE` | Expresión de función invocada inmediatamente | Browsers (sin systemjs) |
| `CJS` | CommonJS (`require` / `module.exports`) | Node.js Legacy |
| `ESM` | ES Modules (`import` / `export`) | Navegadores Modernos y 3va |

## 1.4 Ejemplo de Uso API

```rust
let mut bundler = Bundler::new(PathBuf::from("."));
let options = BundlerOptions {
    minify: true,
    format: OutputFormat::Esm,
    sourcemap: true,
};

bundler.with_options(options);
bundler.add_entry("src/index.ts")?;
let code = bundler.bundle()?;
```

## 1.5 Cumplimiento Normativo (ISO/IEC)

El diseño modular de este empaquetador está trazado bajo el marco procedimental de **ISO/IEC 12207** (Procesos del ciclo de vida del software), proveyendo una etapa de "construcción" inmutable y auditable.
- Al aislar el parsing, minificación y resolución, asegura la trazabilidad del código fuente hasta el artefacto compilado.

---

*Implementado en `crates/bundler/src/` (`lib.rs`, `tree_shaker.rs`, `generator.rs`, `resolver.rs`).*
