// 12 - Expo: demo ejecutable.

import { themeColor, Color, Spacing, FontSize, Radius } from "./theme";
import { truncate, pluralize, capitalize, formatCount } from "./text";
import { formatRelative, formatShortDate } from "./date";

console.log("=== Expo helpers, ejecutados con 3va ===\n");

// --- Theme ---
console.log("Tema claro (primary):", themeColor("primary", false));
console.log("Tema oscuro (primary):", themeColor("primary", true));
console.log("Espaciado SM:", Spacing.sm, "px");
console.log("Radio LG:", Radius.lg, "px");
console.log("H1 font-size:", FontSize.h1, "px");

// --- Text ---
console.log("\nTexto truncado:", truncate("Esta cadena es bastante larga para ser una notificación push", 30));
console.log("Plural: ha enviado", pluralize(1, "1 mensaje", "5 mensajes"));
console.log("Capitalizar:", capitalize("exp-router"));
console.log("Conteo 15200:", formatCount(15200));
console.log("Conteo 2300000:", formatCount(2300000));

// --- Date ---
const now = Date.now();
console.log("\nFormato relativo:");
console.log("  ahora:", formatRelative(now));
console.log("  hace 5min:", formatRelative(now - 5 * 60_000));
console.log("  hace 2h:", formatRelative(now - 2 * 3600_000));
console.log("  en 3d:", formatRelative(now + 3 * 86400_000));
console.log("Fecha corta:", formatShortDate(new Date(2026, 8, 11).getTime()));