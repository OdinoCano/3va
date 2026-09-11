// 12 - Expo: validar app.json sin levantar Metro.
// Un script 3va que checa la configuración de tu app Expo antes de compilar.

const fs = require("fs");
const path = require("path");

interface ExpoConfig {
  expo?: {
    name?: string;
    slug?: string;
    version?: string;
    orientation?: "portrait" | "default";
    scheme?: string;
    [key: string]: unknown;
  };
}

const appJsonPath = path.join(__dirname, "..", "app.json");
const errors: string[] = [];

if (!fs.existsSync(appJsonPath)) {
  console.error("app.json no encontrado — ¿olvidaste 3va create expo-app?");
  process.exit(1);
}

const content = fs.readFileSync(appJsonPath, "utf-8");
const config: ExpoConfig = JSON.parse(content);

if (!config.expo) {
  console.error("app.json no contiene la sección 'expo'");
  process.exit(1);
}

const expo = config.expo;

if (!expo.name) errors.push("expo.name es requerido");
if (!expo.slug) errors.push("expo.slug es requerido (usado por Expo Router)");
if (expo.version && !/^\d+\.\d+\.\d+$/.test(expo.version)) {
  errors.push(`expo.version '${expo.version}' no sigue semver`);
}
if (!expo.scheme) {
  console.warn("expo.scheme no está definido — deep links no funcionarán");
}

if (errors.length) {
  errors.forEach((e) => console.error(`  ✗ ${e}`));
  process.exit(1);
}

console.log(`  ✓ app.json válido — ${expo.name} v${expo.version ?? "?"}`);
console.log(`    slug: ${expo.slug}`);
console.log(`    scheme: ${expo.scheme ?? "(no definido)"}`);