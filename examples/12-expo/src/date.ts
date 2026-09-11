// 12 - Expo: formato de fecha relativa (hace X, en X, hoy).
// Componentes Expo-Router muestran timestamps así en la UI.

export function formatRelative(fromMs: number, toMs: number = Date.now()): string {
  const diff = toMs - fromMs;
  const sec = Math.round(diff / 1000);
  if (sec < 0) {
    const absSec = Math.abs(sec);
    if (absSec < 60) return `en ${absSec}s`;
    const min = Math.floor(absSec / 60);
    if (min < 60) return `en ${min}min`;
    const h = Math.floor(min / 60);
    if (h < 24) return `en ${h}h`;
    const d = Math.floor(h / 24);
    return `en ${d}d`;
  }
  if (sec < 60) return "ahora";
  const min = Math.floor(sec / 60);
  if (min < 60) return `hace ${min}min`;
  const h = Math.floor(min / 60);
  if (h < 24) return `hace ${h}h`;
  const d = Math.floor(h / 24);
  return `hace ${d}d`;
}

export function formatShortDate(dateMs: number): string {
  const d = new Date(dateMs);
  const day = String(d.getDate()).padStart(2, "0");
  const month = String(d.getMonth() + 1).padStart(2, "0");
  const year = d.getFullYear();
  return `${day}/${month}/${year}`;
}