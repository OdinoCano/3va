# 01 - ARQUITECTURA GENERAL DEL SISTEMA

## 1.1 Visión de Arquitectura

La arquitectura de 3va sigue un diseño modular basado en crates de Rust, donde cada componente cumple una responsabilidad específica y se comunica a través de interfaces bien definidas. El sistema está diseñado siguiendo principios de seguridad por defecto y separación de privilegios.

```
┌─────────────────────────────────────────────────────────────────┐
│                         3va CLI                                 │
│                    (vvva_cli - Entrypoint)                       │
└─────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
        ▼                     ▼                     ▼
┌───────────────┐    ┌───────────────┐    ┌───────────────┐
│   vvva_core   │    │   vvva_pm     │    │  vvva_bundler │
│   (Runtime)   │    │  (Package     │    │   (Bundler)   │
│               │    │   Manager)    │    │               │
└───────────────┘    └───────────────┘    └───────────────┘
        │                     │                     │
        └─────────────────────┼─────────────────────┘
                              │
                              ▼
                    ┌───────────────┐
                    │ vvva_permissions
                    │ (Permissions) │
                    └───────────────┘
                              │
                              ▼
                    ┌───────────────┐
                    │   vvva_js     │
                    │ (JS Engine)   │
                    └───────────────┘
```

## 1.2 Principios de Diseño

### 1.2.1 Seguridad por Defecto
Todos los componentes operan bajo el principio de "denegar por defecto". Ningún proceso tiene acceso a recursos del sistema sin una Capability explícita otorgada por el usuario.

### 1.2.2 Mínimo Privilegio
Cada componente tiene exactamente los permisos necesarios para cumplir su función. El runtime de JavaScript no tiene acceso directo al sistema de archivos; cualquier operación debe pasar por el verificador de permisos.

### 1.2.3 defensa en Profundidad
Múltiples capas de seguridad protegen el sistema:
- Capa 1: CLI (validación de argumentos)
- Capa 2: Permissions (verificación de capabilities)
- Capa 3: Sandbox (aislamiento del proceso)
- Capa 4: Audit (registro de operaciones)

### 1.2.4 Modularidad
Cada crate es independiente y puede ser reemplazado o actualizado sin afectar otros componentes. La integración con QuickJS, por ejemplo, está abstraída para permitir cambios futuros.

## 1.3 Arquitectura de Componentes

### 1.3.1 Capa de Presentación (CLI)
El CLI actúa como punto de entrada único para todas las operaciones. Parsea los argumentos del usuario, construye el contexto de permisos y delega al componente apropiado.

**Responsabilidades:**
- Parsing de argumentos y validación
- Construcción de PermissionState
- Enrutamiento de comandos
- Formateo de salida

**Interfaces:**
- Interfaz de línea de comandos (usuario)
- Interfaz de eventos (core, pm, bundler)

### 1.3.2 Capa de Ejecución (Core)
El core gestiona el ciclo de vida del runtime, incluyendo el event loop, la gestión de procesos y la coordinación de tareas asíncronas.

**Responsabilidades:**
- Event loop principal
- Scheduling de tareas
- Gestión de memoria
- Coordinación de componentes

**Interfaces:**
- API de tareas asíncronas
- API de gestión de procesos

### 1.3.3 Capa de Seguridad (Permissions)
El sistema de permisos aplica el modelo de capabilities, verificando cada operación contra la lista de permisos granted.

**Responsabilidades:**
- Almacenamiento de capabilities
- Verificación de permisos
- Matching de patrones
- Auditoría de operaciones

**Interfaces:**
- API de verificación de permisos
- API de auditoría

### 1.3.4 Capa de Ejecución JavaScript (JS)
El motor JavaScript ejecuta el código del usuario en un entorno aislado con acceso a las APIs web estándar.

**Responsabilidades:**
- Ejecución de código JS/TS
- Gestión de módulos
- Implementación de polyfills
- Aislamiento del contexto

**Interfaces:**
- API de evaluación de código
- API de módulos
- API de globals

### 1.3.5 Capa de Paquetes (PM)
El gestor de paquetes maneja la descarga, verificación e instalación de dependencias.

**Responsabilidades:**
- Resolución de dependencias
- Descarga de paquetes
- Verificación de firmas
- Sandboxing de instalación

**Interfaces:**
- API de instalación
- API de resolución
- API de lockfile

## 1.4 Modelo de Despliegue

3va se desplaza como un binario único que contiene todos los componentes integrados. Esto facilita la distribución y reduce la superficie de ataque.

```
┌─────────────────────────────────────────┐
│              Binario 3va                │
├─────────────────────────────────────────┤
│ CLI    │ Core │ Perms │ JS │ PM │ Bundler│
└─────────────────────────────────────────┘
```

---

*Documento conforme a ISO/IEC/IEEE 42010 y arquitectura de sistemas.*