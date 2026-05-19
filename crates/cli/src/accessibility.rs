use std::env;

/// Determina si el entorno o el usuario han solicitado desactivar
/// el coloreado ANSI y animaciones complejas, garantizando la compatibilidad
/// con displays Braille y lectores de pantalla (EN 301 549).
pub fn is_accessible_mode(cli_flag: bool) -> bool {
    // Si el usuario pasó explícitamente el flag --accessible
    if cli_flag {
        return true;
    }

    // Verificar el estándar global NO_COLOR
    if let Ok(no_color) = env::var("NO_COLOR") {
        if !no_color.is_empty() {
            return true;
        }
    }

    // Si la consola no es un TTY, deberíamos comportarnos como accesibles por defecto
    // aunque esto depende de cada implementación, pero para este proyecto lo simplificamos
    // a la presencia del flag o variable.

    false
}
