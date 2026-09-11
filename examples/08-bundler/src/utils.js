// 08 - Bundler: shared utility
// Imported by index.js — gets inlined into the final bundle.

export function formatTime(date) {
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  return `${hours}:${minutes}:${seconds}`;
}

export function pluralize(count, singular, plural) {
  return count === 1 ? singular : plural;
}