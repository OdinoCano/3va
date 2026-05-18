# 04 - AUDITORÍA Y LOGGING

## 4.1 Sistema de Auditoría

3va implementa un sistema completo de auditoría que registra todas las operaciones sensibles para cumplimiento regulatorio y análisis forense.

## 4.2 Eventos Auditados

### 4.2.1 Categorías de Eventos

| Categoría | Descripción | Eventos |
|-----------|-------------|---------|
| permission | Verificaciones de permisos | check, allow, deny |
| fs | Operaciones de sistema de archivos | read, write, delete, mkdir |
| network | Operaciones de red | connect, send, receive |
| process | Creación de procesos | spawn, exec |
| env | Acceso a variables de entorno | get, set |
| module | Carga de módulos | load, resolve |
| security | Eventos de seguridad | blocked, flagged |

### 4.2.2 Formato de Evento

```rust
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,           // RFC 3339
    pub event_id: Uuid,                      // Unique identifier
    pub category: AuditCategory,             // permission, fs, network, etc.
    pub action: String,                      // read, write, connect, etc.
    pub resource: Option<String>,            // Path, URL, etc.
    pub principal: Principal,                // User/session info
    pub decision: AuditDecision,             // allow, deny
    pub reason: Option<String>,              // Por qué se permitió/denegó
    pub metadata: HashMap<String, String>,   // Additional data
    pub source: EventSource,                 // CLI, API, module
}

pub enum AuditDecision {
    Allow,
    Deny(String),  // Razón de denegación
}

pub enum EventSource {
    UserCode,
    Builtin,
    Package(String),
    CLI,
}
```

### 4.2.3 Ejemplo de Evento

```json
{
  "timestamp": "2026-05-18T14:30:00.123Z",
  "eventId": "550e8400-e29b-41d4-a716-446655440000",
  "category": "fs",
  "action": "read",
  "resource": "/app/config.json",
  "principal": {
    "user": "root",
    "session": "abc123"
  },
  "decision": "allow",
  "source": "userCode",
  "metadata": {
    "mode": "sync",
    "size": "1024"
  }
}
```

## 4.3 Configuración de Auditoría

### 4.3.1 Niveles de Logging

| Nivel | Eventos Registrados |
|-------|---------------------|
| off | Ninguno |
| errors | Solo denegaciones y errores |
| warnings | errors + advertencias |
| info | warnings + operaciones principales |
| debug | info + todos los detalles |
| trace | debug + información de debugging |

### 4.3.2 Configuración

```rust
pub struct AuditConfig {
    pub level: AuditLevel,
    pub destinations: Vec<AuditDestination>,
    pub retention_days: u32,
    pub max_file_size: u64,
    pub rotate: bool,
    pub filters: AuditFilters,
}

pub enum AuditDestination {
    File(PathBuf),
    Stdout,
    Stderr,
    Syslog,
    Custom(Box<dyn AuditSink>),
}

pub struct AuditFilters {
    pub categories: Vec<AuditCategory>,
    pub min_decision: AuditDecision,  // Solo permitir decisões >= nivel
    pub resources: Option<Vec<String>>,  // Filtrar por recursos específicos
}
```

### 4.3.2 CLI Configuration

```bash
# Registrar a archivo
3va run app.ts --audit-log=/var/log/3va/audit.log

# Registro a stdout
3va run app.ts --audit-log=stdout

# Nivel de detalle
3va run app.ts --audit-level=info --audit-log=/var/log/3va/audit.log

# Filtrar solo denegaciones
3va run app.ts --audit-level=errors
```

## 4.4 Implementación

### 4.4.1 Audit Logger

```rust
pub struct AuditLogger {
    config: AuditConfig,
    writer: Box<dyn Write>,
    formatter: AuditFormatter,
}

impl AuditLogger {
    pub fn log(&self, event: AuditEvent) {
        // 1. Filtrar según configuración
        if !self.should_log(&event) {
            return;
        }

        // 2. Formatear
        let formatted = self.formatter.format(&event);

        // 3. Escribir
        if let Err(e) = self.writer.write(formatted) {
            eprintln!("Audit log write failed: {}", e);
        }
    }

    fn should_log(&self, event: &AuditEvent) -> bool {
        // Verificar nivel
        if !event.category.enabled_at(self.config.level) {
            return false;
        }

        // Verificar filtros
        if let Some(resources) = &self.config.filters.resources {
            if let Some(resource) = &event.resource {
                return resources.iter().any(|r| resource.contains(r));
            }
        }

        true
    }
}
```

### 4.4.2 Integración con Permissions

```rust
// En PermissionState
pub fn check_with_audit(&self, cap: &Capability) -> bool {
    let decision = if self.check(cap) {
        AuditDecision::Allow
    } else {
        AuditDecision::Deny("No matching capability".to_string())
    };

    audit::log(AuditEvent {
        category: AuditCategory::Permission,
        action: "check".to_string(),
        resource: Some(format!("{:?}", cap)),
        decision,
        ..Default::default()
    });

    decision == AuditDecision::Allow
}
```

## 4.5 Rotación de Logs

### 4.5.1 Configuración

```rust
pub struct LogRotation {
    pub max_size: u64,        // Tamaño máximo por archivo
    pub max_files: u32,       // Número máximo de archivos
    pub compress: bool,       // Comprimir archivos viejos
}

impl LogRotation {
    pub fn should_rotate(&self, current_size: u64) -> bool {
        current_size >= self.max_size
    }

    pub fn rotate(&self, path: &Path) -> std::io::Result<Vec<PathBuf>> {
        // 1. Renombrar archivo actual a .1
        // 2. Comprimir archivos antigos si está habilitado
        // 3. Eliminar archivos > max_files
    }
}
```

## 4.6 Cumplimiento Regulatorio

### 4.6.1 GDPR

```rust
// Para cumplimiento GDPR:
// - Registro de accesos a datos personales
// - Retención configurable
// - Derecho a eliminación

pub struct GdprConfig {
    pub log_personal_data_access: bool,
    pub personal_data_patterns: Vec<Regex>,
    pub retention_days: u32,
    pub right_to_deletion: bool,
}
```

### 4.6.2 ISO 27001

```rust
// Cumplimiento ISO 27001:
// - Auditoría de seguridad
// - Trazabilidad
// - No repudio

pub struct Iso27001Config {
    pub log_all_security_events: bool,
    pub log_access_control: bool,
    pub immutability: bool,  // Logs no pueden ser modificados
    pub integrity_check: bool,  // Checksum de logs
}
```

## 4.7 Herramientas de Análisis

### 4.7.1 CLI de Auditoría

```bash
# Ver logs de auditoría
3va audit view --file /var/log/3va/audit.log

# Filtrar por categoría
3va audit view --category=denied

# Filtrar por tiempo
3va audit view --since="2026-05-18T10:00:00Z"

# Generar reporte
3va audit report --output=audit-report.html

#统计
3va audit stats --period=24h
```

### 4.7.2 Log Aggregation

```rust
// Agregación de múltiples fuentes
pub struct AuditAggregator {
    sources: Vec<Box<dyn AuditSource>>,
}

impl AuditAggregator {
    pub fn query(&self, query: AuditQuery) -> Vec<AuditEvent> {
        // Agregar eventos de múltiples fuentes
        // y retornar resultados unificados
    }
}
```

---

*Auditoría conforme a ISO 27001, GDPR, y estándares de seguridad.*