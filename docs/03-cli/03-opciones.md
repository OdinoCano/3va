# 03 - OPCIONES Y FLAGS
3.1 Sistema de Opciones
Este documento detalla todas las opciones y flags disponibles en 3va.
3.2 Opciones por Categoría
3.2.1 Opciones de Permiso
--allow-read
Permite operaciones de lectura del sistema de archivos.
Tipos de uso:
# Permitir lectura global
--allow-read
# Permitir lectura de path específico
--allow-read=/path/to/dir
--allow-read=/path/to/file.js
--allow-read=/path/to/dir/*
Matching de patrones:
- 
/path - Permite todo en ese directorio
- 
/path/* - Equivalente al anterior
- 
/path/**/*.js - Todos los archivos .js recursivamente
--allow-write
Permite operaciones de escritura del sistema de archivos.
Tipos de uso:
# Permitir escritura global
--allow-write
# Permitir escritura en path específico
--allow-write=/tmp
--allow-write=/app/cache
--allow-net
Permite conexiones de red.
Tipos de uso:
# Permitir toda red
--allow-net
# Permitir hosts específicos
--allow-net=api.example.com
--allow-net=*.example.com
--allow-net=192.168.1.0/24
# Múltiples hosts
--allow-net=api.example.com --allow-net=cdn.example.com
Patrones soportados:
- 
host - Host exacto
- 
*.host - Subdominios
- 
host:port - Host y puerto específico
- 
*.host:8080 - Subdominios con puerto
--allow-env
Permite acceso a variables de entorno.
Uso:
--allow-env
Acceso limitado:
# Solo variables específicas (futuro)
--allow-env=PATH,HOME
--allow-child-process
Permite crear procesos hijos.
Uso:
--allow-child-process
Con restricciones (futuro):
--allow-child-process=git,curl
3.2.2 Opciones de Denegación
Los flags de denegación se usan para quitar permisos específicos cuando se usa un preset que concede más de lo necesario.
# Desarrollo local pero sin red
3va run dev.ts --allow-read --allow-write --deny-net
# Entorno de prueba sin procesos hijos
3va run test.ts --allow-read --deny-child-process
3.2.3 Opciones de Runtime
--inspect
Activa el inspector de depuración de Chrome.
3va run app.ts --inspect
# Listening on ws://127.0.0.1:9229/...
--inspect-brk
Inspector con breakpoint inicial.
3va run app.ts --inspect-brk
--watch
Recarga automática ante cambios.
3va run app.ts --watch
3.2.4 Opciones de Package Manager
--save, --save-dev, --save-peer
Ubicación de la dependencia.
3va install lodash --save           # dependencies
3va install jest --save-dev       # devDependencies
3va install react --save-peer     # peerDependencies
--global
Instalación global del paquete.
3va install typescript --global
3.2.5 Opciones de Build
--out-dir
Directorio de salida.
3va build index.ts --out-dir ./dist
--format
Formato del bundle.
3va build index.ts --format=esm    # ES Modules
3va build index.ts --format=cjs   # CommonJS
3va build index.ts --format=iife  # IIFE
--target
Target de compilación.
3va build index.ts --target=node
3va build index.ts --target=browser
3va build index.ts --target=webworker
--minify
Minificar el output.
3va build index.ts --minify
--source-map
Generar source maps.
3va build index.ts --source-map
3va build index.ts --source-map=hidden
3.2.6 Opciones de Testing
--coverage
Generar reporte de cobertura.
3va test --coverage
--update-snapshots
Actualizar snapshots automáticamente.
3va test --update-snapshots
--reporter
Seleccionar reporter.
3va test --reporter=spec
3va test --reporter=dot
3va test --reporter=json
3.3 Presets de Permisos
3.3.1 preset:node
Simula el comportamiento de Node.js.
3va run app.ts --preset=node
# Equivalente a:
--allow-read --allow-write --allow-net --allow-env --allow-child-process
3.3.2 preset:browser
Simula el entorno de navegador.
3va run app.ts --preset=browser
# Equivalente a:
--allow-net --allow-read --allow-write
3.3.3 preset:none
Sin permisos (más restrictivo que default).
3va run app.ts --preset=none
# Equivalente a:
# (ningún permiso granted por defecto)
3.4 Variables de Entorno del CLI
Variable	Descripción
3VA_CONFIG	Ruta al archivo de configuración
3VA_LOG_LEVEL	Nivel de logging
3VA_CACHE_DIR	Directorio de cache
3VA_REGISTRY	Registry de npm a usar
Opciones conforme a IEEE 829 y diseño de CLI.