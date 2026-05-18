4.1 Flujo Principal: Ejecución de Script
4.1.1 Diagrama de Flujo
┌──────────────┐
│    Usuario   │
│ invoca CLI   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Parseo    │ ──► Validación de argumentos
│  argumentos │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Build     │ ──► Construcción de PermissionState
│ Permissions │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Inicializar│ ──► Creación de Runtime + JsEngine
│   Runtime   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ Cargar Módulo│ ──► Verificar FileRead permission
│   (require) │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Verificar  │ ──► Matching contra PermissionState
│  Permiso    │
└──────┬───────┘
       │
   ┌───┴───┐
   │ALLOW  │DENY
   ▼       ▼
┌─────┐ ┌─────┐
│Ejecutar│ │Error│
│Código │ │403  │
└─────┘ └─────┘
   │
   ▼
┌──────────────┐
│   Output    │
│  al stdout  │
└─────────────┘
4.1.2 Descripción de Pasos
1. 
Parseo de argumentos: CLI valida y normaliza los argumentos
2. 
Build Permissions: Construye el estado de permisos desde flags
3. 
Inicializar Runtime: Crea el event loop y motor JS
4. 
Cargar Módulo: Lee el archivo y verifica permisos de lectura
5. 
Verificar Permiso: Compara la operación contra capabilities granted
6. 
Ejecutar Código: Ejecuta en QuickJS con polyfills
7. 
Output: Escribe resultado a stdout/stderr
4.2 Flujo: Instalación de Paquete
4.2.1 Diagrama de Flujo
┌──────────────┐
│  3va install │
│   <package>  │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Verificar   │ ──► Permiso --allow-net para registry
│  Permiso Net │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Consultar  │ ──► API del registry (npm/yarn)
│   Registry   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Resolver    │ ──► Árbol de dependencias
│ Dependencias │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Verificar  │ ──► Checksum, firma digital
│  Firma      │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Escanear    │ ──► Análisis de malware/vulnerabilidades
│  Seguridad   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Descargar  │ ──► Cache local + verificación hash
│   Paquetes  │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Instalar    │ ──► Copia a node_modules (SIN post-install)
│  Sandbox    │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Generar     │ ──► lockfile con versiones exactas
│  Lockfile   │
└─────────────┘
4.3 Flujo: Verificación de Permisos
4.3.1 Algoritmo de Verificación
fn verify_permission(
    operation: &Operation,
    resources: &[Resource],
    permissions: &PermissionState
) -> Result<Decision> {
    
    // 1. Verificar si es una operación denegada explícitamente
    if permissions.is_denied(operation) {
        return Ok(Denied::Explicit);
    }
    
    // 2. Buscar capability que permita la operación
    for cap in &permissions.granted {
        if matches_capability(operation, resources, cap) {
            return Ok(Allowed(cap.clone()));
        }
    }
    
    // 3. Si no hay match, denegar (deny-by-default)
    Ok(Denied::NoCapability)
}
4.3.2 Matriz de Decisión
Operación	Recurso	Capability Match
fs.read	/app/config	FileRead(/app/config)
fs.read	/etc/passwd	FileRead(/app/config)
net.connect	api.example.com	Network(*.example.com)
net.connect	evil.com	Network(*.example.com)
spawn	ls	SpawnProcess
spawn	(none)	(none)
4.4 Flujo de Datos: Módulos
4.4.1 Resolución de Módulos ESM
Archivo.ts
    │
    ▼
┌─────────────────┐
│   Parsear       │ ──► Detectar imports/exports
│   imports       │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Resolver       │ ──► node_modules, path resolution
│  路径           │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Verificar      │ ──► FileRead permission
│   Permiso       │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Cargar        │ ──► Leer archivo, transpilar TS►JS
│   Módulo        │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Ejecutar      │ ──► Añadir al module graph
│   en contexto  │
└─────────────────┘
4.4.2 Formato de Module Graph
pub struct ModuleGraph {
    pub modules: HashMap<ModuleId, Module>,
    pub edges: HashMap<ModuleId, Vec<ModuleId>>,
}
Flujos de datos conforme a IEEE 1012 y modelado de procesos.