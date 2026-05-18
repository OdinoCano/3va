# 02 - COMANDOS DISPONIBLES

## 2.1 Catálogo de Comandos

Este documento describe exhaustivamente todos los comandos disponibles en la CLI de 3va.

## 2.2 Comandos de Ejecución

### 2.2.1 run
Ejecuta archivos JavaScript o TypeScript.

**Firma:**
```
3va run [OPTIONS] <FILE> [-- <ARGUMENTS>]
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| FILE | string (requerido) | Ruta al archivo a ejecutar |

**Opciones:**
| Opción | Tipo | Default | Descripción |
|--------|------|---------|-------------|
| --inspect | boolean | false | Activa inspector |
| --inspect-brk | boolean | false | Inspector con breakpoint |
| --watch | boolean | false | Watch mode |
| --env | string | - | Variables de entorno |
| --allow-read | boolean | false | Permitir lectura |
| --allow-net | string | - | Permitir red |

**Comportamiento:**
1. Valida que el archivo exista y sea legible
2. Verifica permisos de lectura
3. Crea el contexto de ejecución
4. Ejecuta el código
5. Devuelve el resultado o error

**Ejemplos:**
```bash
3va run app.ts
3va run app.ts -- --arg1 value1
3va run app.ts --inspect-brk --watch
3va run app.ts --allow-read=/app --allow-net=api.example.com
```

### 2.2.2 eval
Evalúa código JavaScript desde la línea de comandos.

**Firma:**
```
3va eval [OPTIONS] <CODE>
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| CODE | string (requerido) | Código a evaluar |

**Opciones:**
| Opción | Tipo | Default | Descripción |
|--------|------|---------|-------------|
| --print | boolean | false | Imprime el resultado |
| --json | boolean | false | Salida JSON |
| --module | boolean | false | Tratar como módulo |

**Ejemplos:**
```bash
3va eval "1 + 1"
3va eval --print "require('fs').readFileSync"
3va eval --json "Promise.resolve(42)"
```

## 2.3 Comandos de Paquetes

### 2.3.1 install
Instala paquetes del registry.

**Firma:**
```
3va install [OPTIONS] <PACKAGE>[@<VERSION>]...
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| PACKAGE | string[] | Paquetes a instalar |

**Opciones:**
| Opción | Tipo | Default | Descripción |
|--------|------|---------|-------------|
| --save | boolean | false | Guardar en dependencies |
| --save-dev | boolean | false | Guardar en devDependencies |
| --save-peer | boolean | false | Guardar en peerDependencies |
| --global | boolean | false | Instalación global |
| --allow-net | string | - | Dominio permitido |

**Ejemplos:**
```bash
3va install lodash
3va install react@18 react-dom@18 --save
3va install -D jest @types/jest
```

### 2.3.2 remove
Desinstala paquetes.

**Firma:**
```
3va remove <PACKAGE>...
```

**Ejemplos:**
```bash
3va remove lodash
3va remove react react-dom --save
```

### 2.3.3 update
Actualiza paquetes a las últimas versiones permitidas.

**Firma:**
```
3va update [OPTIONS] [PACKAGE]...
```

**Opciones:**
| Opción | Descripción |
|--------|-------------|
| --latest | Actualizar a última versión |
| --dry-run | Simular sin ejecutar |

### 2.3.4 list
Lista paquetes instalados.

**Firma:**
```
3va list [OPTIONS]
```

**Opciones:**
| Opción | Descripción |
|--------|-------------|
| --depth=N | Profundidad de dependencias |
| --json | Salida JSON |
| --prod | Solo production |
| --dev | Solo devDependencies |

## 2.4 Comandos de Testing

### 2.4.1 test
Ejecuta la suite de pruebas.

**Firma:**
```
3va test [FILES] [OPTIONS]
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| FILES | string[] | Archivos o patrones |

**Opciones:**
| Opción | Tipo | Default | Descripción |
|--------|------|---------|-------------|
| --watch | boolean | false | Watch mode |
| --coverage | boolean | false | Coverage report |
| --bail | boolean | false | Bail en primer fallo |
| --update-snapshots | boolean | false | Actualizar snapshots |
| --test-name-pattern | string | - | Filtrar por nombre |
| --reporter | string | "spec" | Reporter |

**Entorno de pruebas:**
Los archivos de prueba pueden usar las siguientes extensiones: `.test.js`, `.test.ts`, `.spec.js`, `.spec.ts`, `.test.jsx`, `.test.tsx`

### 2.4.2 test:ui
Abre la interfaz visual de testing.

**Firma:**
```
3va test:ui [OPTIONS]
```

## 2.5 Comandos de Build

### 2.5.1 build
Empaqueta el código para distribución.

**Firma:**
```
3va build <ENTRY> [OPTIONS]
```

**Parámetros:**
| Parámetro | Tipo | Descripción |
|-----------|------|-------------|
| ENTRY | string (requerido) | Punto de entrada |

**Opciones:**
| Opción | Tipo | Default | Descripción |
|--------|------|---------|-------------|
| --out-dir | string | "./dist" | Directorio salida |
| --format | string | "esm" | Formato: esm, cjs, iife |
| --target | string | "node" | Target |
| --minify | boolean | false | Minificar |
| --source-map | boolean | true | Source maps |
| --tree-shaking | boolean | true | Tree shaking |

### 2.5.2 build:watch
Build en modo watch.

**Firma:**
```
3va build:watch <ENTRY> [OPTIONS]
```

## 2.6 Comandos de Información

### 2.6.1 version
Muestra información de versión.

**Firma:**
```
3va version [--json]
```

### 2.6.2 info
Muestra información del proyecto.

**Firma:**
```
3va info [PACKAGE]
```

### 2.6.3 doctor
Diagnostica problemas de instalación.

**Firma:**
```
3va doctor
```

**Verificaciones:**
- Versión de 3va
- Permisos de archivos
- Conectividad de red
- Integridad del cache

### 2.6.4 help
Muestra ayuda.

**Firma:**
```
3va help [COMMAND]
```

---

*Comandos conformes a IEEE 829 y diseño de CLI.*