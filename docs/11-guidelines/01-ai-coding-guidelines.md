# 01 - DIRECTRICES DE DESARROLLO PARA IA (AI GUIDELINES)

Cualquier sistema de Inteligencia Artificial (LLMs, Code Assistants) o desarrollador humano que genere código para el proyecto **3va** DEBE acatar estrictamente las siguientes reglas, destinadas a cumplir con la directiva europea **EN 301 549** de Accesibilidad.

## 1. Regla de Oro: Accesibilidad Terminal por Defecto

El runtime de `3va` es usado por desarrolladores con impedimentos visuales que dependen de *Displays Braille* y *Lectores de Pantalla* (Screen Readers). La salida del CLI debe respetar el módulo global de accesibilidad `crates/cli/src/accessibility.rs`.

### 1.1 Prohibido el Arte ASCII Excesivo
Si el CLI necesita imprimir cuadros, tablas o elementos decorativos, la IA DEBE envolver dicha lógica en una comprobación del flag `--accessible`.
Los caracteres como `┌`, `─`, `│`, `└` abruman el hardware Braille, que los leerá literalmente.
- **Incorrecto:** Siempre imprimir tablas dibujadas con barras.
- **Correcto:** Revisar `accessibility::is_accessible_mode()` y si es true, imprimir una lista plana.

### 1.2 Cuidado con las Animaciones (Spinners/Barras de progreso)
Al crear comandos que toman tiempo (ej. `3va install`, `3va bundle`), está prohibido enviar animaciones que dependan de retornos de carro continuos (`\r`) si el modo accesible está activo.
- Un retorno de carro continuo congela la línea Braille.
- En modo accesible, imprime mensajes estáticos: `INFO: Descargando...` y `INFO: Descarga completada`.

### 1.3 Los Colores NO son Semántica Exclusiva
El sistema de logs (`tracing`) ya está configurado para desactivar los códigos ANSI (`NO_COLOR`) en modo accesible. Por ende, la IA jamás debe usar "colores" como la única manera de denotar un estado (ej. pintar texto de rojo asumiendo que el usuario entenderá que es un error).
- **Obligatorio:** Siempre anteponer prefijos de texto claro como `ERROR:`, `WARN:`, `SUCCESS:`.

## 2. Ejemplo de Implementación

Al añadir lógica a `main.rs` o comandos del CLI:

```rust
// CORRECTO
let is_accessible = accessibility::is_accessible_mode(cli.accessible);

if is_accessible {
    println!("INFO: Compilando proyecto...");
} else {
    // Aquí puedes usar spinners, crate indicatif, o tablas ASCII
    start_spinner("Compilando...");
}
```

> **Atención IA**: Si el usuario te pide "haz la consola más bonita" o "añade una barra de progreso", tienes **obligatoriamente** que respetar estas directrices e incluir la rama del `if` para el modo accesible.
