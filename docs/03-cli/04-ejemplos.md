# 04 - EJEMPLOS DE USO

## 4.1 Ejemplos de Ejecución

### 4.1.1 Ejecución Básica

```bash
# Ejecutar archivo JavaScript
3va run hello.js

# Ejecutar archivo TypeScript
3va run app.ts

# Ejecutar con argumentos
3va run script.ts -- --nombre valor
```

### 4.1.2 Con Permisos Granulares

```bash
# Leer solo un directorio específico
3va run app.ts --allow-read=/app/data

# Conectar a una API específica
3va run app.ts --allow-net=api.example.com

# Desarrollo completo local
3va run dev.ts --allow-read --allow-write --allow-net
```

### 4.1.3 Modo Inspector

```bash
# Con inspector para Chrome DevTools
3va run app.ts --inspect

# Con breakpoint inicial
3va run app.ts --inspect-brk
# Después en Chrome: chrome://inspect
```

## 4.2 Ejemplos de Package Manager

### 4.2.1 Instalación Básica

```bash
# Instalar un paquete
3va install lodash

# Con versión específica
3va install react@18.2.0

# Instalar y guardar en dependencies
3va install axios --save

# Instalar como dependencia de desarrollo
3va install jest --save-dev
```

### 4.2.2 Instalación con Permisos

```bash
# Instalar con acceso a red para registry
3va install axios --allow-net=registry.npmjs.org

# Instalar desde registry específico
3va install axios --registry=https://registry.npmmirror.com
```

### 4.2.3 Gestión de Paquetes

```bash
# Listar paquetes instalados
3va list

# Listar con profundidad
3va list --depth=2

# Actualizar paquetes
3va update

# Desinstalar
3va remove lodash
```

## 4.3 Ejemplos de Testing

### 4.3.1 Ejecución de Tests

```bash
# Ejecutar todos los tests
3va test

# Ejecutar archivos específicos
3va test tests/

# Modo watch
3va test --watch

# Con coverage
3va test --coverage

# Bail en primer fallo
3va test --bail

# Actualizar snapshots
3va test --update-snapshots

# Filtrar por nombre
3va test --test-name-pattern="auth"
```

### 4.3.2 Configuración de Tests

Archivo `jest.config.js` o en `package.json`:
```javascript
module.exports = {
  testEnvironment: "node",
  testMatch: ["**/*.test.js", "**/*.test.ts"],
  collectCoverage: true,
  coverageDirectory: "coverage",
};
```

## 4.4 Ejemplos de Build

### 4.4.1 Build Básico

```bash
# Build básico
3va build index.ts

# Output a directorio específico
3va build index.ts --out-dir ./dist

# Minificar
3va build index.ts --minify
```

### 4.4.2 Build Avanzado

```bash
# ES Modules para Node.js
3va build index.ts --format=esm --target=node

# IIFE para navegador
3va build index.ts --format=iife --target=browser --minify

# Con source maps
3va build index.ts --source-map

# Watch mode
3va build:watch index.ts
```

## 4.5 Ejemplos de Seguridad

### 4.5.1 Configuración Restrictiva

```bash
# Entorno muy restrictivo
3va run app.ts --allow-read

# Sin acceso a red
3va run app.ts --deny-net

# Solo lectura de archivos específicos
3va run app.ts --allow-read=/app/config.json
```

### 4.5.2 Auditoría

```bash
# Ver logs de auditoría
3va run app.ts -V 2>&1 | grep AUDIT

# Logs en archivo
3va run app.ts --log-file=/var/log/3va/audit.log
```

## 4.6 Scripts de package.json

```json
{
  "scripts": {
    "start": "3va run src/index.ts",
    "dev": "3va run src/index.ts --watch",
    "build": "3va build src/index.ts --out-dir dist --minify",
    "test": "3va test",
    "test:watch": "3va test --watch",
    "test:coverage": "3va test --coverage",
    "install": "3va install"
  }
}
```

---

*Ejemplos conformes a IEEE 829 y casos de uso.*